//! Общее: пути, конфиг, состояние, запуск команд, сведения о системе.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub type Log<'a> = &'a dyn Fn(&str);

pub fn env_or(k: &str, def: &str) -> String {
    std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| def.to_string())
}

pub fn conf_path() -> String {
    env_or("UPD_CONF", "/etc/upd.conf")
}
pub fn state_dir() -> String {
    env_or("UPD_STATE_DIR", "/var/lib/upd")
}
pub fn test_mode() -> bool {
    std::env::var("UPD_STATE_DIR").is_ok()
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

// ---------- конфиг ----------

#[derive(Clone, Debug)]
pub struct Config {
    pub keep: usize,
    pub timeout: u64,
    pub extra_from_list: usize,
    pub rescan_count: usize,
    pub retries: u32,
    pub mirror_max_age_h: i64,
    pub network_memory_days: i64,
    pub max_lag_h: i64,
    pub parallel: usize,
    pub parallel_vpn: usize,
    pub prefetch: bool,
    pub prefetch_on_battery: bool,
    pub prefetch_on_metered: bool,
    pub min_free_gb: u64,
    pub flatpak: bool,
    pub aur: bool,
    pub firmware: bool,
    pub news: bool,
    pub snapshot: bool,
    pub mirrors: Vec<String>,
}

/// (ключ, описание) — порядок и тексты для записи файла настроек.
const DOCS: &[(&str, &str)] = &[
    ("keep", "Сколько лучших рабочих зеркал ставить первыми"),
    ("timeout", "Таймаут замера одного зеркала, секунд"),
    ("extra_from_list", "Сколько зеркал из текущего списка дополнительно замерять при проверке"),
    ("rescan_count", "Сколько зеркал брать из автопоиска при смене сети"),
    ("retries", "Попыток скачать обновления, прежде чем сдаться"),
    ("mirror_max_age_h", "Перепроверять зеркала, если последний замер старше N часов"),
    ("network_memory_days", "Сколько дней помнить лучшие зеркала для каждой сети"),
    ("max_lag_h", "Отбрасывать зеркала, отставшие от самого свежего больше чем на N часов"),
    ("parallel", "Сколько зеркал замерять одновременно"),
    ("parallel_vpn", "То же, когда поднят VPN/TUN (прокси делит канал между замерами)"),
    ("prefetch", "Скачивать обновления заранее в фоне (1 — да, 0 — нет)"),
    ("prefetch_on_battery", "Фоновая загрузка при работе от батареи"),
    ("prefetch_on_metered", "Фоновая загрузка и замеры зеркал в лимитной сети (точка доступа телефона)"),
    ("min_free_gb", "Сколько ГБ должно остаться свободными после загрузки"),
    ("flatpak", "Обновлять Flatpak"),
    ("aur", "Обновлять AUR (paru/yay), только при ручном обновлении"),
    ("firmware", "Проверять прошивки через fwupd"),
    ("news", "Показывать новости Arch, требующие ручного вмешательства, перед обновлением"),
    ("snapshot", "Делать снапшот перед обновлением, если этого не делает snap-pac"),
];

impl Config {
    pub fn defaults(mirrors: Vec<String>) -> Self {
        Config {
            keep: 3,
            timeout: 10,
            extra_from_list: 8,
            rescan_count: 15,
            retries: 5,
            mirror_max_age_h: 12,
            network_memory_days: 3,
            max_lag_h: 3,
            parallel: 4,
            parallel_vpn: 2,
            prefetch: true,
            prefetch_on_battery: false,
            prefetch_on_metered: false,
            min_free_gb: 2,
            flatpak: true,
            aur: true,
            firmware: true,
            news: true,
            snapshot: true,
            mirrors,
        }
    }

    pub fn load(default_mirrors: Vec<String>) -> Self {
        let mut c = Config::defaults(default_mirrors);
        let Ok(text) = fs::read_to_string(conf_path()) else { return c };
        c.mirrors.clear();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            if k == "mirror" {
                if !contains(&c.mirrors, v) {
                    c.mirrors.push(v.to_string());
                }
                continue;
            }
            let Ok(n) = v.parse::<i64>() else { continue };
            let b = n != 0;
            match k {
                "keep" => c.keep = n.max(1) as usize,
                "timeout" => c.timeout = n.max(2) as u64,
                "extra_from_list" => c.extra_from_list = n.max(0) as usize,
                "rescan_count" => c.rescan_count = n.max(3) as usize,
                "retries" => c.retries = n.max(1) as u32,
                "mirror_max_age_h" => c.mirror_max_age_h = n,
                "network_memory_days" => c.network_memory_days = n,
                "max_lag_h" => c.max_lag_h = n.max(1),
                "parallel" => c.parallel = n.max(1) as usize,
                "parallel_vpn" => c.parallel_vpn = n.max(1) as usize,
                "prefetch" => c.prefetch = b,
                "prefetch_on_battery" => c.prefetch_on_battery = b,
                "prefetch_on_metered" => c.prefetch_on_metered = b,
                "min_free_gb" => c.min_free_gb = n.max(0) as u64,
                "flatpak" => c.flatpak = b,
                "aur" => c.aur = b,
                "firmware" => c.firmware = b,
                "news" => c.news = b,
                "snapshot" => c.snapshot = b,
                _ => {}
            }
        }
        c
    }

    fn value(&self, k: &str) -> i64 {
        match k {
            "keep" => self.keep as i64,
            "timeout" => self.timeout as i64,
            "extra_from_list" => self.extra_from_list as i64,
            "rescan_count" => self.rescan_count as i64,
            "retries" => self.retries as i64,
            "mirror_max_age_h" => self.mirror_max_age_h,
            "network_memory_days" => self.network_memory_days,
            "max_lag_h" => self.max_lag_h,
            "parallel" => self.parallel as i64,
            "parallel_vpn" => self.parallel_vpn as i64,
            "prefetch" => self.prefetch as i64,
            "prefetch_on_battery" => self.prefetch_on_battery as i64,
            "prefetch_on_metered" => self.prefetch_on_metered as i64,
            "min_free_gb" => self.min_free_gb as i64,
            "flatpak" => self.flatpak as i64,
            "aur" => self.aur as i64,
            "firmware" => self.firmware as i64,
            "news" => self.news as i64,
            "snapshot" => self.snapshot as i64,
            _ => 0,
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let mut s = String::from("# upd — настройки. Правится вручную или через TUI (upd → Зеркала).\n");
        for (k, doc) in DOCS {
            s += &format!("\n# {doc}\n{k} = {}\n", self.value(k));
        }
        s += "\n# Предпочитаемые зеркала: замеряются всегда, наравне с найденными автоматически\n";
        for m in &self.mirrors {
            s += &format!("mirror = {m}\n");
        }
        atomic_write(Path::new(&conf_path()), s.as_bytes(), 0o644)
    }
}

// ---------- состояние ----------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Probe {
    pub url: String,
    #[serde(default)]
    pub src: String,
    #[serde(default)]
    pub ok: bool,
    /// скорость этого замера, байт/с
    #[serde(default)]
    pub speed: f64,
    /// сглаженная скорость (по истории замеров в этой сети)
    #[serde(default)]
    pub score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lag_h: Option<f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub err: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Stat {
    pub ewma: f64,
    pub ok: u32,
    pub fail: u32,
    pub streak_fail: u32,
    pub last: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NetMem {
    #[serde(default)]
    pub best: Vec<String>,
    #[serde(default)]
    pub checked: i64,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub stats: BTreeMap<String, Stat>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MirrorState {
    #[serde(default)]
    pub checked: i64,
    #[serde(default)]
    pub results: Vec<Probe>,
    #[serde(default)]
    pub best: Vec<String>,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub networks: BTreeMap<String, NetMem>,
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub event_time: i64,
    #[serde(default)]
    pub hints: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct News {
    pub title: String,
    pub date: i64,
    pub link: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct UpdState {
    #[serde(default)]
    pub checked: i64,
    #[serde(default)]
    pub list: Vec<String>,
    #[serde(default)]
    pub downloaded: bool,
    #[serde(default)]
    pub download_size: u64,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub skipped: String,
    #[serde(default)]
    pub flatpak: Vec<String>,
    #[serde(default)]
    pub firmware: Vec<String>,
    #[serde(default)]
    pub news: Vec<News>,
}

pub fn load_json<T: for<'de> Deserialize<'de> + Default>(name: &str) -> T {
    fs::read(Path::new(&state_dir()).join(name))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_json<T: Serialize>(name: &str, v: &T) -> std::io::Result<()> {
    let dir = PathBuf::from(state_dir());
    fs::create_dir_all(&dir)?;
    let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o755));
    let data = serde_json::to_vec_pretty(v).unwrap_or_default();
    atomic_write(&dir.join(name), &data, 0o644)
}

pub fn atomic_write(path: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let tmp = dir.join(format!(".upd.{}.tmp", std::process::id()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
    fs::rename(&tmp, path)
}

/// Блокировка: одна тяжёлая задача за раз. Освобождается при закрытии файла.
pub struct Lock(#[allow(dead_code)] fs::File);

pub fn lock(block: bool) -> Result<Lock, String> {
    let dir = state_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let f = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(Path::new(&dir).join(".lock"))
        .map_err(|e| e.to_string())?;
    use std::os::fd::AsRawFd;
    let op = if block { libc::LOCK_EX } else { libc::LOCK_EX | libc::LOCK_NB };
    if unsafe { libc::flock(f.as_raw_fd(), op) } != 0 {
        return Err("busy".into());
    }
    Ok(Lock(f))
}

// ---------- команды ----------

pub fn have(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

/// Запуск с захватом stdout; код выхода (-1, если не запустилось).
pub fn out(cmd: &str, args: &[&str]) -> (String, i32) {
    match Command::new(cmd).args(args).env("LC_ALL", "C").stderr(Stdio::null()).output() {
        Ok(o) => (String::from_utf8_lossy(&o.stdout).into_owned(), o.status.code().unwrap_or(-1)),
        Err(_) => (String::new(), -1),
    }
}

/// Запуск с выводом в терминал (или молча). Ok — если код 0.
pub fn run(quiet: bool, env: &[(&str, &str)], cmd: &str, args: &[&str]) -> Result<(), String> {
    let mut c = Command::new(cmd);
    c.args(args);
    for (k, v) in env {
        c.env(k, v);
    }
    if quiet {
        c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
        let o = c.output().map_err(|e| format!("{cmd}: {e}"))?;
        if o.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&o.stderr);
        return Err(format!("{cmd}: {}", last_line(&err).unwrap_or("ошибка")));
    }
    let st = c.status().map_err(|e| format!("{cmd}: {e}"))?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("{cmd}: код {}", st.code().unwrap_or(-1)))
    }
}

pub fn lines(s: &str) -> Vec<String> {
    s.lines().filter(|l| !l.trim().is_empty()).map(String::from).collect()
}

pub fn last_line(s: &str) -> Option<&str> {
    s.lines().rev().map(str::trim).find(|l| !l.is_empty())
}

pub fn contains(list: &[String], s: &str) -> bool {
    list.iter().any(|x| x.trim_end_matches('/') == s.trim_end_matches('/'))
}

pub fn host_of(u: &str) -> &str {
    let u = u.trim_start_matches("https://").trim_start_matches("http://");
    u.split('/').next().unwrap_or(u)
}

/// Пользователь, от имени которого запущен sudo (для paru/flatpak --user).
pub fn invoking_user() -> Option<String> {
    std::env::var("SUDO_USER").ok().or_else(|| std::env::var("DOAS_USER").ok()).filter(|u| !u.is_empty() && u != "root")
}

pub fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

pub fn confirm(q: &str, default_yes: bool) -> bool {
    print!("{q} {} ", if default_yes { "[Y/n]" } else { "[y/N]" });
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    let _ = std::io::stdin().lock().read_line(&mut s);
    let s = s.trim().to_lowercase();
    if s.is_empty() {
        return default_yes;
    }
    matches!(s.as_str(), "y" | "yes" | "д" | "да")
}

// ---------- форматирование ----------

pub fn fmt_speed(p: &Probe) -> String {
    if !p.ok {
        return if p.err.is_empty() { "—".into() } else { p.err.clone() };
    }
    let s = if p.score > 0.0 { p.score } else { p.speed };
    if s >= 1048576.0 {
        format!("{:.1} MB/s", s / 1048576.0)
    } else {
        format!("{:.0} KB/s", s / 1024.0)
    }
}

pub fn fmt_bytes(n: u64) -> String {
    let n = n as f64;
    if n >= 1073741824.0 {
        format!("{:.1} ГБ", n / 1073741824.0)
    } else if n >= 1048576.0 {
        format!("{:.0} МБ", n / 1048576.0)
    } else {
        format!("{:.0} КБ", n / 1024.0)
    }
}

pub fn fmt_ago(ts: i64) -> String {
    if ts == 0 {
        return "не было".into();
    }
    let d = now() - ts;
    match d {
        _ if d < 60 => "только что".into(),
        _ if d < 3600 => format!("{} мин назад", d / 60),
        _ if d < 48 * 3600 => format!("{} ч назад", d / 3600),
        _ => format!("{} дн назад", d / 86400),
    }
}

/// Unix-время → «2026-09-24 22:19» (UTC-смещение берём из /etc/localtime через libc).
pub fn fmt_time(ts: i64) -> String {
    if ts == 0 {
        return "не было".into();
    }
    let t = ts as _;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:04}-{:02}-{:02} {:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min)
}

// ---------- даты ----------

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// RFC 2822 / RSS / apt: «Thu, 24 Sep 2026 20:10:05 +0000» (или UTC/GMT) → unix.
pub fn parse_rfc2822(s: &str) -> Option<i64> {
    let s = s.split_once(',').map(|x| x.1).unwrap_or(s).trim();
    let f: Vec<&str> = s.split_whitespace().collect();
    if f.len() < 4 {
        return None;
    }
    let d: i64 = f[0].parse().ok()?;
    const M: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let m = M.iter().position(|x| f[1].to_lowercase().starts_with(x))? as i64 + 1;
    let y: i64 = f[2].parse().ok()?;
    let hms: Vec<i64> = f[3].split(':').filter_map(|x| x.parse().ok()).collect();
    let (h, mi, se) = (*hms.first()?, *hms.get(1).unwrap_or(&0), *hms.get(2).unwrap_or(&0));
    let mut off = 0;
    if let Some(z) = f.get(4) {
        if (z.starts_with('+') || z.starts_with('-')) && z.len() == 5 {
            let v: i64 = z[1..].parse().ok()?;
            off = (v / 100 * 3600 + v % 100 * 60) * if z.starts_with('-') { -1 } else { 1 };
        }
    }
    Some(days_from_civil(y, m, d) * 86400 + h * 3600 + mi * 60 + se - off)
}

/// «[2026-09-24T22:14:30+0300]» из pacman.log → unix.
pub fn parse_iso(s: &str) -> Option<i64> {
    let s = s.trim_matches(|c| c == '[' || c == ']');
    let (date, rest) = s.split_once('T')?;
    let dp: Vec<i64> = date.split('-').filter_map(|x| x.parse().ok()).collect();
    if dp.len() != 3 || rest.len() < 8 {
        return None;
    }
    let hms: Vec<i64> = rest[..8].split(':').filter_map(|x| x.parse().ok()).collect();
    if hms.len() != 3 {
        return None;
    }
    let tz = &rest[8..];
    let mut off = 0;
    if tz.len() >= 5 && (tz.starts_with('+') || tz.starts_with('-')) {
        let v: i64 = tz[1..5].parse().unwrap_or(0);
        off = (v / 100 * 3600 + v % 100 * 60) * if tz.starts_with('-') { -1 } else { 1 };
    }
    Some(days_from_civil(dp[0], dp[1], dp[2]) * 86400 + hms[0] * 3600 + hms[1] * 60 + hms[2] - off)
}

// ---------- сеть ----------

pub struct NetInfo {
    pub id: String,
    pub label: String,
    pub online: bool,
    pub vpn: bool,
    pub dev: String,
}

/// Отпечаток сети: шлюз, его MAC и поднятые VPN/TUN. Только /proc и /sys, без запросов в интернет.
pub fn fingerprint() -> NetInfo {
    let (gw, dev) = default_route();
    let mac = arp_mac(&gw);
    let vpn = vpn_ifaces();
    let raw = format!("{dev}|{gw}|{mac}|{}", vpn.join(","));
    let id = sha1_smol::Sha1::from(raw.as_bytes()).digest().to_string()[..10].to_string();
    let mut label = if dev.is_empty() { "нет сети".to_string() } else { dev.clone() };
    if !gw.is_empty() {
        label += &format!(" через {gw}");
    }
    if !vpn.is_empty() {
        label += &format!(" + VPN {}", vpn.join(","));
    }
    NetInfo { id, label, online: !gw.is_empty() || !vpn.is_empty(), vpn: !vpn.is_empty(), dev }
}

fn default_route() -> (String, String) {
    let Ok(data) = fs::read_to_string("/proc/net/route") else { return Default::default() };
    let mut best: Option<(i64, String, String)> = None;
    for l in data.lines().skip(1) {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 8 || f[1] != "00000000" || f[7] != "00000000" {
            continue;
        }
        let metric: i64 = f[6].parse().unwrap_or(0);
        let Ok(g) = u32::from_str_radix(f[2], 16) else { continue };
        let b = g.to_le_bytes();
        let gw = format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]);
        if best.as_ref().map(|x| metric < x.0).unwrap_or(true) {
            best = Some((metric, gw, f[0].to_string()));
        }
    }
    best.map(|(_, g, d)| (g, d)).unwrap_or_default()
}

fn arp_mac(ip: &str) -> String {
    if ip.is_empty() {
        return String::new();
    }
    fs::read_to_string("/proc/net/arp")
        .unwrap_or_default()
        .lines()
        .find_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() >= 4 && f[0] == ip).then(|| f[3].to_string())
        })
        .unwrap_or_default()
}

