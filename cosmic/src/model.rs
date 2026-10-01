//! Данные и действия, общие для апплета и окна: снимки состояния, обращения к помощнику,
//! запуск окна и терминала, настройки пользователя, направление текста.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;
use upd::common::{fmt_bytes, load_json, UpdState};
use upd::helper::{self, Request};
use upd::summary::{OpStatus, Summary};
use upd::{backend, i18n, vpn};

pub const APP_ID: &str = "io.github.upd";
pub const APPLET_ID: &str = "io.github.upd.Applet";
/// Своя символическая иконка приложения (ставится в hicolor вместе с апплетом).
pub const ICON: &str = "io.github.upd-symbolic";

/// Язык интерфейса — как у upd (`lang` в /etc/upd.conf или локаль).
pub fn init_lang() -> i18n::Lang {
    let l = i18n::resolve(&upd::common::conf_lang());
    i18n::set(l);
    l
}

/// Арабский пишется справа налево: ряды зеркалятся, текст выравнивается по правому краю.
pub fn rtl() -> bool {
    i18n::cur() == i18n::Lang::Ar
}

/// Снимок для панели. Помощник спрашивается, только если upd занят (иначе он запускался бы каждые несколько секунд).
pub fn load_summary() -> Result<Summary, String> {
    backend::detect().map(|b| upd::summary::gather(b.as_ref()))
}

/// Занят ли upd: команда держит блокировку /var/lib/upd/.lock (flock), её видно и обычному пользователю.
pub fn upd_busy() -> bool {
    use std::os::fd::AsRawFd;
    let path = PathBuf::from(upd::common::state_dir()).join(".lock");
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
    let dir = PathBuf::from(upd::common::state_dir());
    ["updates.json", "mirrors.json", "vpn.json"]
        .iter()
        .filter_map(|f| std::fs::metadata(dir.join(f)).ok()?.modified().ok())
        .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(b))
}

/// Состояние операции: у помощника, если он её ведёт; иначе — по блокировке upd (фоновая проверка, TUI).
pub fn load_op(ask_helper: bool) -> Result<OpStatus, String> {
    let busy = upd_busy();
    if (busy || ask_helper) && helper::available() {
        let st = helper::call_as::<OpStatus>(&Request::Status)?;
        if st.running || !busy { return Ok(st); }
    }
    Ok(OpStatus { running: busy, command: if busy { "auto".into() } else { String::new() }, ..Default::default() })
}

pub fn vpn_snapshot() -> Result<vpn::Snapshot, String> {
    helper::call_as(&Request::VpnSnapshot)
}

/// Главная группа и текущий сервер: Proxy → ⚡ Auto → 🇩🇪 DE-1.
pub fn vpn_current(s: &vpn::Snapshot) -> Option<(String, Option<u64>)> {
    let chain = s.chain();
    let last = chain.last()?.clone();
    let delay = s.delay.get(&last).copied().filter(|d| *d > 0);
    Some((last, delay))
}

/// Серверы главной группы: сначала отвечающие, по задержке.
pub fn vpn_servers(s: &vpn::Snapshot, limit: usize) -> Option<(String, Vec<(String, Option<u64>)>)> {
    let group = s.chain().first()?.clone();
    let g = s.groups.iter().find(|g| g.name == group)?;
    let mut v: Vec<(String, Option<u64>)> = g.all.iter().map(|n| (n.clone(), s.delay.get(n).copied().filter(|d| *d > 0))).collect();
    v.sort_by_key(|(_, d)| d.unwrap_or(u64::MAX));
    v.truncate(limit);
    Some((group, v))
}

pub fn start(args: &[&str]) -> Result<helper::OperationId, String> {
    helper::call_as::<helper::StartReply>(&Request::Start { args: args.iter().map(|s| s.to_string()).collect() })
        .map(|reply| reply.operation_id)
}

/// Окно upd: страница и, при необходимости, команда, которую окно запустит и покажет.
pub fn open_window(page: Option<&str>, run: Option<&str>) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    open_window_from(exe, page, run)
}

fn open_window_from(mut exe: PathBuf, page: Option<&str>, run: Option<&str>) -> Result<(), String> {
    // Linux current_exe points at the unlinked inode after an atomic update.
    // Launch the replacement at its installed path, preserving its directory.
    if !exe.exists() {
        if let Some(name) = exe.file_name().and_then(|name| name.to_str()).and_then(|name| name.strip_suffix(" (deleted)")) {
            let replacement = exe.with_file_name(name);
            if replacement.is_file() { exe = replacement; }
        }
    }
    let mut c = Command::new(exe);
    if let Some(p) = page {
        c.args(["--page", p]);
    }
    if let Some(r) = run {
        c.args(["--run", r]);
    }
    crate::launch::spawn(&mut c).map(|_| ()).map_err(|e| format!("upd-cosmic: {e}"))
}

/// Интерактивные инструменты (pacdiff, редактор правил VPN) запускаются в терминале.
pub fn open_terminal(args: &[&str]) -> Result<(), String> {
    let upd = if std::path::Path::new("/usr/bin/upd").exists() { "/usr/bin/upd" } else { "upd" };
    let mut cmd: Vec<String> = vec![upd.into()];
    cmd.extend(args.iter().map(|s| s.to_string()));
    cmd.push("--pause".into());
    let candidates: [(&str, &[&str]); 7] = [
        ("cosmic-term", &["--"]),
        ("xdg-terminal-exec", &[]),
        ("gnome-terminal", &["--"]),
        ("konsole", &["-e"]),
        ("alacritty", &["-e"]),
        ("kitty", &[]),
        ("xterm", &["-e"]),
    ];
    for (term, pre) in candidates {
        match crate::launch::spawn(Command::new(term).args(pre).args(&cmd)) {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("{term}: {error}")),
        }
    }
    Err(upd::t!("не найден эмулятор терминала").into())
}

