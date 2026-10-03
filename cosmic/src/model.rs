//! Состояние панели, настройки уведомлений и запуск TUI.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;
use cm::helper::{self, Request};
use cm::summary::{OpStatus, Summary};
use cm::{backend, i18n};

pub const APP_ID: &str = "io.github.cm";
pub const APPLET_ID: &str = "io.github.cm.Applet";

/// Язык интерфейса — как у cm (`lang` в /etc/cm.conf или локаль).
pub fn init_lang() -> i18n::Lang {
    let l = i18n::resolve(&cm::common::conf_lang());
    i18n::set(l);
    l
}

/// Снимок для панели. Помощник спрашивается, только если cm занят (иначе он запускался бы каждые несколько секунд).
pub fn load_summary() -> Result<Summary, String> {
    backend::detect().map(|b| cm::summary::gather(b.as_ref()))
}

/// Занят ли cm: команда держит блокировку /var/lib/cm/.lock (flock), её видно и обычному пользователю.
pub fn cm_busy() -> bool {
    use std::os::fd::AsRawFd;
    let path = PathBuf::from(cm::common::state_dir()).join(".lock");
    let Ok(f) = std::fs::File::open(&path) else { return false };
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } == 0 {
        unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_UN) };
        false
    } else {
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EWOULDBLOCK)
    }
}

/// Подпись файлов состояния по времени изменения: изменилась — пора перечитать сводку.
pub fn state_stamp() -> u64 {
    let dir = PathBuf::from(cm::common::state_dir());
    ["updates.json", "mirrors.json", "vpn.json"]
        .iter()
        .filter_map(|f| std::fs::metadata(dir.join(f)).ok()?.modified().ok())
        .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(b))
}

/// Состояние операции: у помощника, если он её ведёт; иначе — по блокировке cm (фоновая проверка, TUI).
pub fn load_op(ask_helper: bool) -> Result<OpStatus, String> {
    let busy = cm_busy();
    if (busy || ask_helper) && helper::available() {
        let st = helper::call_as::<OpStatus>(&Request::Status)?;
        if st.running || !busy { return Ok(st); }
    }
    Ok(OpStatus { running: busy, command: if busy { "auto".into() } else { String::new() }, ..Default::default() })
}

pub fn open_tui() -> Result<(), String> { open_tui_new(false) }
pub fn open_tui_new(force_new: bool) -> Result<(), String> {
    crate::tui_launch::open(force_new, || spawn_terminal(&["tui"]))
}

#[cfg(test)]
fn open_terminal(args: &[&str]) -> Result<(), String> { spawn_terminal(args).map(|_| ()) }

fn spawn_terminal(args: &[&str]) -> Result<u32, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let directory = exe.parent().unwrap_or_else(|| std::path::Path::new("."));
    let cm = [directory.join("cm"), directory.join("cm-linux-amd64")]
        .into_iter().find(|path| path.is_file()).unwrap_or_else(|| "cm".into());
    launch_terminal(&cm, args)
}

fn launch_terminal(cm: &std::path::Path, args: &[&str]) -> Result<u32, String> {
    let candidates: [(&str, &[&str]); 7] = [
        ("cosmic-term", &["--"]), ("xdg-terminal-exec", &[]),
        ("gnome-terminal", &["--"]), ("konsole", &["-e"]),
        ("alacritty", &["-e"]), ("kitty", &[]), ("xterm", &["-e"]),
    ];
    for (term, pre) in candidates {
        match crate::launch::spawn(Command::new(term).args(pre).arg(cm).args(args)) {
            Ok(pid) => return Ok(pid),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("{term}: {error}")),
        }
    }
    Err(cm::t!("не найден эмулятор терминала").into())
}

// ---------- настройки пользователя (не системные) ----------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Prefs {
    #[serde(default = "yes")]
    pub notifications: bool,
}

fn yes() -> bool {
    true
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { notifications: true }
    }
}

fn prefs_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"));
    base.join("cm").join("cosmic.json")
}

pub fn load_prefs() -> Prefs {
    std::fs::read(prefs_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

// ---------- тексты ----------

/// Разбивка обновлений для второй строки: «Пакеты 8 · Flatpak 2».
pub fn updates_breakdown(s: &Summary) -> Vec<(&'static str, usize)> {
    let mut v = vec![];
    if !s.packages.is_empty() {
        v.push((cm::t!("Пакеты"), s.packages.len()));
    }
    if !s.flatpak.is_empty() {
        v.push(("Flatpak", s.flatpak.len()));
    }
    if !s.firmware.is_empty() {
        v.push((cm::t!("Прошивки"), s.firmware.len()));
    }
    v
}

#[derive(Clone, Debug)]
pub struct Metadata { pub tui_running: bool, pub busy: bool, pub stamp: u64, pub lang: i18n::Lang, pub launch_errors: Vec<String> }
pub fn metadata() -> Metadata { Metadata { tui_running: cm::summary::tui_running(), busy: cm_busy(), stamp: state_stamp(), lang: init_lang(), launch_errors: crate::launch::errors() } }

#[cfg(test)]
mod launcher_tests {
    use super::*;
    use cm::common::contract_fixtures::{TempDirGuard, with_prepend_path};
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn tui_launcher_passes_binary_and_tui_as_separate_arguments() {
        let dir = TempDirGuard::new("cosmic-tui-launch").unwrap();
        let marker = dir.path().join("arguments");
        let executable = dir.path().join("cosmic-term");
        std::fs::write(&executable, format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n", marker.display())).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        with_prepend_path(dir.path(), || launch_terminal(std::path::Path::new("/tmp/cm with spaces"), &["tui"]).unwrap());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::fs::read_to_string(&marker).unwrap_or_default() != "--\n/tmp/cm with spaces\ntui\n" { assert!(std::time::Instant::now() < deadline); std::thread::sleep(std::time::Duration::from_millis(10)); }
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "--\n/tmp/cm with spaces\ntui\n");
    }
    #[test]
    fn terminal_permission_error_does_not_launch_fallback() {
        let dir = TempDirGuard::new("cosmic-terminal-failure").unwrap();
        std::fs::write(dir.path().join("cosmic-term"), "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(dir.path().join("cosmic-term"), std::fs::Permissions::from_mode(0o644)).unwrap();
        let marker = dir.path().join("fallback");
        std::fs::write(dir.path().join("xdg-terminal-exec"), format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
        std::fs::set_permissions(dir.path().join("xdg-terminal-exec"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = with_prepend_path(dir.path(), || {
            let mut env = cm::common::contract_fixtures::EnvGuard::new();
            env.set("PATH", dir.path());
            open_terminal(&["vpn", "rules"])
        }).unwrap_err();
        assert!(error.contains("cosmic-term")); assert!(!marker.exists());
    }
}