fn vpn_ifaces() -> Vec<String> {
    let mut r = vec![];
    if let Ok(rd) = fs::read_dir("/sys/class/net") {
        for e in rd.flatten() {
            let p = e.path();
            let t = fs::read_to_string(p.join("type")).unwrap_or_default();
            let op = fs::read_to_string(p.join("operstate")).unwrap_or_default();
            let tun = p.join("tun_flags").exists();
            if (t.trim() == "65534" || tun) && op.trim() != "down" {
                r.push(e.file_name().to_string_lossy().into_owned());
            }
        }
    }
    r.sort();
    r
}

/// Лимитная сеть (NetworkManager): точка доступа телефона и т.п.
pub fn metered(dev: &str) -> bool {
    if dev.is_empty() || !have("nmcli") {
        return false;
    }
    let (s, _) = out("nmcli", &["-t", "-g", "GENERAL.METERED", "dev", "show", dev]);
    s.trim().starts_with("yes")
}

/// Работа от батареи: есть батарея, и ни один сетевой адаптер питания не подключён.
pub fn on_battery() -> bool {
    let Ok(rd) = fs::read_dir("/sys/class/power_supply") else { return false };
    let (mut battery, mut mains) = (false, false);
    for e in rd.flatten() {
        let p = e.path();
        let t = fs::read_to_string(p.join("type")).unwrap_or_default();
        match t.trim() {
            "Battery" => battery = true,
            "Mains" | "USB" => mains |= fs::read_to_string(p.join("online")).unwrap_or_default().trim() == "1",
            _ => {}
        }
    }
    battery && !mains
}