pub fn open_url(address: &str) -> Result<(), String> {
    let url = url::Url::parse(address).map_err(|_| upd::t!("Некорректная веб-ссылка").to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(upd::t!("Разрешены только веб-ссылки HTTP и HTTPS").into());
    }
    crate::launch::spawn(Command::new("xdg-open").arg(url.as_str())).map(|_| ()).map_err(|e| format!("xdg-open: {e}"))
}

pub fn reboot() -> Result<(), String> {
    upd::common::run(true, &[], "systemctl", &["reboot"])
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
    base.join("upd").join("cosmic.json")
}

pub fn load_prefs() -> Prefs {
    std::fs::read(prefs_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_prefs(p: &Prefs) -> Result<(), String> {
    let path = prefs_path();
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, serde_json::to_vec_pretty(p).unwrap_or_default()).map_err(|e| e.to_string())
}

// ---------- тексты ----------

/// Разбивка обновлений для второй строки: «Пакеты 8 · Flatpak 2».
pub fn updates_breakdown(s: &Summary) -> Vec<(&'static str, usize)> {
    let mut v = vec![];
    if !s.packages.is_empty() {
        v.push((upd::t!("Пакеты"), s.packages.len()));
    }
    if !s.flatpak.is_empty() {
        v.push(("Flatpak", s.flatpak.len()));
    }
    if !s.firmware.is_empty() {
        v.push((upd::t!("Прошивки"), s.firmware.len()));
    }
    v
}

/// «скачаны заранее · 312 МБ» / «312 МБ к загрузке».
pub fn download_note(s: &Summary) -> Option<String> {
    if s.packages.is_empty() {
        return None;
    }
    Some(match (s.downloaded, s.download_size) {
        (true, Some(b)) => upd::t!("скачаны заранее · {0}", fmt_bytes(b)),
        (true, None) => upd::t!("скачаны заранее").into(),
        (false, Some(b)) => upd::t!("к загрузке {0}", fmt_bytes(b)),
        (false, None) => return None,
    })
}

/// Подпись задержки сервера VPN.
pub fn delay_text(d: Option<u64>) -> String {
    match d {
        Some(ms) => upd::t!("{0} мс", ms),
        None => "—".into(),
    }
}

/// Полный список обновлений из updates.json (для окна).
pub fn update_state() -> UpdState {
    load_json("updates.json")
}

/// Ограничение длины строки для узких мест (всплывающее окно).
pub fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}

#[derive(Clone, Debug)]
pub struct Metadata { pub busy: bool, pub stamp: u64, pub lang: i18n::Lang, pub launch_errors: Vec<String> }
pub fn metadata() -> Metadata { Metadata { busy: upd_busy(), stamp: state_stamp(), lang: init_lang(), launch_errors: crate::launch::errors() } }

#[cfg(test)]
mod launcher_tests {
    use super::*;
    use upd::common::contract_fixtures::{TempDirGuard, with_prepend_path};
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn replaced_applet_launches_the_new_window_binary() {
        let dir = TempDirGuard::new("cosmic-replaced-applet").unwrap();
        let marker = dir.path().join("arguments");
        let executable = dir.path().join("upd-cosmic");
        std::fs::write(&executable, format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n", marker.display())).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        open_window_from(dir.path().join("upd-cosmic (deleted)"), Some("settings"), Some("check")).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !marker.exists() { assert!(std::time::Instant::now() < deadline); std::thread::sleep(std::time::Duration::from_millis(10)); }
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "--page\nsettings\n--run\ncheck\n");
        std::fs::remove_file(executable).unwrap();
        assert!(open_window_from(dir.path().join("upd-cosmic (deleted)"), None, None).is_err());
    }
    #[test]
    fn web_scheme_validation_precedes_launcher_and_https_is_one_argument() {
        let dir = TempDirGuard::new("cosmic-web-launch").unwrap();
        let marker = dir.path().join("arguments");
        let body = format!("#!/bin/sh\nprintf '%s\\n' \"$#\" \"$1\" > '{}'\n", marker.display());
        std::fs::write(dir.path().join("xdg-open"), body).unwrap();
        std::fs::set_permissions(dir.path().join("xdg-open"), std::fs::Permissions::from_mode(0o755)).unwrap();
        with_prepend_path(dir.path(), || {
            for url in ["file:///tmp/test", "javascript:alert(1)", "data:text/plain,test", "not a url"] { assert!(open_url(url).is_err()); }
            assert!(!marker.exists());
            open_url("https://example.invalid/a?x=1&y=2").unwrap();
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !marker.exists() { assert!(std::time::Instant::now() < deadline); std::thread::sleep(std::time::Duration::from_millis(10)); }
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "1\nhttps://example.invalid/a?x=1&y=2\n");
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
            let mut env = upd::common::contract_fixtures::EnvGuard::new();
            env.set("PATH", dir.path());
            open_terminal(&["vpn", "rules"])
        }).unwrap_err();
        assert!(error.contains("cosmic-term")); assert!(!marker.exists());
    }
}
