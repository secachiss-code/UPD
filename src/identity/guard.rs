//! Отпечаток профиля и обнаружение запуска мимо CM.

use super::engine::Family;
use super::model::BrowserIdentityProfile;
use super::store::{self, IdentityStore, StoreError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMark {
    pub path: String,
    pub present: bool,
    pub size: u64,
    #[serde(with = "i128_string")]
    pub mtime_ns: i128,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchMark {
    pub generation: u32,
    pub pid: u32,
    pub started_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitMark {
    pub at: i64,
    pub files: Vec<FileMark>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardState {
    pub last_launch: Option<LaunchMark>,
    pub last_exit: Option<ExitMark>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardOutcome {
    FirstLaunch,
    Clean,
    Warned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardError {
    BypassSuspected,
    BrowserRunning,
}

impl GuardError {
    pub fn code(self) -> &'static str {
        match self {
            Self::BypassSuspected => "BypassSuspected",
            Self::BrowserRunning => "BrowserRunning",
        }
    }
}

impl fmt::Display for GuardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BypassSuspected => t!("профиль открывали мимо CM: нужно cm identity confirm"),
            Self::BrowserRunning => t!("этот профиль уже открыт в браузере"),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BypassPolicy {
    Block,
    Warn,
}

pub fn bypass_policy() -> BypassPolicy {
    let text = fs::read_to_string(crate::common::conf_path()).unwrap_or_default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "identity_bypass" {
            continue;
        }
        return if value.trim() == "warn" {
            BypassPolicy::Warn
        } else {
            BypassPolicy::Block
        };
    }
    BypassPolicy::Block
}

pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: kill with signal 0 does not deliver a signal; it only checks that the pid exists.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub fn fingerprint(profile_dir: &Path, family: Family) -> Vec<FileMark> {
    let names: &[&str] = match family {
        Family::Chromium => &["Default/Preferences", "Local State"],
        Family::Gecko => &["prefs.js", "times.json"],
    };
    names
        .iter()
        .map(|name| file_mark(profile_dir, name))
        .collect()
}

pub fn load_state(dir: &Path) -> GuardState {
    let Ok(text) = fs::read_to_string(dir.join("state.json")) else {
        return GuardState::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn record_launch(dir: &Path, generation: u32, pid: u32, at: i64) -> Result<(), StoreError> {
    let mut state = load_state(dir);
    state.last_launch = Some(LaunchMark {
        generation,
        pid,
        started_at: at,
    });
    save_state(dir, &state)
}

pub fn record_exit(
    dir: &Path,
    profile_dir: &Path,
    family: Family,
    at: i64,
) -> Result<(), StoreError> {
    let mut state = load_state(dir);
    state.last_exit = Some(ExitMark {
        at,
        files: fingerprint(profile_dir, family),
    });
    save_state(dir, &state)
}

pub fn check_before_launch(
    store: &IdentityStore,
    profile: &mut BrowserIdentityProfile,
    family: Family,
    policy: BypassPolicy,
    is_alive: &dyn Fn(u32) -> bool,
) -> Result<GuardOutcome, GuardError> {
    let profile_dir = store.profile_dir(&profile.id);
    if let Some(pid) = lock_pid(&profile_dir, family)
        && is_alive(pid)
    {
        return Err(GuardError::BrowserRunning);
    }
    if profile.bypass_suspected && policy == BypassPolicy::Block {
        return Err(GuardError::BypassSuspected);
    }
    let state = load_state(&store.dir(&profile.id));
    if state.last_exit.is_none() && state.last_launch.is_none() {
        return Ok(GuardOutcome::FirstLaunch);
    }
    let current = fingerprint(&profile_dir, family);
    let changed = match &state.last_exit {
        None => true,
        Some(exit) => exit.files != current,
    };
    if !changed {
        return Ok(GuardOutcome::Clean);
    }
    match policy {
        BypassPolicy::Block => {
            profile.bypass_suspected = true;
            profile.record(crate::common::now(), "bypass-suspected");
            store
                .save(profile)
                .map_err(|_| GuardError::BypassSuspected)?;
            Err(GuardError::BypassSuspected)
        }
        BypassPolicy::Warn => {
            profile.record(crate::common::now(), "bypass-warned");
            store
                .save(profile)
                .map_err(|_| GuardError::BypassSuspected)?;
            Ok(GuardOutcome::Warned)
        }
    }
}

pub fn confirm(
    store: &IdentityStore,
    profile: &mut BrowserIdentityProfile,
    now: i64,
) -> Result<(), StoreError> {
    if !profile.bypass_suspected {
        return Ok(());
    }
    profile.bypass_suspected = false;
    profile.record(now, "bypass-confirmed");
    let dir = store.dir(&profile.id);
    let mut state = load_state(&dir);
    state.last_exit = Some(ExitMark {
        at: now,
        files: fingerprint(&store.profile_dir(&profile.id), profile.engine.family),
    });
    save_state(&dir, &state)?;
    store.save(profile)
}

fn save_state(dir: &Path, state: &GuardState) -> Result<(), StoreError> {
    let bytes = serde_json::to_vec_pretty(state).map_err(|_| StoreError::Invalid)?;
    store::write_private(&dir.join("state.json"), &bytes)
}

fn file_mark(profile_dir: &Path, name: &str) -> FileMark {
    let path = profile_dir.join(name);
    let missing = FileMark {
        path: name.to_string(),
        present: false,
        size: 0,
        mtime_ns: 0,
        sha256: String::new(),
    };
    let Ok(meta) = fs::metadata(&path) else {
        return missing;
    };
    if !meta.is_file() {
        return missing;
    }
    let Ok(bytes) = fs::read(&path) else {
        return missing;
    };
    FileMark {
        path: name.to_string(),
        present: true,
        size: bytes.len() as u64,
        mtime_ns: mtime_ns(&meta),
        sha256: sha256_hex(&bytes),
    }
}

fn mtime_ns(meta: &fs::Metadata) -> i128 {
    let seconds = meta.mtime() as i128;
    let nanos = meta.mtime_nsec() as i128;
    seconds.saturating_mul(1_000_000_000).saturating_add(nanos)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn lock_pid(profile_dir: &Path, family: Family) -> Option<u32> {
    let path = match family {
        Family::Chromium => profile_dir.join("SingletonLock"),
        Family::Gecko => profile_dir.join("lock"),
    };
    let target = fs::read_link(path).ok()?;
    let text = target.to_string_lossy();
    let text = text.trim();
    let pid = match family {
        Family::Chromium => text.rsplit_once('-')?.1,
        Family::Gecko => text.rsplit_once(":+")?.1,
    };
    let pid = pid.parse::<u32>().ok()?;
    (pid > 0).then_some(pid)
}

mod i128_string {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &i128, serializer: S) -> Result<S::Ok, S::Error> {
        let narrowed = i64::try_from(*value).map_err(serde::ser::Error::custom)?;
        serializer.serialize_i64(narrowed)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i128, D::Error> {
        Ok(i64::deserialize(deserializer)? as i128)
    }
}