// ---------- система ----------

pub fn free_space(path: &str) -> u64 {
    let Ok(c) = std::ffi::CString::new(path) else { return 0 };
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return 0;
    }
    s.f_bavail as u64 * s.f_frsize as u64
}

fn boot_time() -> i64 {
    fs::read_to_string("/proc/stat")
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("btime ").and_then(|v| v.trim().parse().ok()))
        .unwrap_or(0)
}

fn kernel_release() -> String {
    fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_string()
}

/// Нужна перезагрузка: отметка дистрибутива, пропали модули запущенного ядра, или новое ядро после загрузки.
pub fn reboot_needed() -> bool {
    if Path::new("/run/reboot-required").exists() {
        return true;
    }
    let rel = kernel_release();
    if !rel.is_empty() && !Path::new("/usr/lib/modules").join(&rel).exists() {
        return true;
    }
    let boot = boot_time();
    fs::read_dir("/usr/lib/modules")
        .map(|rd| {
            rd.flatten().any(|e| {
                fs::metadata(e.path().join("modules.dep"))
                    .and_then(|m| m.modified())
                    .map(|t| boot > 0 && t.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) > boot + 60)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

pub fn dir_size(dirs: &[&str]) -> u64 {
    fn walk(p: &Path) -> u64 {
        let Ok(rd) = fs::read_dir(p) else { return 0 };
        rd.flatten()
            .map(|e| match e.file_type() {
                Ok(t) if t.is_dir() => walk(&e.path()),
                Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => 0,
            })
            .sum()
    }
    dirs.iter().map(|d| walk(Path::new(d))).sum()
}

pub fn find_etc(suffixes: &[&str]) -> Vec<String> {
    fn walk(p: &Path, suf: &[&str], r: &mut Vec<String>) {
        let Ok(rd) = fs::read_dir(p) else { return };
        for e in rd.flatten() {
            let path = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => walk(&path, suf, r),
                Ok(_) => {
                    let s = path.to_string_lossy();
                    if suf.iter().any(|x| s.ends_with(x)) {
                        r.push(s.into_owned());
                    }
                }
                _ => {}
            }
        }
    }
    let mut r = vec![];
    walk(Path::new("/etc"), suffixes, &mut r);
    r.sort();
    r
}

pub fn failed_units() -> usize {
    if !have("systemctl") {
        return 0;
    }
    lines(&out("systemctl", &["--failed", "--no-legend", "--plain"]).0).len()
}

pub fn unit_state(unit: &str) -> String {
    if !have("systemctl") {
        return String::new();
    }
    out("systemctl", &["is-active", unit]).0.trim().to_string()
}

pub fn systemd() -> bool {
    Path::new("/run/systemd/system").exists()
}

pub fn tail_file(path: &str, max: u64) -> Vec<String> {
    let Ok(mut f) = fs::File::open(path) else { return vec![] };
    use std::io::{Read, Seek, SeekFrom};
    if let Ok(m) = f.metadata() {
        if m.len() > max {
            let _ = f.seek(SeekFrom::End(-(max as i64)));
        }
    }
    let mut b = vec![];
    let _ = f.read_to_end(&mut b);
    String::from_utf8_lossy(&b).lines().map(String::from).collect()
}

// ---------- кому нужен перезапуск ----------

/// Процессы, которые держат удалённые (обновлённые) библиотеки или бинарники.
#[derive(Default, Clone, Debug)]
pub struct Restart {
    /// системные службы, которые можно перезапустить
    pub services: Vec<String>,
    /// системные службы, перезапуск которых оборвёт сеанс — нужна перезагрузка
    pub critical: Vec<String>,
    /// программы пользователя — перезапустить вручную или перелогиниться
    pub apps: Vec<String>,
}

const CRITICAL: &[&str] = &[
    "dbus", "dbus-broker", "systemd-logind", "gdm", "sddm", "lightdm", "display-manager", "systemd-journald", "polkit",
];

pub fn needs_restart() -> Restart {
    let mut r = Restart::default();
    let Ok(rd) = fs::read_dir("/proc") else { return r };
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(pid) = name.to_str().filter(|s| s.chars().all(|c| c.is_ascii_digit())) else { continue };
        let base = Path::new("/proc").join(pid);
        let exe_deleted = fs::read_link(base.join("exe")).map(|p| p.to_string_lossy().ends_with(" (deleted)")).unwrap_or(false);
        let libs_deleted = exe_deleted
            || fs::read_to_string(base.join("maps"))
                .map(|m| m.lines().any(|l| l.ends_with(" (deleted)") && l.contains(".so") && (l.contains(" /usr/") || l.contains(" /lib"))))
                .unwrap_or(false);
        if !libs_deleted {
            continue;
        }
        let cg = fs::read_to_string(base.join("cgroup")).unwrap_or_default();
        let path = cg.lines().find_map(|l| l.strip_prefix("0::")).unwrap_or("").to_string();
        let comm = fs::read_to_string(base.join("comm")).unwrap_or_default().trim().to_string();
        let last = path.rsplit('/').next().unwrap_or("");
        if path.starts_with("/system.slice/") && last.ends_with(".service") {
            let unit = last.to_string();
            let stem = unit.trim_end_matches(".service");
            let target = if CRITICAL.iter().any(|c| stem == *c || stem.starts_with(&format!("{c}@"))) { &mut r.critical } else { &mut r.services };
            if !target.contains(&unit) {
                target.push(unit);
            }
        } else if path.contains("/user@") || path.starts_with("/user.slice") {
            if !comm.is_empty() && !r.apps.contains(&comm) {
                r.apps.push(comm);
            }
        } else if (path == "/init.scope" || pid == "1") && !r.critical.contains(&"systemd (PID 1)".to_string()) {
            r.critical.push("systemd (PID 1)".into());
        }
    }
    r.services.sort();
    r.critical.sort();
    r.apps.sort();
    r
}

// ---------- os-release ----------

pub fn os_release() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for p in ["/etc/os-release", "/usr/lib/os-release"] {
        if let Ok(t) = fs::read_to_string(p) {
            for l in t.lines() {
                if let Some((k, v)) = l.split_once('=') {
                    m.insert(k.to_string(), v.trim_matches(|c| c == '"' || c == '\'').to_string());
                }
            }
            break;
        }
    }
    m.entry("PRETTY_NAME".into()).or_insert_with(|| "Linux".into());
    m
}
