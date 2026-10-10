//! Общие хелперы тестов направления BI (роль 2). Подключается из `tests/audit_bi_*.rs`
//! через `#[path]`; сам по себе не является тестом.
//!
//! Правило сдачи: проверки собираются в `Checks`, а не падают на первой ошибке. Каждое
//! расхождение печатается как `Exx: ожидалось …, получено …` — это и есть запись дефекта.

#![allow(dead_code)]

use cm::identity::engine::{Brand, EngineInfo, Family};
use cm::identity::model::{BrowserIdentityProfile, Strategy};
use cm::identity::presets::EnvironmentProfile;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

pub const ZONEINFO: &str = "/usr/share/zoneinfo";
pub const TIME_NOW: i64 = 1_700_000_000;

/// Накопитель проверок одного ребра.
pub struct Checks {
    edge: &'static str,
    failures: Vec<String>,
    passed: usize,
}

impl Checks {
    pub fn new(edge: &'static str) -> Self {
        Self {
            edge,
            failures: Vec::new(),
            passed: 0,
        }
    }

    /// Сравнение значения с ожидаемым из спецификации ребра.
    pub fn eq<T: Debug + PartialEq>(&mut self, what: &str, expected: T, actual: T) {
        if expected == actual {
            self.passed += 1;
        } else {
            self.failures.push(format!(
                "{}: {what}: ожидалось {expected:?}, получено {actual:?}",
                self.edge
            ));
        }
    }

    /// Булево условие из спецификации ребра.
    pub fn holds(&mut self, what: &str, holds: bool) {
        if holds {
            self.passed += 1;
        } else {
            self.failures.push(format!(
                "{}: {what}: ожидалось истина, получено ложь",
                self.edge
            ));
        }
    }

    /// Завершает ребро: при расхождениях тест красный, и каждое расхождение перечислено.
    pub fn finish(self) {
        if self.failures.is_empty() {
            eprintln!("{} PASS: проверок {}", self.edge, self.passed);
            return;
        }
        eprintln!(
            "{} FAIL: красных проверок {} из {}",
            self.edge,
            self.failures.len(),
            self.failures.len() + self.passed
        );
        panic!("{}", self.failures.join("\n"));
    }
}

/// Сериализует изменения окружения процесса между тестами одного файла.
pub fn env_guard() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set_env(key: &str, value: &str) {
    // SAFETY: вызывается только под `env_guard()`; конкурентных читателей окружения в тестах нет.
    unsafe { std::env::set_var(key, value) }
}

pub fn unset_env(key: &str) {
    // SAFETY: вызывается только под `env_guard()`; конкурентных читателей окружения в тестах нет.
    unsafe { std::env::remove_var(key) }
}

pub fn write_exec(path: &Path, body: &str) {
    fs::write(path, body).expect("запись фикстуры");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod фикстуры");
}

/// Браузер-заглушка на `sh`: на `--version` печатает строку версии, иначе ничего не делает.
pub fn sh_version_fixture(dir: &Path, name: &str, version_line: &str) -> PathBuf {
    let path = dir.join(name);
    write_exec(&path, &format!("#!/bin/sh\necho '{version_line}'\n"));
    path
}

/// Браузер-заглушка на python: на `--version` печатает версию; иначе дописывает в `runs.jsonl`
/// рядом с собой pid, sid, argv и env, спит `sleep_secs` и выходит с кодом 0.
pub fn recording_fixture(dir: &Path, name: &str, version_line: &str, sleep_secs: u64) -> PathBuf {
    let path = dir.join(name);
    let body = format!(
        r#"#!/usr/bin/python3
import json, os, sys, time
if sys.argv[1:] == ["--version"]:
    print({version_line:?})
    sys.exit(0)
here = os.path.dirname(os.path.abspath(sys.argv[0]))
# Окружение берём из /proc/self/environ: интерпретатор сам дописывает LC_CTYPE в os.environ.
initial = open("/proc/self/environ", "rb").read().split(b"\0")
env = dict(item.decode().split("=", 1) for item in initial if b"=" in item)
record = {{"pid": os.getpid(), "sid": os.getsid(0), "argv": sys.argv[1:], "env": env}}
with open(os.path.join(here, "runs.jsonl"), "a") as out:
    out.write(json.dumps(record) + "\n")
time.sleep({sleep_secs})
"#
    );
    write_exec(&path, &body);
    path
}

/// Записи запусков фикстуры из `runs.jsonl` в каталоге фикстуры.
pub fn read_runs(dir: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(dir.join("runs.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("runs.jsonl"))
        .collect()
}

pub fn engine(family: Family, brand: Brand, major: u32, version: &str) -> EngineInfo {
    EngineInfo {
        family,
        brand,
        major,
        version: version.to_string(),
    }
}

pub fn chrome155() -> EngineInfo {
    engine(Family::Chromium, Brand::Chrome, 155, "155.0.8059.39")
}

pub fn chrome156() -> EngineInfo {
    engine(Family::Chromium, Brand::Chrome, 156, "156.0.1.1")
}

pub fn chromium155() -> EngineInfo {
    engine(Family::Chromium, Brand::Chromium, 155, "155.0.7000.1")
}

pub fn brave154() -> EngineInfo {
    engine(Family::Chromium, Brand::Brave, 154, "154.1.96.61")
}

pub fn firefox157() -> EngineInfo {
    engine(Family::Gecko, Brand::Firefox, 157, "157.0")
}

pub fn librewolf157() -> EngineInfo {
    engine(Family::Gecko, Brand::LibreWolf, 157, "157.0-1")
}

/// Окружение DE/local, записанное литералами (не через пресеты: пресеты проверяет E01/E02).
pub fn de_env() -> EnvironmentProfile {
    EnvironmentProfile {
        country: "DE".to_string(),
        timezone: "Europe/Berlin".to_string(),
        languages: vec![
            "de-DE".to_string(),
            "de".to_string(),
            "en-US".to_string(),
            "en".to_string(),
        ],
        posix_locale: "de_DE.UTF-8".to_string(),
        matches_country: true,
    }
}

pub fn profile(
    id: &str,
    strategy: Strategy,
    browser: &Path,
    engine: EngineInfo,
    environment: Option<EnvironmentProfile>,
) -> BrowserIdentityProfile {
    BrowserIdentityProfile {
        schema_version: 1,
        id: id.to_string(),
        strategy,
        browser: browser.to_path_buf(),
        engine,
        environment,
        extra_args: Vec::new(),
        created_at: TIME_NOW,
        generation: 0,
        history: Vec::new(),
        bypass_suspected: false,
    }
}

/// Родительское окружение с секретом и системными переменными, которые план не должен пропускать.
pub fn parent_env() -> BTreeMap<String, String> {
    [
        ("DISPLAY", ":0"),
        ("WAYLAND_DISPLAY", "wayland-1"),
        ("XAUTHORITY", "/h/.Xauthority"),
        ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
        ("HOME", "/h"),
        ("PATH", "/usr/bin"),
        ("LC_TIME", "ru_RU.UTF-8"),
        ("LANGUAGE", "ru"),
        ("TZ", "Europe/Moscow"),
        ("SECRET_TOKEN", "x"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}
