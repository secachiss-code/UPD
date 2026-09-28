//! Данные и действия, общие для апплета и окна: снимки состояния, обращения к помощнику,
//! запуск окна и терминала, настройки пользователя, направление текста.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::{Command, Stdio};
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
pub fn load_summary() -> Summary {
    match backend::detect() {
        Ok(b) => upd::summary::gather(b.as_ref()),
        Err(_) => Summary::default(),
    }
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
pub fn load_op(ask_helper: bool) -> OpStatus {
    let busy = upd_busy();
    if (busy || ask_helper) && helper::available() {
        if let Ok(st) = helper::call_as::<OpStatus>(&Request::Status) {
            if st.running || !busy {
                return st;
            }
        }
    }
    OpStatus { running: busy, command: if busy { "auto".into() } else { String::new() }, ..Default::default() }
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

pub fn start(args: &[&str]) -> Result<(), String> {
    helper::call(&Request::Start { args: args.iter().map(|s| s.to_string()).collect() }).map(|_| ())
}

/// Окно upd: страница и, при необходимости, команда, которую окно запустит и покажет.
pub fn open_window(page: Option<&str>, run: Option<&str>) {
    let exe = std::env::current_exe().unwrap_or_else(|_| "upd-cosmic".into());
    let mut c = Command::new(exe);
    if let Some(p) = page {
        c.args(["--page", p]);
    }
    if let Some(r) = run {
        c.args(["--run", r]);
    }
    let _ = c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
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
        if upd::common::have(term) {
            return Command::new(term)
                .args(pre)
                .args(&cmd)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map(|_| ())
                .map_err(|e| format!("{term}: {e}"));
        }
    }
    Err(upd::t!("не найден эмулятор терминала").into())
}

pub fn open_url(url: &str) {
    let _ = Command::new("xdg-open").arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
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
