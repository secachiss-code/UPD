//! Распознавание браузера по его собственному `--version`.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
const VERSION_OUTPUT_LIMIT: u64 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Family {
    Chromium,
    Gecko,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Brand {
    Chrome,
    Chromium,
    Brave,
    Firefox,
    LibreWolf,
}

impl Brand {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Chromium => "Chromium",
            Self::Brave => "Brave",
            Self::Firefox => "Firefox",
            Self::LibreWolf => "LibreWolf",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineInfo {
    pub family: Family,
    pub brand: Brand,
    pub major: u32,
    pub version: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineError {
    UnknownEngine,
    NotAbsolute,
    Timeout,
    VersionFailed,
}

impl EngineError {
    pub fn code(self) -> &'static str {
        match self {
            Self::UnknownEngine => "UnknownEngine",
            Self::NotAbsolute => "NotAbsolute",
            Self::Timeout => "Timeout",
            Self::VersionFailed => "VersionFailed",
        }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnknownEngine => t!("неизвестный браузер"),
            Self::NotAbsolute => t!("путь к браузеру должен быть абсолютным"),
            Self::Timeout => t!("браузер не ответил вовремя"),
            Self::VersionFailed => t!("не удалось узнать версию браузера"),
        })
    }
}

pub fn parse_version(stdout: &str) -> Result<EngineInfo, EngineError> {
    let line = stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    const RULES: &[(&str, Family, Brand)] = &[
        ("Google Chrome ", Family::Chromium, Brand::Chrome),
        ("Chromium ", Family::Chromium, Brand::Chromium),
        ("Brave Browser ", Family::Chromium, Brand::Brave),
        ("Mozilla Firefox ", Family::Gecko, Brand::Firefox),
        ("LibreWolf ", Family::Gecko, Brand::LibreWolf),
    ];
    for (prefix, family, brand) in RULES {
        let Some(rest) = line.strip_prefix(prefix) else {
            continue;
        };
        let version = rest.split_whitespace().next().unwrap_or("");
        if version.is_empty() {
            return Err(EngineError::UnknownEngine);
        }
        return Ok(EngineInfo {
            family: *family,
            brand: *brand,
            major: major_version(version)?,
            version: version.to_string(),
        });
    }
    Err(EngineError::UnknownEngine)
}

pub fn detect(path: &Path) -> Result<EngineInfo, EngineError> {
    if !path.is_absolute() {
        return Err(EngineError::NotAbsolute);
    }
    let mut child = Command::new(path)
        .arg("--version")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| EngineError::VersionFailed)?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= VERSION_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(EngineError::Timeout);
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(EngineError::VersionFailed);
            }
        }
    };
    if !status.success() {
        return Err(EngineError::VersionFailed);
    }
    let stdout = child.stdout.take().ok_or(EngineError::VersionFailed)?;
    let mut buffer = Vec::new();
    stdout
        .take(VERSION_OUTPUT_LIMIT)
        .read_to_end(&mut buffer)
        .map_err(|_| EngineError::VersionFailed)?;
    parse_version(&String::from_utf8_lossy(&buffer))
}

fn major_version(version: &str) -> Result<u32, EngineError> {
    let bytes = version.as_bytes();
    let start = bytes
        .iter()
        .position(|byte| byte.is_ascii_digit())
        .ok_or(EngineError::UnknownEngine)?;
    let end = start
        + bytes[start..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
    version[start..end]
        .parse()
        .map_err(|_| EngineError::UnknownEngine)
}
