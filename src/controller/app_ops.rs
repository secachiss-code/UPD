//! Запуск приложения внутри netns своего туннеля и только при живом ядре.

use crate::app::spec;
use crate::common::test_mode;
use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::core_ops::Workers;
use super::drop::{self, run_as_for};
use super::harden::ChildLimits;
use super::net_ops::NetCtx;
use super::owner::Owned;
use super::peer::PeerIdentity;
use super::protocol::{ControlError, Reply, ReplyData};
use super::registry::Generations;

const MAX_ENV_BYTES: usize = 4096;

#[allow(clippy::too_many_arguments)]
pub fn launch(
    owned: &Owned,
    id: &str,
    generation: u64,
    program: &str,
    args: &[String],
    peer: &PeerIdentity,
    workers: &Workers,
    nets: &NetCtx,
    base: &Path,
) -> Result<Reply, ControlError> {
    let Some(index) = super::net_ops::current_index(nets, owned)? else {
        return Err(ControlError::NotRunning);
    };
    let generations = Generations::load(&owned.generations_path())?;
    if generations
        .current(&format!("net:{}", owned.instance.as_str()))
        .is_none()
    {
        return Err(ControlError::NotRunning);
    }
    // Поколение запроса — поколение worker-а, которое клиент прочитал из `WorkerStatus`.
    // Ядро обязано отвечать: запись в карте остаётся и после гибели процесса.
    let status = workers.status(owned, "status");
    let Some(ReplyData::Status {
        running: true,
        generation: Some(current),
        api,
        ..
    }) = status.data
    else {
        return Err(ControlError::NotRunning);
    };
    if api != "api_ready" {
        return Err(ControlError::NotRunning);
    }
    if current != generation {
        return Err(ControlError::GenerationMismatch);
    }
    if spec::program_rejected(program, args) {
        return Err(ControlError::BadArgument);
    }
    let run_as = run_as_for(peer.uid, peer.gid)?;
    let env = env_of(owned, peer.uid)?;
    let netns_path = netns_root(base).join(format!("cm-{index}"));
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(&netns_path)
        .map_err(|_| ControlError::NotRunning)?;
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (key, value) in &env {
        command.env(key, value);
    }
    // Без файла DNS приложение не запускается: оно читало бы resolv.conf хоста.
    let resolv = nets
        .etc_root
        .join(format!("cm-{index}"))
        .join("resolv.conf");
    if !resolv.is_file() {
        return Err(ControlError::NotRunning);
    }
    if let Some(home) = env.get("HOME").filter(|home| Path::new(home).is_dir()) {
        command.current_dir(home);
    } else {
        command.current_dir("/");
    }
    drop::drop_into_netns_pre_exec(
        &mut command,
        file.as_raw_fd(),
        Some(&resolv),
        run_as,
        &[],
        ChildLimits::default(),
    )?;
    // Дескриптор netns нужен только до exec потомка; у контроллера он закрывается здесь.
    let child = command.spawn().map_err(|_| ControlError::Failed)?;
    drop(file);
    let pid = child.id();
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(Reply::ok(id, Some(ReplyData::Launched { pid })))
}

pub fn env_of(
    owned: &Owned,
    uid: u32,
) -> Result<std::collections::BTreeMap<String, String>, ControlError> {
    let mut env = std::collections::BTreeMap::new();
    env.insert("PATH".to_owned(), "/usr/bin:/bin".to_owned());
    if let Some(home) = home_dir(uid) {
        env.insert("HOME".to_owned(), home);
    }
    env.insert("XDG_RUNTIME_DIR".to_owned(), format!("/run/user/{uid}"));
    if let Some((timezone, locale)) = read_env_preset(owned)? {
        env.insert("TZ".to_owned(), timezone);
        env.insert("LANG".to_owned(), locale);
    }
    Ok(env)
}

fn netns_root(base: &Path) -> PathBuf {
    if test_mode() {
        base.join("netns")
    } else {
        PathBuf::from("/run/netns")
    }
}

fn read_env_preset(owned: &Owned) -> Result<Option<(String, String)>, ControlError> {
    let path = owned
        .root
        .join("instances")
        .join(owned.instance.as_str())
        .join("env.json");
    if std::fs::symlink_metadata(&path).is_err() {
        return Ok(None);
    }
    let bytes = super::owner::read_owned_file(owned, &path, MAX_ENV_BYTES)?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| ControlError::InvalidConfig)?;
    let timezone = value
        .get("timezone")
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .to_owned();
    let locale = value
        .get("locale")
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .to_owned();
    if !crate::identity::tzdata::valid_zone_name(&timezone)
        || locale.is_empty()
        || locale.len() > 64
        || !locale
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-' | b'@'))
    {
        return Err(ControlError::InvalidConfig);
    }
    Ok(Some((timezone, locale)))
}

fn home_dir(uid: u32) -> Option<String> {
    // SAFETY: passwd is a plain C struct; the all-zero bit pattern is valid.
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0u8; 16 * 1024];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: passwd and the buffer outlive the call; on success pw_dir points into `buf`.
    let rc = unsafe {
        libc::getpwuid_r(
            uid,
            &mut pw,
            buf.as_mut_ptr().cast(),
            buf.len(),
            &mut result,
        )
    };
    if rc != 0 || result.is_null() {
        return None;
    }
    // SAFETY: getpwuid_r succeeded, so pw_dir is a NUL-terminated string inside `buf`.
    let dir = unsafe { std::ffi::CStr::from_ptr(pw.pw_dir) };
    Some(dir.to_string_lossy().into_owned())
}
