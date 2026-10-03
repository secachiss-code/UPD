//! VPN на ядре mihomo (Clash.Meta — то же ядро, что внутри FlClash).
//! Подписки, сборка конфига, управление через REST API, обновление ядра по релизам FlClash, геофайлы.

use crate::common::*;
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::fs;
use std::io::Read;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use url::Url;

pub const SERVICE: &str = "cm-vpn.service";
pub const AUTO_GROUP: &str = "⚡ Auto";
/// Имя группы до 0.2.5: на него могут ссылаться свои правила пользователя
const AUTO_GROUP_OLD: &str = "⚡ Авто";

/// Имя группы или сервера для показа: служебная группа автовыбора — на языке интерфейса,
/// в конфиге mihomo она всегда AUTO_GROUP (на это имя опираются сохранённый выбор и правила).
pub fn label(name: &str) -> String {
    if name == AUTO_GROUP || name == AUTO_GROUP_OLD {
        t!("⚡ Авто").to_string()
    } else {
        name.to_string()
    }
}

/// Имя группы автовыбора, как его пишет пользователь в своих правилах: на любом языке интерфейса.
fn is_auto_group(s: &str) -> bool {
    s == AUTO_GROUP || s == AUTO_GROUP_OLD || crate::i18n::ALL.iter().any(|l| s == crate::i18n::tr_to(*l, "⚡ Авто"))
}
/// Порт своего DNS mihomo (listen 127.0.0.1:1053).
pub const DNS_PORT: u16 = 1053;
const TUN_DEV: &str = "cm-vpn";
pub const TEST_URL: &str = "https://www.gstatic.com/generate_204";
/// Панели подписок (Marzban, Remnawave, 3x-ui…) по User-Agent отдают конфиг для mihomo/FlClash
const UA: &str = "mihomo/1.19.32";
const SUBSCRIPTION_AGENTS: [&str; 5] = [UA, "ClashMeta/1.19.32", "Clash-Verge/2.4.2", "FlClash/0.8.92", "v2rayNG/1.8.10"];
const GEO_BASE: &str = "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest";
/// (имя файла, которое ищет mihomo в своём каталоге; имя в релизе meta-rules-dat)
const GEO: [(&str, &str); 4] = [("geoip.metadb", "geoip.metadb"), ("GeoSite.dat", "geosite.dat"), ("GeoIP.dat", "geoip.dat"), ("ASN.mmdb", "GeoLite2-ASN.mmdb")];
static PROFILE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn etc() -> String {
    env_or("CM_VPN_ETC", if Path::new("/etc/cm/vpn").exists() || !Path::new("/etc/upd/vpn").exists() { "/etc/cm/vpn" } else { "/etc/upd/vpn" })
}
pub fn home() -> String {
    env_or("CM_VPN_HOME", &format!("{}/vpn", state_dir()))
}
pub fn core_bin() -> String {
    format!("{}/bin/mihomo", home())
}
fn config_path() -> String {
    format!("{}/config.yaml", home())
}
/// API mihomo: только Unix-сокет в каталоге службы (0700); TCP-контроллер не включается.
fn api_socket() -> PathBuf {
    Path::new(&home()).join("mihomo.sock")
}

fn private_dir(p: &str) -> Result<(), String> {
    fs::create_dir_all(p).map_err(|e| format!("{p}: {e}"))?;
    fs::set_permissions(p, fs::Permissions::from_mode(0o700)).map_err(|e| format!("{p}: {e}"))
}

fn write_private(path: &str, data: &[u8]) -> Result<(), String> {
    atomic_write(Path::new(path), data, 0o600).map_err(|e| format!("{path}: {e}"))
}

// ======================= подписки =======================

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SubInfo {
    pub upload: u64,
    pub download: u64,
    pub total: u64,
    pub expire: i64,
}

impl SubInfo {
    /// Израсходовано: сумма в u128, без переполнения при любых u64.
    pub fn used(&self) -> u128 {
        u128::from(self.upload) + u128::from(self.download)
    }
    /// Израсходовано не меньше 90% лимита.
    pub fn nearly_exhausted(&self) -> bool {
        self.total > 0 && self.used() * 10 >= u128::from(self.total) * 9
    }
}

/// Объём больше u64 (сумма счётчиков) показывается как предел u64.
pub fn fmt_bytes_wide(n: u128) -> String {
    fmt_bytes(u64::try_from(n).unwrap_or(u64::MAX))
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Sub {
    pub id: String,
    pub name: String,
    /// адрес подписки — секрет: хранится только в /etc/cm/vpn (0600), наружу не выводится
    pub url: String,
    /// Некоторым серверам нужен определённый User-Agent для выдачи формата mihomo.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub user_agent: String,
    #[serde(default)]
    pub updated: i64,
    #[serde(default)]
    pub interval_h: i64,
    /// clash — готовый конфиг Clash/mihomo; uri — список ссылок vless://, ss://… (base64 или текст)
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub info: Option<SubInfo>,
    #[serde(default)]
    pub nodes: usize,
    #[serde(default)]
    pub error: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Subs {
    #[serde(default)]
    pub active: String,
    #[serde(default)]
    pub list: Vec<Sub>,
}

pub fn load_subs() -> Result<Subs, String> {
    let path = format!("{}/subs.json", etc());
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Subs::default()),
        Err(e) => return Err(format!("{path}: {e}")),
    };
    let mut subs: Subs = serde_json::from_slice(&bytes).map_err(|e| format!("{path}: {e}"))?;
    for sub in &mut subs.list {
        sub.name = sanitize_profile_name(&sub.name);
        scrub_stored_error(&mut sub.error);
    }
    Ok(subs)
}

/// Удалить из сохранённых ошибок старые сообщения, которые могли содержать URL подписки.
fn scrub_stored_error(error: &mut String) -> bool {
    if error.to_ascii_lowercase().contains("://") {
        *error = t!("ошибка запроса (URL скрыт)").into();
        true
    } else {
        false
    }
}

fn save_subs(s: &Subs) -> Result<(), String> {
    let mut safe_subs = s.clone();
    for sub in &mut safe_subs.list {
        sub.name = sanitize_profile_name(&sub.name);
        scrub_stored_error(&mut sub.error);
    }
    private_dir(&etc())?;
    write_private(&format!("{}/subs.json", etc()), &serde_json::to_vec_pretty(&safe_subs).unwrap_or_default())?;
    publish_state(&safe_subs);
    Ok(())
}

/// Адрес без секретов: схема и хост.
pub fn mask_url(u: &str) -> String {
    let (scheme, rest) = u.split_once("://").unwrap_or(("", u));
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    format!("{scheme}://{host}/…")
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = vec![];
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => return None,
        } as u32;
        buf = buf << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(hi), Some(lo)) = (hex_digit(b[i + 1]), hex_digit(b[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn sanitize_profile_name(name: &str) -> String {
    name.chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(
                    *c,
                    '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
                )
        })
        .take(128)
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn pct_encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

#[cfg(test)]
fn agent(timeout: u64, via_proxy: Option<(u16, bool)>) -> ureq::Agent {
    agent_with_redirects(timeout, via_proxy, 5)
}

fn agent_with_redirects(timeout: u64, via_proxy: Option<(u16, bool)>, redirects: u32) -> ureq::Agent {
    // timeout_read: если загрузка встала (сервер молчит 30 с), не ждать общий таймаут
    let mut b = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .timeout(Duration::from_secs(timeout))
        .redirects(redirects)
        .user_agent(UA);
    if let Some((p, ipv6)) = via_proxy {
        let host = if ipv6 { "[::1]" } else { "127.0.0.1" };
        if let Ok(px) = ureq::Proxy::new(format!("http://{host}:{p}")) {
            b = b.proxy(px);
        }
    }
    b.build()
}

fn safe_ureq_error(error: &ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(transport) => match transport.kind() {
            ureq::ErrorKind::InvalidUrl => t!("неверный URL").into(),
            ureq::ErrorKind::UnknownScheme => t!("неподдерживаемая схема URL").into(),
            ureq::ErrorKind::Dns => t!("ошибка DNS").into(),
            ureq::ErrorKind::InsecureRequestHttpsOnly => t!("небезопасная HTTP схема").into(),
            ureq::ErrorKind::ConnectionFailed => t!("ошибка соединения").into(),
            ureq::ErrorKind::TooManyRedirects => t!("слишком много перенаправлений").into(),
            ureq::ErrorKind::BadStatus => t!("неверный HTTP ответ").into(),
            ureq::ErrorKind::BadHeader => t!("неверный HTTP заголовок").into(),
            ureq::ErrorKind::Io => t!("ошибка ввода-вывода").into(),
            ureq::ErrorKind::InvalidProxyUrl => t!("неверный адрес прокси").into(),
            ureq::ErrorKind::ProxyConnect => t!("ошибка соединения с прокси").into(),
            ureq::ErrorKind::ProxyUnauthorized => t!("ошибка авторизации прокси").into(),
            ureq::ErrorKind::HTTP => t!("ошибка HTTP").into(),
        },
    }
}

/// GET напрямую; если не вышло — через наш VPN (если работает), затем через прокси FlClash (если запущен).
fn get(url: &str, timeout: u64, port: u16) -> Result<ureq::Response, String> {
    get_with_redirects(url, timeout, port, 5)
}

fn get_with_redirects(url: &str, timeout: u64, port: u16, redirects: u32) -> Result<ureq::Response, String> {
    get_with_redirects_user_agent(url, timeout, port, redirects, UA)
}

fn get_with_redirects_user_agent(url: &str, timeout: u64, port: u16, redirects: u32, user_agent: &str) -> Result<ureq::Response, String> {
    let mut errs = match agent_with_redirects(timeout, None, redirects).get(url).set("User-Agent", user_agent).call() {
        Ok(r) => return Ok(r),
        Err(e) => vec![t!("напрямую: {}", safe_ureq_error(&e))],
    };
    let mut via: Vec<((u16, bool), &str)> = vec![];
    if running() {
        via.push(((port, false), t!("через VPN")));
    }
    via.extend(flclash_ports().into_iter().map(|p| (p, t!("через FlClash"))));
    for ((p, ipv6), what) in via {
        match agent_with_redirects(timeout, Some((p, ipv6)), redirects).get(url).set("User-Agent", user_agent).call() {
            Ok(r) => return Ok(r),
            Err(e) => errs.push(format!("{what} :{p}: {}", safe_ureq_error(&e))),
        }
    }
    Err(errs.join("; "))
}

fn subscription_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| t!("некорректный URL подписки").to_string())?;
    if url.scheme() != "https" {
        return Err(t!("URL подписки должен использовать HTTPS; HTTP не поддерживается").into());
    }
    if url.host_str().filter(|host| !host.is_empty()).is_none() {
        return Err(t!("у URL подписки отсутствует host").into());
    }
    Ok(url)
}

fn get_subscription(raw: &str, timeout: u64, port: u16, user_agent: &str) -> Result<ureq::Response, String> {
    const MAX_REDIRECTS: u32 = 5;
    let mut current = subscription_url(raw)?;
    for redirects in 0..=MAX_REDIRECTS {
        let response = get_with_redirects_user_agent(current.as_str(), timeout, port, 0, user_agent)?;
        if !matches!(response.status(), 301 | 302 | 303 | 307 | 308) {
            if (300..400).contains(&response.status()) {
                return Err(t!("HTTP {} вместо профиля подписки", response.status()));
            }
            return Ok(response);
        }
        if redirects == MAX_REDIRECTS {
            return Err(t!("слишком много перенаправлений подписки").into());
        }
        let location = response.header("location").ok_or(t!("в перенаправлении нет Location"))?;
        current = subscription_url(current.join(location).map_err(|_| t!("неверный адрес перенаправления подписки"))?.as_str())?;
    }
    unreachable!()
}

/// IPv6 адрес в формате /proc/net/tcp6: каждое 32-битное слово выведено в native endian.
fn proc_ipv6_addr(raw: &str) -> Option<std::net::Ipv6Addr> {
    if raw.len() != 32 {
        return None;
    }
    let mut octets = [0u8; 16];
    for (i, word) in raw.as_bytes().chunks_exact(8).enumerate() {
        let word = std::str::from_utf8(word).ok()?;
        let bytes = u32::from_str_radix(word, 16).ok()?.to_ne_bytes();
        octets[i * 4..i * 4 + 4].copy_from_slice(&bytes);
    }
    Some(std::net::Ipv6Addr::from(octets))
}

fn flclash_listen_ports(tcp: &str, tcp6: &str, inodes: &std::collections::HashSet<String>) -> Vec<(u16, bool)> {
    let mut ports = vec![];
    for line in tcp.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        // локальный адрес 127.0.0.1 или 0.0.0.0, состояние 0A — LISTEN
        let Some((addr, port)) = f.get(1).and_then(|a| a.split_once(':')) else { continue };
        if f.get(3) != Some(&"0A") || !["0100007F", "00000000"].contains(&addr) || !f.get(9).map(|i| inodes.contains(*i)).unwrap_or(false) {
            continue;
        }
        if let Ok(p) = u16::from_str_radix(port, 16) {
            if p != 9090 && !ports.iter().any(|(seen, _)| *seen == p) {
                ports.push((p, false));
            }
        }
    }
    for line in tcp6.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        let Some((addr, port)) = f.get(1).and_then(|a| a.split_once(':')) else { continue };
        let Some(addr) = proc_ipv6_addr(addr) else { continue };
        if f.get(3) != Some(&"0A")
            || !(addr.is_loopback() || addr.is_unspecified())
            || !f.get(9).map(|i| inodes.contains(*i)).unwrap_or(false)
        {
            continue;
        }
        if let Ok(p) = u16::from_str_radix(port, 16) {
            if p != 9090 && !ports.iter().any(|(seen, _)| *seen == p) {
                ports.push((p, true));
            }
        }
    }
    ports
}

/// Реальный uid процесса из /proc/PID/status.
fn proc_uid(pid_dir: &Path) -> Option<u32> {
    fs::read_to_string(pid_dir.join("status")).ok()?.lines().find_map(|l| l.strip_prefix("Uid:")?.split_whitespace().next()?.parse().ok())
}

/// Чьим прокси можно доверить адрес подписки: root и пользователь, запустивший cm.
fn trusted_uids() -> Vec<u32> {
    let mut uids = vec![0, unsafe { libc::getuid() }];
    for k in ["SUDO_UID", "PKEXEC_UID"] {
        if let Some(uid) = std::env::var(k).ok().and_then(|v| v.parse().ok()) {
            uids.push(uid);
        }
    }
    if let Some(user) = invoking_user() {
        if let Ok(uid) = out("id", &["-u", &user]).0.trim().parse() {
            uids.push(uid);
        }
    }
    uids
}

/// Локальные mixed-порты FlClashCore; bool указывает, что listener доступен по IPv6.
/// Имя процесса задаёт он сам, поэтому слушатель принимается только от процесса root или пользователя cm.
fn flclash_ports() -> Vec<(u16, bool)> {
    let mut inodes = std::collections::HashSet::new();
    let trusted = trusted_uids();
    for e in fs::read_dir("/proc").into_iter().flatten().flatten() {
        if fs::read_to_string(e.path().join("comm")).unwrap_or_default().trim() != "FlClashCore" {
            continue;
        }
        if !proc_uid(&e.path()).map(|uid| trusted.contains(&uid)).unwrap_or(false) {
            continue;
        }
        for fd in fs::read_dir(e.path().join("fd")).into_iter().flatten().flatten() {
            let l = fs::read_link(fd.path()).map(|l| l.to_string_lossy().into_owned()).unwrap_or_default();
            if let Some(i) = l.strip_prefix("socket:[").and_then(|x| x.strip_suffix(']')) {
                inodes.insert(i.to_string());
            }
        }
    }
    if inodes.is_empty() {
        return vec![];
    }
    let tcp = fs::read_to_string("/proc/net/tcp").unwrap_or_default();
    let tcp6 = fs::read_to_string("/proc/net/tcp6").unwrap_or_default();
    flclash_listen_ports(&tcp, &tcp6, &inodes)
}

/// Как read_limited, но пишет в лог прогресс каждые 10% (для больших файлов).
fn read_progress(r: ureq::Response, max: u64, log: Log) -> Result<Vec<u8>, String> {
    let total: u64 = r.header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    if total > max {
        return Err(t!("ответ превышает лимит {}", fmt_bytes(max)));
    }
    let mut rd = r.into_reader().take(max.saturating_add(1));
    let (mut b, mut buf, mut step) = (vec![], [0u8; 64 << 10], 1u64);
    loop {
        let n = rd.read(&mut buf).map_err(|e| t!("загрузка прервалась на {}: {1}", fmt_bytes(b.len() as u64), e))?;
        if n == 0 {
            return Ok(b);
        }
        if (b.len() + n) as u64 > max {
            return Err(t!("ответ превышает лимит {}", fmt_bytes(max)));
        }
        b.extend_from_slice(&buf[..n]);
        if total > 0 && b.len() as u64 * 10 >= total * step {
            log(&t!("  {}% ({} из {})", step * 10, fmt_bytes(b.len() as u64), fmt_bytes(total)));
            step += 1;
        }
    }
}

/// Счётчик из заголовка: десятичное целое. Целое в записи с плавающей точкой принимается, пока оно точное (< 2^53);
/// отрицательное, дробное и слишком большое значение — «неизвестно» (0), а не насыщение до u64::MAX.
fn userinfo_num(v: &str) -> Option<u64> {
    let v = v.trim();
    if let Ok(n) = v.parse::<u64>() {
        return Some(n);
    }
    let f: f64 = v.parse().ok()?;
    (f.is_finite() && f >= 0.0 && f.fract() == 0.0 && f < 9_007_199_254_740_992.0).then_some(f as u64)
}

fn parse_userinfo(h: &str) -> SubInfo {
    let mut i = SubInfo::default();
    for part in h.split(';') {
        let Some((k, v)) = part.trim().split_once('=') else { continue };
        let n = userinfo_num(v).unwrap_or(0);
        match k.trim() {
            "upload" => i.upload = n,
            "download" => i.download = n,
            "total" => i.total = n,
            "expire" => i.expire = i64::try_from(n).unwrap_or(0),
            _ => {}
        }
    }
    i
}

/// Only retry successful HTTP responses with unusable bodies. Transport/auth errors
/// stop immediately; never send the private URL to a conversion service.
fn negotiate_subscription<T>(preferred: &str, mut request: impl FnMut(&str) -> Result<(String, T), String>) -> Result<(String, (T, String, usize)), String> {
    let mut agents = Vec::new();
    if !preferred.is_empty() {
        validate_user_agent(preferred)?;
        agents.push(preferred);
    }
    for agent in SUBSCRIPTION_AGENTS {
        if !agents.contains(&agent) { agents.push(agent); }
    }
    let mut errors = Vec::new();
    for agent in agents {
        let (body, metadata) = request(agent)?;
        match classify_sub(&body) {
            Ok((kind, nodes)) => return Ok((body, (metadata, kind, nodes))),
            Err(error) => errors.push(format!("{agent}: {error}")),
        }
    }
    Err(errors.join("; "))
}

/// Скачать подписку, понять формат, сохранить профиль. Меняет sub на месте.
fn fetch_sub(sub: &mut Sub, c: &Config) -> Result<ProfileChange, String> {
    let previous_kind = sub.kind.clone();
    let (body, (accepted, kind, nodes)) = negotiate_subscription(&sub.user_agent, |user_agent| {
        let r = get_subscription(&sub.url, 15, c.vpn_port, user_agent)?;
        let mut candidate = sub.clone();
    if let Some(h) = r.header("subscription-userinfo") {
        candidate.info = Some(parse_userinfo(h));
    }
    if let Some(h) = r.header("profile-update-interval").and_then(|v| v.trim().parse::<i64>().ok()) {
        candidate.interval_h = h.clamp(1, MAX_HOURS);
    }
    candidate.name = sanitize_profile_name(&candidate.name);
    if candidate.name.is_empty() {
        const MAX_TITLE_HEADER: usize = 4096;
        const MAX_DISPOSITION_HEADER: usize = 8192;
        let title = r.header("profile-title").filter(|t| t.len() <= MAX_TITLE_HEADER).map(|t| match t.strip_prefix("base64:") {
            Some(b) => b64_decode(b).map(|v| String::from_utf8_lossy(&v).into_owned()).unwrap_or_default(),
            None => t.to_string(),
        }).map(|name| sanitize_profile_name(&name)).filter(|name| !name.is_empty());
        let file = r.header("content-disposition").filter(|d| d.len() <= MAX_DISPOSITION_HEADER).and_then(|d| {
            d.split(';').find_map(|p| {
                let p = p.trim();
                p.strip_prefix("filename*=UTF-8''").map(pct_decode).or_else(|| p.strip_prefix("filename=").map(|x| x.trim_matches('"').to_string()))
            })
        }).map(|name| sanitize_profile_name(&name)).filter(|name| !name.is_empty());
        candidate.name = title.or(file).unwrap_or_else(|| sanitize_profile_name(&host_of(&mask_url(&candidate.url))));
    }
        let body = String::from_utf8_lossy(&read_limited(r, 32 << 20)?).into_owned();
        Ok((body, candidate))
    })?;
    private_dir(&format!("{}/profiles", home()))?;
    let target = PathBuf::from(profile_path(sub, &kind));
    let temp = stage_profile(&target, &body, &kind, nodes)?;
    let old_extension = match previous_kind.as_str() {
        "uri" => Some(PathBuf::from(profile_path(sub, "uri"))),
        "clash" => Some(PathBuf::from(profile_path(sub, "clash"))),
        _ => None,
    };
    let change = match install_profile(&temp, &target, old_extension) {
        Ok(change) => change,
        Err(e) => {
            let _ = remove_profile_file(&temp);
            return Err(e);
        }
    };
    *sub = accepted;
    sub.kind = kind;
    sub.nodes = nodes;
    sub.updated = now();
    sub.error.clear();
    Ok(change)
}

fn profile_path(sub: &Sub, kind: &str) -> String {
    format!("{}/profiles/{}.{}", home(), sub.id, if kind == "uri" { "txt" } else { "yaml" })
}

struct ProfileChange {
    target: PathBuf,
    previous_target: Option<PathBuf>,
    old_extension: Option<PathBuf>,
}

fn sync_profile_dir(path: &Path) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| t!("{}: нет каталога профилей", path.display()))?;
    fs::File::open(parent).and_then(|dir| dir.sync_all()).map_err(|e| format!("{}: {e}", parent.display()))
}

fn remove_profile_file(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => sync_profile_dir(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn stage_profile(path: &Path, body: &str, kind: &str, nodes: usize) -> Result<PathBuf, String> {
    let dir = path.parent().ok_or_else(|| t!("{}: нет каталога профилей", path.display()))?;
    for _ in 0..8 {
        let n = PROFILE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = dir.join(format!(".cm-profile-{}-{n}.tmp", std::process::id()));
        let mut file = match fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temp) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let result = (|| -> Result<(), String> {
            use std::io::Write;
            file.write_all(body.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
            file.sync_all().map_err(|e| format!("{}: {e}", path.display()))?;
            drop(file);
            let staged = fs::read_to_string(&temp).map_err(|e| format!("{}: {e}", path.display()))?;
            let (staged_kind, staged_nodes) = classify_sub(&staged)?;
            if staged_kind != kind || staged_nodes != nodes {
                return Err(t!("временный профиль отличается от проверенного ответа подписки").into());
            }
            sync_profile_dir(path)
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
        return Ok(temp);
    }
    Err(t!("{}: не удалось создать временный профиль", path.display()))
}

fn install_profile(temp: &Path, target: &Path, old_extension: Option<PathBuf>) -> Result<ProfileChange, String> {
    let previous_target = match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_file() => {
            let parent = target.parent().ok_or_else(|| t!("{}: нет каталога профилей", target.display()))?;
            let mut backup = None;
            // Hard link keeps the old bytes available for rollback without another data write.
            for _ in 0..8 {
                let n = PROFILE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
                let candidate = parent.join(format!(".cm-profile-backup-{}-{n}.tmp", std::process::id()));
                match fs::hard_link(target, &candidate) {
                    Ok(()) => {
                        if let Err(e) = sync_profile_dir(target) {
                            let _ = fs::remove_file(&candidate);
                            return Err(e);
                        }
                        backup = Some(candidate);
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(t!("{}: не удалось сохранить прежний профиль: {1}", target.display(), e)),
                }
            }
            Some(backup.ok_or_else(|| t!("{}: не удалось создать резервную ссылку профиля", target.display()))?)
        }
        Ok(_) => return Err(t!("{}: ожидался обычный файл профиля", target.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{}: {e}", target.display())),
    };
    if let Err(e) = fs::rename(temp, target) {
        if let Some(backup) = &previous_target {
            let _ = remove_profile_file(backup);
        }
        let _ = remove_profile_file(temp);
        return Err(format!("{}: {e}", target.display()));
    }
    if let Err(e) = sync_profile_dir(target) {
        let rollback = match &previous_target {
            Some(backup) => fs::rename(backup, target).map_err(|rollback| t!("{}: {2}; прежний профиль сохранён в {}", target.display(), backup.display(), rollback)),
            None => remove_profile_file(target),
        };
        return match rollback {
            Ok(()) => Err(e),
            Err(rollback) => Err(t!("{0}; не удалось вернуть прежний профиль: {1}", e, rollback)),
        };
    }
    Ok(ProfileChange { target: target.to_path_buf(), previous_target, old_extension })
}

fn rollback_profile_change(change: &ProfileChange) -> Result<(), String> {
    match &change.previous_target {
        Some(backup) => {
            fs::rename(backup, &change.target).map_err(|e| t!("{}: {2}; прежний профиль сохранён в {}", change.target.display(), backup.display(), e))?;
            sync_profile_dir(&change.target)
        }
        None => remove_profile_file(&change.target),
    }
}

fn finish_profile_change(change: ProfileChange, log: Log) {
    if let Some(old) = &change.old_extension {
        if old != &change.target {
            if let Err(e) = remove_profile_file(old) {
                log(&t!("VPN: старый профиль оставлен для очистки: {0}", e));
            }
        }
    }
    if let Some(backup) = &change.previous_target {
        if let Err(e) = remove_profile_file(backup) {
            log(&t!("VPN: резервный профиль оставлен для очистки: {0}", e));
        }
    }
}

fn classify_sub(body: &str) -> Result<(String, usize), String> {
    if let Ok(Value::Mapping(m)) = parse_profile(body) {
        check_subscription_placeholder(&m)?;
        let proxies = m.get("proxies").and_then(Value::as_sequence).map(|s| s.len()).unwrap_or(0);
        if proxies > 0 || m.contains_key("proxy-providers") {
            return Ok(("clash".into(), proxies));
        }
    }
    let count = |t: &str| t.lines().filter(|line| {
        line.trim().split_once("://").is_some_and(|(scheme, address)| {
            !address.is_empty() && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        })
    }).count();
    let n = count(body);
    if n > 0 {
        return Ok(("uri".into(), n));
    }
    if let Some(dec) = b64_decode(body.trim()) {
        let n = count(&String::from_utf8_lossy(&dec));
        if n > 0 {
            return Ok(("uri".into(), n));
        }
    }
    Err(t!("ответ не похож на подписку (нет ни конфига Clash, ни ссылок vless://, ss://…)").into())
}

pub fn add_sub(url: &str, name: &str, c: &Config, log: Log) -> Result<(), String> {
    let _vpn_files = vpn_config_lock(true)?;
    let _lock = subscriptions_lock(true)?;
    subscription_url(url)?;
    let mut subs = load_subs()?;
    if subs.list.iter().any(|s| s.url == url) {
        return Err(t!("такая подписка уже есть").into());
    }
    let id = format!("{:x}", now()) + &sha1_smol::Sha1::from(url).digest().to_string()[..6];
    let mut sub = Sub { id: id.clone(), name: sanitize_profile_name(name), url: url.to_string(), interval_h: c.vpn_sub_update_h, ..Default::default() };
    log(&t!("скачиваю подписку {}...", mask_url(url)));
    let change = fetch_sub(&mut sub, c)?;
    log(&t!("«{}»: серверов {}, формат {}", sub.name, sub.nodes, sub.kind));
    subs.list.push(sub);
    if subs.active.is_empty() || !subs.list.iter().any(|s| s.id == subs.active) {
        subs.active = id;
    }
    if let Err(e) = save_subs(&subs) {
        return match rollback_profile_change(&change) {
            Ok(()) => Err(e),
            Err(rollback) => Err(t!("{0}; профиль не удалось вернуть: {1}", e, rollback)),
        };
    }
    finish_profile_change(change, log);
    Ok(())
}

/// Какая подписка: номер из `cm vpn subs` (для разовой команды) или неизменный id (из TUI).
/// Номер и id разрешаются в запись только под блокировкой подписок.
#[derive(Clone, Debug, PartialEq)]
pub enum SubRef {
    Index(usize),
    Id(String),
}

impl SubRef {
    /// «3» — третья по списку; «id:…» — запись с этим id.
    pub fn parse(s: &str) -> Option<SubRef> {
        match s.strip_prefix("id:") {
            Some(id) if !id.is_empty() => Some(SubRef::Id(id.to_string())),
            Some(_) => None,
            None => s.parse::<usize>().ok().filter(|n| *n > 0).map(|n| SubRef::Index(n - 1)),
        }
    }

    fn resolve(&self, subs: &Subs) -> Result<usize, String> {
        match self {
            SubRef::Index(i) if *i < subs.list.len() => Ok(*i),
            SubRef::Id(id) => subs.list.iter().position(|s| &s.id == id).ok_or_else(|| t!("подписки уже нет — список изменился").into()),
            _ => Err(t!("нет такой подписки").into()),
        }
    }
}

fn validate_user_agent(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 512 || !value.bytes().all(|b| (32..=126).contains(&b)) {
        return Err(t!("неверный User-Agent: нужна строка ASCII до 512 байт без управляющих символов").into());
    }
    Ok(())
}

pub fn set_user_agent(reference: &SubRef, user_agent: &str) -> Result<(), String> {
    validate_user_agent(user_agent)?;
    let _vpn_files = vpn_config_lock(true)?;
    let _subs_lock = subscriptions_lock(true)?;
    let mut subs = load_subs()?;
    let index = reference.resolve(&subs)?;
    subs.list[index].user_agent = user_agent.to_owned();
    save_subs(&subs)
}

pub fn delete_sub(r: &SubRef) -> Result<String, String> {
    let _vpn_files = vpn_config_lock(true)?;
    let _lock = subscriptions_lock(true)?;
    let mut subs = load_subs()?;
    let idx = r.resolve(&subs)?;
    let s = subs.list.remove(idx);
    if subs.active == s.id {
        subs.active = subs.list.first().map(|x| x.id.clone()).unwrap_or_default();
    }
    save_subs(&subs)?;
    for ext in ["yaml", "txt"] {
        let _ = remove_profile_file(Path::new(&format!("{}/profiles/{}.{ext}", home(), s.id)));
    }
    Ok(s.name)
}

/// Выбор фиксируется только после проверки и успешного применения конфига.
pub fn use_sub(r: &SubRef, c: &Config) -> Result<String, String> {
    let subs = load_subs()?;
    let selected = &subs.list[r.resolve(&subs)?];
    let invalid = fs::read_to_string(profile_path(selected, &selected.kind))
        .map(|body| classify_sub(&body).is_err()).unwrap_or(true);
    let reference = SubRef::Id(selected.id.clone());
    if invalid { repair_profile(&reference, c)?; }
    switch_subscription(&reference, c, service_active(), &mut || reload())
}

fn repair_profile(reference: &SubRef, c: &Config) -> Result<(), String> {
    let _vpn_files = vpn_config_lock(true)?;
    let _lock = subscriptions_lock(true)?;
    let mut subs = load_subs()?;
    let index = reference.resolve(&subs)?;
    let change = fetch_sub(&mut subs.list[index], c)?;
    if let Err(error) = save_subs(&subs) {
        rollback_profile_change(&change)?;
        return Err(error);
    }
    finish_profile_change(change, &|_| {});
    Ok(())
}

fn switch_subscription(r: &SubRef, c: &Config, active: bool, reload_core: &mut impl FnMut() -> Result<(), String>) -> Result<String, String> {
    switch_subscription_with(r, c, active, reload_core, &mut save_subs)
}

fn switch_subscription_with(r: &SubRef, c: &Config, active: bool, reload_core: &mut impl FnMut() -> Result<(), String>, commit: &mut impl FnMut(&Subs) -> Result<(), String>) -> Result<String, String> {
    let _vpn_files = vpn_config_lock(true)?;
    let latest = if Path::new(&conf_path()).exists() { Config::load(c.mirrors.clone())? } else { c.clone() };
    let _lock = subscriptions_lock(true)?;
    let mut subs = load_subs()?;
    let previous_subs = subs.clone();
    let selected = subs.list[r.resolve(&subs)?].clone();
    let mut stage = "build";
    let result = (|| {
        subs.active = selected.id.clone();
        let candidate = build_config_for(&latest, &subs)?;
        stage = "validate";
        validate_candidate(&candidate)?;
        let previous = match fs::read(config_path()) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !active => None,
            Err(e) => return Err(format!("{}: {e}", config_path())),
        };
        stage = "write";
        let applied = write_private(&config_path(), candidate.as_bytes()).and_then(|_| {
            stage = "apply";
            if active { reload_core() } else { Ok(()) }
        });
        let saving = applied.is_ok();
        if saving { stage = "save"; }
        let committed = applied.and_then(|_| commit(&subs));
        if let Err(error) = committed {
            let mut rollback_errors = vec![];
            let restored = match previous {
                Some(bytes) => write_private(&config_path(), &bytes),
                None => match fs::remove_file(config_path()) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(e) => Err(e.to_string()),
                },
            };
            match restored {
                Ok(()) if active => if let Err(error) = reload_core() { rollback_errors.push(error); },
                Ok(()) => {},
                Err(error) => rollback_errors.push(error),
            }
            if saving {
                // Atomic rename can succeed before directory fsync fails.
                if let Err(error) = commit(&previous_subs) { rollback_errors.push(error); }
            }
            return Err(if rollback_errors.is_empty() {
                format!("{error}; previous VPN configuration restored")
            } else { format!("{error}; VPN rollback failed: {}", rollback_errors.join("; ")) });
        }
        resolve_failure_locked();
        Ok(selected.name.clone())
    })();
    if let Err(error) = &result { store_failure_locked(stage, Some(&selected), error); }
    result
}

struct CandidateConfig(PathBuf);
impl Drop for CandidateConfig {
    fn drop(&mut self) { let _ = fs::remove_file(&self.0); }
}
fn validate_candidate(config: &str) -> Result<(), String> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    private_dir(&home())?;
    let path = Path::new(&home()).join(format!(".candidate-{}-{}.yaml", std::process::id(), SEQUENCE.fetch_add(1, Ordering::Relaxed)));
    let candidate = CandidateConfig(path);
    write_private(candidate.0.to_str().ok_or("invalid VPN path")?, config.as_bytes())?;
    let mut policy = CapturePolicy::background(Some(64 << 10));
    policy.stderr_max = 64 << 10;
    let output = capture_with_policy(std::process::Command::new(core_bin()).args(["-t", "-d", &home(), "-f", candidate.0.to_str().ok_or("invalid VPN path")?]).env("LC_ALL", "C"), policy).map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut reason = core_validation_reason(&stderr, &stdout).to_owned();
        if let Ok(profile) = parse_profile(config) { redact_credentials(&profile, &mut reason); }
        scrub_stored_error(&mut reason);
        return Err(t!("mihomo не принял конфиг: {}", reason));
    }
    Ok(())
}

fn core_validation_reason<'a>(stderr: &'a str, stdout: &'a str) -> &'a str {
    let meaningful = |line: &&str| {
        let lower = line.to_ascii_lowercase();
        !line.trim().is_empty() && !(lower.contains("configuration file") && lower.contains("test failed"))
    };
    let error = |line: &&str| meaningful(line) && {
        let lower = line.to_ascii_lowercase();
        lower.contains("error") || lower.contains("fatal")
    };
    stderr.lines().rev().find(error).or_else(|| stdout.lines().rev().find(error))
        .or_else(|| stderr.lines().rev().find(meaningful)).or_else(|| stdout.lines().rev().find(meaningful))
        .map(str::trim).unwrap_or("validation failed")
}

#[derive(Debug, Serialize)]
pub struct ProfileCheck {
    pub name: String,
    pub active: bool,
    pub error: Option<String>,
}

/// Проверка всех сохранённых профилей без изменения выбора, конфига и работающего ядра.
pub fn doctor(c: &Config, reference: Option<&SubRef>) -> Result<Vec<ProfileCheck>, String> {
    let _vpn_files = vpn_config_lock(true)?;
    let _subs_lock = subscriptions_lock(true)?;
    let subs = load_subs()?;
    let selected = reference.map(|r| r.resolve(&subs)).transpose()?;
    let mut checks = vec![];
    for (i, sub) in subs.list.iter().enumerate() {
        if selected.is_some_and(|n| n != i) { continue; }
        let mut candidate = subs.clone();
        candidate.active = sub.id.clone();
        let result = build_config_for(c, &candidate).and_then(|config| validate_candidate(&config));
        checks.push(ProfileCheck { name: sanitize_profile_name(&sub.name), active: subs.active == sub.id, error: result.err().map(|e| safe_diagnostic(&e, Some(sub))) });
    }
    Ok(checks)
}

/// Обновить подписки: все (force) или те, у которых подошёл срок. true — активная изменилась.
pub fn update_subs(c: &Config, log: Log, force: bool) -> Result<bool, String> {
    let _vpn_files = vpn_config_lock(true)?;
    let _lock = subscriptions_lock(true)?;
    update_subs_locked(c, log, force)
}

fn update_subs_locked(c: &Config, log: Log, force: bool) -> Result<bool, String> {
    let mut subs = load_subs()?;
    let mut active_changed = false;
    let mut profile_changes = Vec::new();
    for s in subs.list.iter_mut() {
        let interval = if s.interval_h > 0 { s.interval_h } else { c.vpn_sub_update_h };
        if !force && !elapsed_at_least(s.updated, hours_secs(interval)) {
            continue;
        }
        let previous = s.clone();
        match fetch_sub(s, c) {
            Ok(change) => {
                profile_changes.push(change);
                log(&t!("подписка «{}»: обновлена, серверов {}", s.name, s.nodes));
                active_changed |= s.id == subs.active;
            }
            Err(e) => {
                *s = previous;
                s.error = e.clone();
                log(&t!("подписка «{}»: {1}", s.name, e));
            }
        }
    }
    if let Err(e) = save_subs(&subs) {
        let mut rollback_errors = Vec::new();
        for change in profile_changes.iter().rev() {
            if let Err(error) = rollback_profile_change(change) {
                rollback_errors.push(error);
            }
        }
        if rollback_errors.is_empty() {
            return Err(e);
        }
        return Err(t!("{1}; не удалось вернуть прежние профили: {}", rollback_errors.join("; "), e));
    }
    for change in profile_changes {
        finish_profile_change(change, log);
    }
    Ok(active_changed)
}

// ======================= публичное состояние (без секретов) =======================

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SubPub {
    /// неизменный id записи: подтверждение в TUI адресует его, а не номер строки
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub host: String,
    pub updated: i64,
    pub nodes: usize,
    pub info: Option<SubInfo>,
    pub error: String,
    pub active: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct VpnState {
    #[serde(default)]
    pub core_version: String,
    #[serde(default)]
    pub core_latest: String,
    #[serde(default)]
    pub flclash_tag: String,
    #[serde(default)]
    pub flclash_applied: String,
    #[serde(default)]
    pub checked: i64,
    #[serde(default)]
    pub geo_updated: i64,
    #[serde(default)]
    pub subs: Vec<SubPub>,
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub event_time: i64,
    #[serde(default)]
    pub last_failure: Option<LastFailure>,
}

/// Последняя ошибка остаётся доступна после выхода команды и перезапуска helper.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LastFailure {
    pub stage: String,
    pub time: i64,
    pub subscription: String,
    pub reason: String,
    #[serde(default)]
    pub resolved: Option<i64>,
}
fn failure_path() -> PathBuf { Path::new(&home()).join("last-error.json") }
pub fn last_failure() -> Option<LastFailure> {
    fs::read(failure_path()).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok())
}
fn safe_diagnostic(error: &str, sub: Option<&Sub>) -> String {
    let mut safe = error.to_owned();
    // Hide the entire URL-bearing error before replacing a known URL: appended
    // query parameters and redirected addresses may contain different secrets.
    if scrub_stored_error(&mut safe) { return safe; }
    if let Some(sub) = sub {
        if !sub.url.is_empty() { safe = safe.replace(&sub.url, "[subscription URL hidden]"); }
        // mihomo can echo credentials from malformed node definitions.
        if let Ok(body) = fs::read_to_string(profile_path(sub, &sub.kind)) {
            match parse_profile(&body) {
                Ok(profile) => redact_credentials(&profile, &mut safe),
                Err(_) if sub.kind == "clash" => return "Profile parsing failed; see the command output".into(),
                Err(_) => {},
            }
        }
    }
    scrub_stored_error(&mut safe);
    safe.chars().filter(|c| !c.is_control() || *c == '\n').take(2000).collect()
}
fn redact_credentials(value: &Value, text: &mut String) {
    match value {
        Value::Mapping(map) => for (key, value) in map {
            let key = key.as_str().unwrap_or("").to_ascii_lowercase();
            if ["password", "uuid", "token", "private-key", "public-key", "short-id", "obfs-password", "age-secret-key", "auth", "auth-str", "header"].contains(&key.as_str()) {
                redact_strings(value, text);
            } else { redact_credentials(value, text); }
        },
        Value::Sequence(values) => for value in values { redact_credentials(value, text); },
        _ => {},
    }
}
fn redact_strings(value: &Value, text: &mut String) {
    match value {
        Value::String(secret) if !secret.is_empty() => *text = text.replace(secret, "[hidden]"),
        Value::Mapping(map) => for (_, value) in map { redact_strings(value, text); },
        Value::Sequence(values) => for value in values { redact_strings(value, text); },
        _ => {},
    }
}
fn store_failure_locked(stage: &str, sub: Option<&Sub>, reason: &str) {
    let failure = LastFailure { stage: stage.into(), time: now(), subscription: sub.map(|s| s.name.clone()).unwrap_or_default(), reason: safe_diagnostic(reason, sub), resolved: None };
    let mut state = load_state();
    state.last_failure = Some(failure.clone());
    let _ = save_json("vpn.json", &state);
    if let Err(error) = serde_json::to_vec_pretty(&failure).map_err(|e| e.to_string()).and_then(|bytes| write_private(failure_path().to_str().unwrap_or(""), &bytes)) {
        eprintln!("VPN: could not save diagnostic: {error}");
    }
}
pub fn record_failure(stage: &str, error: &str) {
    if let Ok(_lock) = vpn_config_lock(true) {
        let subs = load_subs().unwrap_or_default();
        store_failure_locked(stage, subs.list.iter().find(|s| s.id == subs.active), error);
    }
}
fn resolve_failure_locked() {
    if let Some(mut failure) = last_failure() {
        failure.resolved = Some(now());
        let mut state = load_state(); state.last_failure = Some(failure.clone());
        let _ = save_json("vpn.json", &state);
        if let Ok(bytes) = serde_json::to_vec_pretty(&failure) { let _ = write_private(failure_path().to_str().unwrap_or(""), &bytes); }
    }
}
pub fn resolve_failure() {
    if let Ok(_lock) = vpn_config_lock(true) { resolve_failure_locked(); }
}

pub fn load_state() -> VpnState {
    let mut state: VpnState = load_json("vpn.json");
    let mut changed = false;
    for sub in &mut state.subs {
        let safe_name = sanitize_profile_name(&sub.name);
        if safe_name != sub.name {
            sub.name = safe_name;
            changed = true;
        }
        changed |= scrub_stored_error(&mut sub.error);
    }
    if changed {
        let _ = save_json("vpn.json", &state);
    }
    state
}

fn publish_state(s: &Subs) {
    let mut st = load_state();
    st.subs = s
        .list
        .iter()
        .map(|x| {
            let mut error = x.error.clone();
            scrub_stored_error(&mut error);
            SubPub {
                id: x.id.clone(),
                name: sanitize_profile_name(&x.name),
                host: host_of(&mask_url(&x.url)).to_string(),
                updated: x.updated,
                nodes: x.nodes,
                info: x.info.clone(),
                error,
                active: x.id == s.active,
            }
        })
        .collect();
    let _ = save_json("vpn.json", &st);
}

// ======================= конфиг mihomo =======================

fn rules_path() -> String {
    format!("{}/rules.txt", etc())
}

/// Шаблон rules.txt на языке интерфейса (комментарии и имя группы в примере); сами правила от языка не зависят.
pub fn rules_template() -> String {
    format!(
        "{}\n{}\n{}\n# DOMAIN-SUFFIX,mirror.yandex.ru,DIRECT\n# DOMAIN-KEYWORD,torrent,DIRECT\n# GEOSITE,youtube,{}\n# IP-CIDR,10.8.0.0/16,DIRECT,no-resolve\n",
        t!("# cm VPN: свои правила — идут первыми, раньше правил подписки."),
        t!("# Формат mihomo: ТИП,значение,куда. Куда: DIRECT (напрямую), REJECT (блок) или имя группы/сервера."),
        t!("# Примеры:"),
        label(AUTO_GROUP)
    )
}

/// Нетронутый шаблон прежних версий: такой файл можно заменить шаблоном на текущем языке.
const RULES_TEMPLATE_OLD: &str = "# cm VPN: свои правила — идут первыми, раньше правил подписки.\n\
# Формат mihomo: ТИП,значение,куда. Куда: DIRECT (напрямую), REJECT (блок) или имя группы/сервера.\n\
# Примеры:\n\
# DOMAIN-SUFFIX,mirror.yandex.ru,DIRECT\n\
# DOMAIN-KEYWORD,torrent,DIRECT\n\
# GEOSITE,youtube,⚡ Авто\n\
# IP-CIDR,10.8.0.0/16,DIRECT,no-resolve\n";

/// Свои правила. Файла нет — правил нет: сборка конфига (в том числе в ExecStartPre, где /etc только для чтения)
/// ничего не пишет; шаблон создаёт редактор правил.
pub fn user_rules() -> Result<Vec<String>, String> {
    let p = rules_path();
    let text = match fs::read_to_string(&p) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("{p}: {e}")),
    };
    // группу автовыбора в правилах можно назвать на любом языке интерфейса (и прежним «⚡ Авто»):
    // в конфиг mihomo идёт её постоянное имя, иначе mihomo отверг бы правило
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split(',').map(|f| if is_auto_group(f.trim()) { AUTO_GROUP } else { f }).collect::<Vec<_>>().join(","))
        .collect())
}

pub fn edit_rules() -> Result<(), String> {
    let p = rules_path();
    // нет файла или в нём нетронутый шаблон прежней версии — пишем шаблон на текущем языке
    let untouched = fs::read_to_string(&p).map(|t| t == RULES_TEMPLATE_OLD).unwrap_or(false);
    if !Path::new(&p).exists() || untouched {
        private_dir(&etc())?;
        atomic_write(Path::new(&p), rules_template().as_bytes(), 0o600).map_err(|e| format!("{p}: {e}"))?;
    }
    user_rules()?;
    let editor = std::env::var("EDITOR").ok().filter(|e| !e.is_empty()).unwrap_or_else(|| ["nano", "micro", "vim", "vi"].into_iter().find(|e| have(e)).unwrap_or("vi").to_string());
    run(false, &[], &editor, &[&rules_path()])
}

fn k(s: &str) -> Value {
    Value::String(s.into())
}

fn seq(items: &[&str]) -> Value {
    Value::Sequence(items.iter().map(|s| k(s)).collect())
}

fn yaml_map(pairs: Vec<(&str, Value)>) -> Value {
    let mut m = Mapping::new();
    for (key, v) in pairs {
        m.insert(k(key), v);
    }
    Value::Mapping(m)
}

/// Главная группа: та, куда ведёт последнее правило MATCH; иначе первая группа-селектор.
fn main_group(groups: &[Value], rules: &[Value]) -> Option<String> {
    let names: Vec<String> = groups.iter().filter_map(|g| g.get("name")?.as_str().map(String::from)).collect();
    let by_match = rules.iter().rev().filter_map(Value::as_str).find_map(|r| r.strip_prefix("MATCH,").map(|t| t.split(',').next().unwrap_or("").trim().to_string()));
    if let Some(t) = by_match.filter(|t| names.contains(t)) {
        return Some(t);
    }
    groups.iter().find(|g| g.get("type").and_then(Value::as_str) == Some("select")).and_then(|g| g.get("name")?.as_str().map(String::from))
}

// ======================= разбор профиля с бюджетом =======================

/// Пределы разбора профиля. Сырое тело уже ограничено 32 МБ, но алиасы YAML разворачиваются при разборе:
/// узлы и байты строк считаются вместе с развёрнутыми алиасами, до построения дерева сверх бюджета.
const YAML_MAX_NODES: usize = 1_000_000;
const YAML_MAX_BYTES: usize = 64 << 20;
const YAML_MAX_DEPTH: usize = 64;
const YAML_MAX_ALIASES: usize = 10_000;

struct YamlBudget {
    nodes: std::cell::Cell<usize>,
    bytes: std::cell::Cell<usize>,
}

impl YamlBudget {
    fn take<E: serde::de::Error>(&self, bytes: usize) -> Result<(), E> {
        let nodes = self.nodes.get() + 1;
        let total = self.bytes.get().saturating_add(bytes);
        if nodes > YAML_MAX_NODES || total > YAML_MAX_BYTES {
            return Err(E::custom(t!("профиль больше допустимого после развёртывания алиасов YAML")));
        }
        self.nodes.set(nodes);
        self.bytes.set(total);
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Budgeted<'a> {
    budget: &'a YamlBudget,
    depth: usize,
}

impl<'a> Budgeted<'a> {
    fn child<E: serde::de::Error>(self) -> Result<Self, E> {
        if self.depth >= YAML_MAX_DEPTH {
            return Err(E::custom(t!("профиль YAML слишком глубокий")));
        }
        Ok(Budgeted { budget: self.budget, depth: self.depth + 1 })
    }
}

impl<'de> serde::de::DeserializeSeed<'de> for Budgeted<'_> {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> serde::de::Visitor<'de> for Budgeted<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("YAML value")
    }
    fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Value, E> {
        self.budget.take(0)?;
        Ok(Value::Bool(v))
    }
    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Value, E> {
        self.budget.take(0)?;
        Ok(Value::Number(v.into()))
    }
    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Value, E> {
        self.budget.take(0)?;
        Ok(Value::Number(v.into()))
    }
    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Value, E> {
        self.budget.take(0)?;
        Ok(Value::Number(v.into()))
    }
    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Value, E> {
        self.budget.take(v.len())?;
        Ok(Value::String(v.to_string()))
    }
    fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Value, E> {
        self.budget.take(v.len())?;
        Ok(Value::String(v))
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
        self.budget.take(0)?;
        Ok(Value::Null)
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<Value, E> {
        self.visit_unit()
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(self)
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        self.budget.take(0)?;
        let child = self.child()?;
        let mut out = vec![];
        while let Some(v) = seq.next_element_seed(child)? {
            out.push(v);
        }
        Ok(Value::Sequence(out))
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        self.budget.take(0)?;
        let child = self.child()?;
        let mut out = Mapping::new();
        while let Some(key) = map.next_key_seed(child)? {
            let value = map.next_value_seed(child)?;
            out.insert(key, value);
        }
        Ok(Value::Mapping(out))
    }
    fn visit_enum<A: serde::de::EnumAccess<'de>>(self, data: A) -> Result<Value, A::Error> {
        use serde::de::VariantAccess;
        self.budget.take(0)?;
        let (tag, variant): (String, _) = data.variant()?;
        let value = variant.newtype_variant_seed(self.child()?)?;
        Ok(Value::Tagged(Box::new(serde_yaml::value::TaggedValue { tag: serde_yaml::value::Tag::new(tag), value })))
    }
}

/// Сколько алиасов `*имя` в тексте YAML (оценка сверху: считает и похожие места внутри строк в одинарных кавычках).
fn yaml_alias_count(body: &str) -> usize {
    let mut n = 0;
    for line in body.lines() {
        let b = line.as_bytes();
        let mut quote = 0u8;
        for i in 0..b.len() {
            let c = b[i];
            if quote != 0 {
                if c == quote {
                    quote = 0;
                }
                continue;
            }
            match c {
                b'"' => quote = b'"',
                b'#' if i == 0 || b[i - 1] == b' ' => break,
                b'*' if (i == 0 || matches!(b[i - 1], b' ' | b'\t' | b'[' | b'{' | b',' | b':' | b'-'))
                    && b.get(i + 1).map(|x| x.is_ascii_alphanumeric() || *x == b'_').unwrap_or(false) =>
                {
                    n += 1
                }
                _ => {}
            }
        }
    }
    n
}

/// Разбор профиля подписки с пределами узлов, байт, глубины и числа алиасов; ключи слияния `<<` раскрываются.
fn parse_profile(body: &str) -> Result<Value, String> {
    if yaml_alias_count(body) > YAML_MAX_ALIASES {
        return Err(t!("в профиле слишком много алиасов YAML").into());
    }
    let budget = YamlBudget { nodes: std::cell::Cell::new(0), bytes: std::cell::Cell::new(0) };
    let seed = Budgeted { budget: &budget, depth: 0 };
    let mut v = serde::de::DeserializeSeed::deserialize(seed, serde_yaml::Deserializer::from_str(body)).map_err(|e| e.to_string())?;
    v.apply_merge().map_err(|e| e.to_string())?;
    Ok(v)
}

/// Что берётся из профиля подписки: узлы, группы, правила и сетевые provider-ы. Остальное (listeners, dns, hosts,
/// sniffer, authentication, skip-auth-prefixes, external-controller-*, iptables, ebpf…) задаёт cm или не задаёт никто.
const PROFILE_KEYS: [&str; 6] = ["proxies", "proxy-groups", "rules", "sub-rules", "proxy-providers", "rule-providers"];

fn check_subscription_placeholder(m: &Mapping) -> Result<(), String> {
    let nodes = m.get("proxies").and_then(Value::as_sequence);
    if !m.get("proxy-providers").and_then(Value::as_mapping).is_some_and(|p| !p.is_empty())
        && nodes.is_some_and(|nodes| !nodes.is_empty() && nodes.iter().all(|node| {
            let name = node.get("name").and_then(Value::as_str).unwrap_or("").to_lowercase();
            ["приложение не поддерживается", "unsupported client", "client not supported"].iter().any(|message| name.contains(message))
        })) {
        return Err(t!("сервер подписки вернул заглушку: приложение не поддерживается; проверьте User-Agent подписки").into());
    }
    Ok(())
}
/// Подкаталоги каталога VPN, куда provider-у можно писать кэш или откуда читать файл.
const PROVIDER_DIRS: [&str; 6] = ["profiles", "providers", "proxies", "rules", "ruleset", "rule-sets"];

/// Путь provider-а: относительный, без «..», внутри одного из PROVIDER_DIRS.
fn provider_path_ok(p: &str) -> bool {
    use std::path::Component;
    let path = Path::new(p);
    let mut parts = path.components().filter(|c| !matches!(c, Component::CurDir));
    let Some(Component::Normal(first)) = parts.next() else { return false };
    !path.is_absolute()
        && PROVIDER_DIRS.iter().any(|d| first == std::ffi::OsStr::new(d))
        && parts.clone().count() > 0
        && parts.all(|c| matches!(c, Component::Normal(_)))
}

fn relative_provider_path(path: &str) -> Option<PathBuf> {
    use std::path::Component;
    if path.chars().any(char::is_control) || path.ends_with('/') { return None; }
    let parts: Vec<_> = Path::new(path).components().filter(|p| !matches!(p, Component::CurDir)).collect();
    if parts.is_empty() || !parts.iter().all(|p| matches!(p, Component::Normal(_))) { return None; }
    Some(parts.into_iter().collect())
}

/// Каждый HTTP-provider получает свой кэш, независимо от исходного имени каталога.
fn provider_cache_path(path: &str, key: &str, identity: &str) -> Option<String> {
    let relative = relative_provider_path(path)?;
    let identity = sha1_smol::Sha1::from(identity).digest().to_string();
    Some(format!("./providers/{key}/{identity}/{}", relative.display()))
}

type ProviderRedirects = std::collections::BTreeMap<PathBuf, Option<String>>;
fn check_providers(m: &mut Mapping, key: &str, redirects: &mut ProviderRedirects) -> Result<(), String> {
    let Some(list) = m.get_mut(key) else { return Ok(()) };
    let list = list.as_mapping_mut().ok_or_else(|| t!("{0} в профиле — не список", key))?;
    for (name, p) in list {
        let name = name.as_str().unwrap_or("?");
        let kind = p.get("type").and_then(Value::as_str).unwrap_or("").to_owned();
        if !matches!(kind.as_str(), "http" | "file" | "inline") {
            return Err(t!("provider «{0}»: тип «{1}» не поддерживается", name, kind));
        }
        let format = p.get("format").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("yaml");
        let behavior = p.get("behavior").and_then(Value::as_str).unwrap_or("");
        if key == "rule-providers" && (!matches!(format, "yaml" | "text" | "mrs")
            || !matches!(behavior, "domain" | "ipcidr" | "classical")
            || (format == "mrs" && behavior == "classical" && kind != "inline")) {
            return Err(t!("provider «{0}»: неверный формат или behavior", name));
        }
        if kind == "http" {
            let url = p.get("url").and_then(Value::as_str).unwrap_or("");
            if !Url::parse(url).ok().is_some_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some()) {
                return Err(t!("provider «{0}»: неверный URL источника", name));
            }
            let path = match p.get("path") {
                Some(Value::String(path)) if !path.is_empty() => path.as_str(),
                Some(Value::String(_)) | None => if format == "mrs" { "provider.mrs" } else { "provider.yaml" },
                Some(_) => return Err(t!("provider «{0}»: путь «{1}» вне каталога профиля", name, "")),
            };
            let headers = p.get("header").map(|h| serde_yaml::to_string(h).unwrap_or_default()).unwrap_or_default();
            let identity = format!("{url}\0{name}\0{format}\0{behavior}\0{headers}");
            let cache = provider_cache_path(path, key, &identity)
                .ok_or_else(|| t!("provider «{0}»: путь «{1}» вне каталога профиля", name, path))?;
            if let Some(original) = p.get("path").and_then(Value::as_str).and_then(relative_provider_path) {
                redirects.entry(original).and_modify(|old| { if old.as_ref() != Some(&cache) { *old = None; } }).or_insert(Some(cache.clone()));
            }
            p.as_mapping_mut().unwrap().insert(k("path"), k(&cache));
        }
        // Inline-provider использует payload; path ядру не нужен.
        if kind == "inline" { p.as_mapping_mut().unwrap().remove("path"); }
    }
    Ok(())
}

fn check_file_providers(m: &mut Mapping, key: &str, redirects: &ProviderRedirects) -> Result<(), String> {
    let Some(list) = m.get_mut(key).and_then(Value::as_mapping_mut) else { return Ok(()) };
    for (name, p) in list {
        if p.get("type").and_then(Value::as_str) != Some("file") { continue; }
        let name = name.as_str().unwrap_or("?");
        let path = p.get("path").ok_or_else(|| t!("provider «{0}»: у файлового provider нет пути", name))?.as_str().unwrap_or("");
        let relative = relative_provider_path(path).ok_or_else(|| t!("provider «{0}»: путь «{1}» вне каталога профиля", name, path))?;
        if let Some(cache) = redirects.get(&relative) {
            let cache = cache.as_ref().ok_or_else(|| t!("provider «{0}»: неоднозначный общий путь кэша", name))?;
            *p.get_mut("path").unwrap() = k(cache);
            continue;
        }
        if !provider_path_ok(path) { return Err(t!("provider «{0}»: путь «{1}» вне каталога профиля", name, path)); }
        let absolute = Path::new(&home()).join(&relative);
        if !absolute.is_file() { return Err(t!("provider «{0}»: локальный файл не найден: {1}", name, path)); }
        let real = fs::canonicalize(&absolute).map_err(|e| e.to_string())?;
        let root = fs::canonicalize(home()).map_err(|e| e.to_string())?;
        if !real.starts_with(root) { return Err(t!("provider «{0}»: путь «{1}» вне каталога профиля", name, path)); }
    }
    Ok(())
}

fn check_provider_references(m: &Mapping) -> Result<(), String> {
    let rules = m.get("rule-providers").and_then(Value::as_mapping);
    let proxies = m.get("proxy-providers").and_then(Value::as_mapping);
    let check_rules = |lines: &Value| -> Result<(), String> {
        for line in lines.as_sequence().into_iter().flatten().filter_map(Value::as_str) {
            let mut fields = line.split(',').map(str::trim);
            if fields.next() == Some("RULE-SET") {
                if let Some(name) = fields.next() {
                    if !rules.is_some_and(|r| r.contains_key(name)) {
                        return Err(t!("правило ссылается на отсутствующий provider «{0}»", name));
                    }
                }
            }
        }
        Ok(())
    };
    if let Some(rules) = m.get("rules") { check_rules(rules)?; }
    for (_, rules) in m.get("sub-rules").and_then(Value::as_mapping).into_iter().flatten() { check_rules(rules)?; }
    for group in m.get("proxy-groups").and_then(Value::as_sequence).into_iter().flatten() {
        for provider in group.get("use").and_then(Value::as_sequence).into_iter().flatten().filter_map(Value::as_str) {
            if !proxies.is_some_and(|p| p.contains_key(provider)) {
                return Err(t!("группа «{0}» ссылается на отсутствующий provider «{1}»", group.get("name").and_then(Value::as_str).unwrap_or("?"), provider));
            }
        }
    }
    Ok(())
}

/// Разрешённая часть профиля подписки.
fn profile_part(src: &Mapping) -> Result<Mapping, String> {
    check_subscription_placeholder(src)?;
    let mut m = Mapping::new();
    for key in PROFILE_KEYS {
        if let Some(v) = src.get(key) {
            m.insert(k(key), v.clone());
        }
    }
    let mut redirects = ProviderRedirects::new();
    check_providers(&mut m, "proxy-providers", &mut redirects)?;
    check_providers(&mut m, "rule-providers", &mut redirects)?;
    check_file_providers(&mut m, "proxy-providers", &redirects)?;
    check_file_providers(&mut m, "rule-providers", &redirects)?;
    check_provider_references(&m)?;
    Ok(m)
}

/// Порт прокси годится для VPN: диапазон как в TUI и не порт своего DNS.
pub fn check_port(c: &Config) -> Result<(), String> {
    if !(1024..=65535).contains(&c.vpn_port) {
        return Err(t!("порт прокси {0} вне диапазона 1024–65535 (vpn_port в {1})", c.vpn_port, conf_path()));
    }
    if c.vpn_dns && c.vpn_port == DNS_PORT {
        return Err(t!("порт прокси {0} занят своим DNS VPN — выбери другой vpn_port", c.vpn_port));
    }
    Ok(())
}

/// Собирает итоговый конфиг: профиль подписки + настройки cm (порты, TUN, DNS, геофайлы, правила, авто-выбор).
pub fn build_config(c: &Config) -> Result<String, String> {
    build_config_for(c, &load_subs()?)
}

fn build_config_for(c: &Config, subs: &Subs) -> Result<String, String> {
    check_port(c)?;
    let sub = subs.list.iter().find(|s| s.id == subs.active).ok_or(t!("нет подписки: добавь её (cm → VPN → Подписки → n)"))?;
    let body = fs::read_to_string(profile_path(sub, &sub.kind)).map_err(|_| t!("профиль подписки не скачан — обнови подписку").to_string())?;

    let mut m: Mapping = if sub.kind == "clash" {
        match parse_profile(&body) {
            Ok(Value::Mapping(m)) => profile_part(&m)?,
            Ok(_) => return Err(t!("профиль подписки повреждён — обнови подписку").into()),
            Err(e) => return Err(t!("профиль подписки не принят: {0}", e)),
        }
    } else {
        let mut m = Mapping::new();
        m.insert(
            k("proxy-providers"),
            yaml_map(vec![(
                "sub",
                yaml_map(vec![
                    ("type", k("file")),
                    ("path", k(&format!("./profiles/{}.txt", sub.id))),
                    ("health-check", yaml_map(vec![("enable", Value::Bool(true)), ("url", k(TEST_URL)), ("interval", Value::from(300))])),
                ]),
            )]),
        );
        m.insert(k("proxy-groups"), Value::Sequence(vec![yaml_map(vec![("name", k("Proxy")), ("type", k("select")), ("use", seq(&["sub"]))])]));
        m.insert(k("rules"), seq(&["MATCH,Proxy"]));
        m
    };

    // --- то, что задаёт cm поверх подписки ---
    m.insert(k("mixed-port"), Value::from(c.vpn_port));
    m.insert(k("allow-lan"), Value::Bool(c.vpn_allow_lan));
    m.insert(k("bind-address"), k(if c.vpn_allow_lan { "*" } else { "127.0.0.1" }));
    // API — только Unix-сокет в каталоге 0700 (UMask службы 0077); TCP-контроллера нет
    m.insert(k("external-controller-unix"), k(&api_socket().to_string_lossy()));
    m.insert(k("mode"), k(c.vpn_mode_name()));
    m.insert(k("log-level"), k("warning"));
    m.insert(k("ipv6"), Value::Bool(c.vpn_ipv6));
    m.insert(k("unified-delay"), Value::Bool(true));
    m.insert(k("tcp-concurrent"), Value::Bool(true));
    m.insert(k("find-process-mode"), k("off")); // экономит CPU; правила по процессам не нужны
    m.insert(k("profile"), yaml_map(vec![("store-selected", Value::Bool(true)), ("store-fake-ip", Value::Bool(true))]));
    // геофайлы — тот же источник, что у FlClash
    m.insert(k("geodata-mode"), Value::Bool(false));
    // геофайлы обновляет только cm (geo_update с проверкой формата), не само ядро
    m.insert(k("geo-auto-update"), Value::Bool(false));
    m.insert(
        k("geox-url"),
        yaml_map(vec![
            ("geoip", k(&format!("{GEO_BASE}/geoip.dat"))),
            ("geosite", k(&format!("{GEO_BASE}/geosite.dat"))),
            ("mmdb", k(&format!("{GEO_BASE}/geoip.metadb"))),
            ("asn", k(&format!("{GEO_BASE}/GeoLite2-ASN.mmdb"))),
        ]),
    );
    m.insert(
        k("tun"),
        yaml_map(vec![
            ("enable", Value::Bool(c.vpn_tun)),
            ("device", k(TUN_DEV)),
            ("stack", k("mixed")),
            ("auto-route", Value::Bool(true)),
            ("auto-redirect", Value::Bool(true)),
            ("auto-detect-interface", Value::Bool(true)),
            ("strict-route", Value::Bool(false)),
            ("dns-hijack", seq(&["any:53", "tcp://any:53"])),
        ]),
    );
    if c.vpn_dns {
        m.insert(
            k("dns"),
            yaml_map(vec![
                ("enable", Value::Bool(true)),
                ("listen", k("127.0.0.1:1053")),
                ("ipv6", Value::Bool(c.vpn_ipv6)),
                ("enhanced-mode", k("fake-ip")),
                ("fake-ip-range", k("198.18.0.1/16")),
                (
                    "fake-ip-filter",
                    seq(&["*.lan", "*.local", "+.home.arpa", "localhost.*", "+.msftconnecttest.com", "+.msftncsi.com", "time.*.com", "ntp.*.com", "+.ntp.org", "+.stun.*.*", "+.stun.*.*.*", "geosite:private"]),
                ),
                ("default-nameserver", seq(&["system", "77.88.8.8", "1.1.1.1"])),
                // адреса самих VPN-серверов резолвим напрямую — до подъёма туннеля DoH может быть недоступен
                ("proxy-server-nameserver", seq(&["system", "77.88.8.8"])),
                ("nameserver", seq(&["https://1.1.1.1/dns-query", "https://dns.google/dns-query"])),
                ("direct-nameserver", seq(&["system"])),
                ("respect-rules", Value::Bool(true)),
            ]),
        );
    }

    // --- группы: «⚡ Авто» первой в главной группе ---
    let rules_now: Vec<Value> = m.get("rules").and_then(Value::as_sequence).cloned().unwrap_or_default();
    let mut groups: Vec<Value> = m.get("proxy-groups").and_then(Value::as_sequence).cloned().unwrap_or_default();
    let main = main_group(&groups, &rules_now);
    groups.retain(|g| !matches!(g.get("name").and_then(Value::as_str), Some(AUTO_GROUP | AUTO_GROUP_OLD)));
    if c.vpn_auto_select {
        // Apply to all nodes, including subscription providers. Country labels
        // come from server names; this does not alter manual selection.
        let metadata_filter = "(?i)(трафик|traffic|осталось|remain|истека|expire|срок|сайт|website|官网|剩余|到期|流量)";
        let exclude_filter = if c.vpn_auto_allow_ru { metadata_filter.to_string() } else {
            format!("{metadata_filter}|🇷🇺|(?i)(россия|россий|russia|russland|russie|俄罗斯|俄羅斯|روسيا)|(?i)(^|[^a-zа-яё])(ru|rus|рф|москва|moscow|moskva|санкт[ -]?петербург|saint[ -]?petersburg|st[ .-]?petersburg|novosibirsk)([^a-zа-яё]|$)")
        };
        groups.insert(
            0,
            yaml_map(vec![
                ("name", k(AUTO_GROUP)),
                ("type", k("url-test")),
                ("include-all", Value::Bool(true)),
                // служебные «серверы» с остатком трафика и сроком — не серверы
                ("exclude-filter", k(&exclude_filter)),
                ("url", k(TEST_URL)),
                ("interval", Value::from(300)),
                ("tolerance", Value::from(50)),
                ("timeout", Value::from(3000)),
                ("lazy", Value::Bool(false)),
            ]),
        );
        if let Some(main) = &main {
            for g in groups.iter_mut() {
                if g.get("name").and_then(Value::as_str) == Some(main.as_str()) {
                    if let Value::Mapping(gm) = g {
                        let mut list: Vec<Value> = gm.get("proxies").and_then(Value::as_sequence).cloned().unwrap_or_default();
                        list.retain(|x| !matches!(x.as_str(), Some(AUTO_GROUP | AUTO_GROUP_OLD)));
                        list.insert(0, k(AUTO_GROUP));
                        gm.insert(k("proxies"), Value::Sequence(list));
                    }
                }
            }
        }
    } else if let Some(main) = &main {
        for g in groups.iter_mut() {
            if let (Some(n), Value::Mapping(gm)) = (g.get("name").and_then(Value::as_str).map(String::from), &mut *g) {
                if n == *main {
                    if let Some(Value::Sequence(list)) = gm.get_mut("proxies") {
                        list.retain(|x| !matches!(x.as_str(), Some(AUTO_GROUP | AUTO_GROUP_OLD)));
                    }
                }
            }
        }
    }
    m.insert(k("proxy-groups"), Value::Sequence(groups));

    // --- правила: свои → локальная сеть → Россия → правила подписки ---
    let mut rules: Vec<Value> = user_rules()?.into_iter().map(Value::String).collect();
    if c.vpn_direct_lan {
        for r in ["GEOSITE,private,DIRECT", "GEOIP,private,DIRECT,no-resolve"] {
            rules.push(k(r));
        }
    }
    if c.vpn_direct_ru {
        for r in ["GEOSITE,category-ru,DIRECT", "DOMAIN-SUFFIX,ru,DIRECT", "DOMAIN-SUFFIX,xn--p1ai,DIRECT", "GEOIP,RU,DIRECT"] {
            rules.push(k(r));
        }
    }
    rules.extend(rules_now);
    if !rules.iter().any(|r| r.as_str().map(|s| s.starts_with("MATCH,")).unwrap_or(false)) {
        rules.push(k(&format!("MATCH,{}", main.unwrap_or_else(|| if c.vpn_auto_select { AUTO_GROUP.into() } else { "DIRECT".into() }))));
    }
    m.insert(k("rules"), Value::Sequence(rules));

    serde_yaml::to_string(&Value::Mapping(m)).map_err(|e| e.to_string())
}

pub fn write_config(c: &Config) -> Result<bool, String> {
    let _vpn_files = vpn_config_lock(true)?;
    let latest = if Path::new(&conf_path()).exists() { Config::load(c.mirrors.clone())? } else { c.clone() };
    write_config_locked(&latest)
}

fn write_config_locked(c: &Config) -> Result<bool, String> {
    let y = build_config(c)?;
    private_dir(&home())?;
    let p = config_path();
    if fs::read_to_string(&p).map(|o| o == y).unwrap_or(false) {
        return Ok(false);
    }
    write_private(&p, y.as_bytes())?;
    Ok(true)
}

// ======================= API mihomo =======================

/// Владелец сокета и процесса mihomo: служба работает от root; в тестах — текущий пользователь.
fn service_uid() -> u32 {
    if test_mode() { unsafe { libc::geteuid() } } else { 0 }
}

/// Сокет API — наш: сокет и его каталог принадлежат службе, каталог закрыт для группы и остальных.
/// Права самого сокета mihomo ставит 0666, поэтому доступ ограничивает каталог: без права поиска
/// в нём к сокету не подключиться, и подменить сокет может только владелец каталога.
fn check_api_socket(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let bad = || t!("{0}: сокет API mihomo чужой или доступен не только службе", path.display());
    let md = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let dir = path.parent().map(fs::symlink_metadata).ok_or_else(bad)?.map_err(|e| format!("{}: {e}", path.display()))?;
    let uid = service_uid();
    if !md.file_type().is_socket() || md.uid() != uid || !dir.is_dir() || dir.uid() != uid || dir.mode() & 0o077 != 0 {
        return Err(bad());
    }
    Ok(())
}

/// uid процесса на другом конце Unix-сокета (SO_PEERCRED).
fn peer_uid(s: &std::os::unix::net::UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let r = unsafe { libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, &mut cred as *mut _ as *mut libc::c_void, &mut len) };
    (r == 0).then_some(cred.uid)
}

/// Предел ответа API: списки прокси и соединений бывают большими, остальное — короткий JSON.
fn api_limit(path: &str) -> u64 {
    if path.starts_with("/proxies") || path.starts_with("/connections") || path.starts_with("/group") { 16 << 20 } else { 1 << 20 }
}

/// Ответ HTTP/1.1: код и тело (Content-Length, chunked или до закрытия соединения).
fn parse_http_response(raw: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let bad = || t!("неверный HTTP ответ").to_string();
    let head_end = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(bad)?;
    let head = std::str::from_utf8(&raw[..head_end]).map_err(|_| bad())?;
    let mut lines = head.split("\r\n");
    let status: u16 = lines.next().and_then(|l| l.split_whitespace().nth(1)).and_then(|c| c.parse().ok()).ok_or_else(bad)?;
    let (mut length, mut chunked) = (None, false);
    for l in lines {
        let Some((k, v)) = l.split_once(':') else { continue };
        match k.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = Some(v.trim().parse::<usize>().map_err(|_| bad())?),
            "transfer-encoding" => chunked = v.to_ascii_lowercase().contains("chunked"),
            _ => {}
        }
    }
    let body = &raw[head_end + 4..];
    if chunked {
        let mut out = vec![];
        let mut pos = 0;
        loop {
            let line_end = body[pos..].windows(2).position(|w| w == b"\r\n").ok_or_else(bad)? + pos;
            let size_str = std::str::from_utf8(&body[pos..line_end]).map_err(|_| bad())?;
            let size = usize::from_str_radix(size_str.split(';').next().unwrap_or("").trim(), 16).map_err(|_| bad())?;
            pos = line_end + 2;
            if size == 0 {
                return Ok((status, out));
            }
            let end = pos.checked_add(size).filter(|e| *e <= body.len()).ok_or_else(bad)?;
            out.extend_from_slice(&body[pos..end]);
            pos = end + 2;
        }
    }
    match length {
        Some(n) if n <= body.len() => Ok((status, body[..n].to_vec())),
        Some(_) => Err(t!("ответ обрезан относительно Content-Length").into()),
        None => Ok((status, body.to_vec())),
    }
}

/// Запрос к API mihomo через Unix-сокет. Сокет и процесс на нём проверяются до отправки запроса.
pub fn api(method: &str, path: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value, String> {
    api_with_timeout(method, path, body, Duration::from_secs(12))
}

fn api_with_timeout(method: &str, path: &str, body: Option<serde_json::Value>, request_timeout: Duration) -> Result<serde_json::Value, String> {
    use std::io::Write;
    let sock = api_socket();
    check_api_socket(&sock)?;
    let mut s = std::os::unix::net::UnixStream::connect(&sock).map_err(|e| format!("{}: {e}", sock.display()))?;
    if peer_uid(&s) != Some(service_uid()) {
        return Err(t!("{0}: на сокете API не процесс службы", sock.display()));
    }
    let timeout = Some(request_timeout);
    let _ = s.set_read_timeout(timeout);
    let _ = s.set_write_timeout(timeout);
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: mihomo\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    if !body.is_empty() {
        req += "Content-Type: application/json\r\n";
    }
    req += "\r\n";
    req += &body;
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let max = api_limit(path);
    let mut raw = vec![];
    (&mut s).take(max.saturating_add(1)).read_to_end(&mut raw).map_err(|e| e.to_string())?;
    if raw.len() as u64 > max {
        return Err(t!("ответ превышает лимит {}", fmt_bytes(max)));
    }
    let (status, body) = parse_http_response(&raw)?;
    let text = String::from_utf8_lossy(&body);
    match status {
        204 => Ok(serde_json::Value::Null),
        200..=299 if text.trim().is_empty() => Ok(serde_json::Value::Null),
        200..=299 => serde_json::from_str(&text).map_err(|e| e.to_string()),
        code => {
            let msg = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v["message"].as_str().map(String::from));
            Err(msg.unwrap_or_else(|| format!("HTTP {code}")))
        }
    }
}

pub fn running() -> bool {
    api("GET", "/version", None).is_ok()
}

pub fn reload() -> Result<(), String> {
    api("PUT", "/configs?force=true", Some(serde_json::json!({ "path": config_path() }))).map(|_| ())
}

pub fn set_mode(mode: &str) -> Result<(), String> {
    api("PATCH", "/configs", Some(serde_json::json!({ "mode": mode }))).map(|_| ())
}

pub fn select(group: &str, name: &str) -> Result<(), String> {
    api("PUT", &format!("/proxies/{}", pct_encode(group)), Some(serde_json::json!({ "name": name })))?;
    // старые соединения шли через прежний сервер — закрываем, чтобы переключение было сразу
    let _ = api("DELETE", "/connections", None);
    Ok(())
}

pub fn group_delay(group: &str) -> Result<std::collections::BTreeMap<String, u64>, String> {
    let v = api("GET", &format!("/group/{}/delay?url={}&timeout=5000", pct_encode(group), pct_encode(TEST_URL)), None)?;
    Ok(v.as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_u64()?))).collect()).unwrap_or_default())
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Group {
    pub name: String,
    pub kind: String,
    pub now: String,
    pub all: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub running: bool,
    pub mode: String,
    pub tun: bool,
    pub version: String,
    pub groups: Vec<Group>,
    /// последняя известная задержка каждого сервера, мс (0 — не отвечает)
    pub delay: std::collections::BTreeMap<String, u64>,
    pub down: u64,
    pub up: u64,
    pub conns: usize,
    /// почему ядро не ответило (пусто — ответило или сокета API нет)
    pub error: String,
}

impl Snapshot {
    /// Состояние API отдельно от последнего замера выбранного сервера.
    pub fn health_line(&self) -> String {
        if !self.running { return t!("API VPN не готов").into(); }
        let chain = self.chain();
        let Some(selected) = chain.last() else { return t!("API готов · сервер не выбран").into(); };
        match self.delay.get(selected) {
            Some(0) => t!("API готов · последний замер сервера завершился ошибкой").into(),
            Some(delay) => t!("API готов · последний замер сервера: {0} мс", delay),
            None => t!("API готов · доступность сервера ещё не проверена").into(),
        }
    }

    /// Цепочка от главной группы до реального сервера: Proxy → ⚡ Auto → 🇩🇪 DE-1
    pub fn chain(&self) -> Vec<String> {
        let mut out = vec![];
        let mut cur = self.groups.iter().find(|g| g.kind == "Selector").or(self.groups.first()).map(|g| g.name.clone());
        while let Some(n) = cur {
            if out.contains(&n) || out.len() > 6 {
                break;
            }
            out.push(n.clone());
            cur = self.groups.iter().find(|g| g.name == n).map(|g| g.now.clone()).filter(|x| !x.is_empty());
        }
        out
    }
}

pub fn snapshot() -> Snapshot {
    let mut s = Snapshot::default();
    let v = match api("GET", "/version", None) {
        Ok(v) => v,
        Err(e) => {
            if api_socket().exists() {
                s.error = e;
            }
            return s;
        }
    };
    s.running = true;
    s.version = v["version"].as_str().unwrap_or("").to_string();
    if let Ok(cfg) = api("GET", "/configs", None) {
        s.mode = cfg["mode"].as_str().unwrap_or("").to_string();
        s.tun = cfg["tun"]["enable"].as_bool().unwrap_or(false);
    }
    if let Ok(p) = api("GET", "/proxies", None) {
        if let Some(obj) = p["proxies"].as_object() {
            for (name, x) in obj {
                if let Some(d) = x["history"].as_array().and_then(|h| h.last()).and_then(|h| h["delay"].as_u64()) {
                    s.delay.insert(name.clone(), d);
                }
                let kind = x["type"].as_str().unwrap_or("");
                if ["Selector", "URLTest", "Fallback", "LoadBalance"].contains(&kind) && name != "GLOBAL" {
                    s.groups.push(Group {
                        name: name.clone(),
                        kind: kind.into(),
                        now: x["now"].as_str().unwrap_or("").into(),
                        all: x["all"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default(),
                    });
                }
            }
            // порядок групп как в конфиге: GLOBAL.all перечисляет их по порядку
            if let Some(order) = obj.get("GLOBAL").and_then(|g| g["all"].as_array()) {
                let pos = |n: &str| order.iter().position(|v| v.as_str() == Some(n)).unwrap_or(usize::MAX);
                s.groups.sort_by_key(|g| pos(&g.name));
            }
        }
    }
    if let Ok(cn) = api("GET", "/connections", None) {
        s.down = cn["downloadTotal"].as_u64().unwrap_or(0);
        s.up = cn["uploadTotal"].as_u64().unwrap_or(0);
        s.conns = cn["connections"].as_array().map(|a| a.len()).unwrap_or(0);
    }
    s
}

// ======================= служба =======================

pub fn service_active() -> bool {
    unit_state(SERVICE) == "active"
}

/// После серии неудачных запусков systemd блокирует службу на 10 минут (start-limit-hit) — снимаем блок.
fn reset_failed() {
    let _ = out("systemctl", &["reset-failed", SERVICE]);
}

pub fn start(c: &Config) -> Result<(), String> {
    let result = (|| {
        if let Some(w) = conflict(c) { return Err(w); }
        reset_failed();
        run(true, &[], "systemctl", &["start", SERVICE]).map_err(|e| format!("{e}\n{}", journal_tail()))?;
        wait_ready(Duration::from_secs(10))?;
        let _ = autostart(c.vpn_autostart);
        Ok(())
    })();
    match &result { Ok(()) => resolve_failure(), Err(error) => record_failure("start", error) }
    result
}

/// API readiness does not assert that a remote proxy server is reachable.
fn wait_ready(timeout: Duration) -> Result<(), String> {
    wait_ready_with(timeout, || api_with_timeout("GET", "/version", None, Duration::from_millis(500)).map(|_| ()))
}
fn wait_ready_with(timeout: Duration, mut probe: impl FnMut() -> Result<(), String>) -> Result<(), String> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match probe() {
            Ok(()) => return Ok(()),
            Err(error) if std::time::Instant::now() >= deadline => return Err(format!("VPN service started, but API is not ready: {error}")),
            Err(_) => std::thread::sleep(Duration::from_millis(100).min(deadline.saturating_duration_since(std::time::Instant::now()))),
        }
    }
}

pub fn stop() -> Result<(), String> {
    run(true, &[], "systemctl", &["stop", SERVICE])
}

pub fn restart(c: &Config) -> Result<(), String> {
    let result = (|| {
        if let Some(w) = conflict(c) { return Err(w); }
        reset_failed();
        run(true, &[], "systemctl", &["restart", SERVICE]).map_err(|e| format!("{e}\n{}", journal_tail()))?;
        wait_ready(Duration::from_secs(10))
    })();
    match &result { Ok(()) => resolve_failure(), Err(error) => record_failure("restart", error) }
    result
}

pub fn core_restart_failure(error: &str) -> String {
    t!("Ядро VPN обновлено на диске, но перезапуск не удался: {0}", error)
}

fn note_core_update(state: &mut VpnState, changed: bool, active: bool, restart: Result<(), String>, flclash_tag: &str) -> Option<String> {
    match restart {
        Err(error) => {
            state.event = core_restart_failure(&error);
            state.event_time = now();
            Some(state.event.clone())
        }
        Ok(()) => {
            state.flclash_applied = flclash_tag.to_string();
            if changed {
                state.event = if active {
                    t!("Ядро VPN обновлено до {} (FlClash {})", state.core_version, flclash_tag)
                } else {
                    t!("Ядро VPN обновлено на диске до {} (FlClash {}); служба VPN не запущена", state.core_version, flclash_tag)
                };
                state.event_time = now();
            }
            None
        }
    }
}

/// Последние строки журнала службы — чтобы причину сбоя было видно сразу, без journalctl.
pub fn journal_tail() -> String {
    let (s, _) = out("journalctl", &["-u", SERVICE, "-n", "8", "--no-pager", "-o", "cat"]);
    s.lines().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n")
}

pub fn autostart(on: bool) -> Result<(), String> {
    run(true, &[], "systemctl", &[if on { "enable" } else { "disable" }, SERVICE])
}

/// Ядра VPN, чей TUN мешает нашему (два авто-маршрута не уживутся). Имя здесь не даёт обойти проверку:
/// создать TUN может только привилегированный процесс; mesh-сети вроде tailscale ядрами VPN не считаются.
const VPN_CORES: [&str; 7] = ["FlClashCore", "mihomo", "clash", "clash-meta", "verge-mihomo", "sing-box", "xray"];

struct ProcInfo {
    pid: String,
    comm: String,
    ours: bool,
    sockets: Vec<String>,
    tuns: Vec<String>,
}

/// Процессы с их сокетами и TUN-интерфейсами (/proc/PID/fd и fdinfo); ours — наше ядро mihomo.
fn processes() -> Vec<ProcInfo> {
    let core = fs::canonicalize(core_bin()).ok();
    let mut list = vec![];
    for e in fs::read_dir("/proc").into_iter().flatten().flatten() {
        let name = e.file_name();
        let Some(pid) = name.to_str().filter(|p| p.bytes().all(|b| b.is_ascii_digit())) else { continue };
        let dir = e.path();
        let ours = core.is_some() && fs::read_link(dir.join("exe")).ok() == core;
        let (mut sockets, mut tuns) = (vec![], vec![]);
        for fd in fs::read_dir(dir.join("fd")).into_iter().flatten().flatten() {
            let Ok(target) = fs::read_link(fd.path()) else { continue };
            let target = target.to_string_lossy();
            if let Some(i) = target.strip_prefix("socket:[").and_then(|x| x.strip_suffix(']')) {
                sockets.push(i.to_string());
            } else if target == "/dev/net/tun" {
                let info = fs::read_to_string(dir.join("fdinfo").join(fd.file_name())).unwrap_or_default();
                if let Some(iff) = info.lines().find_map(|l| l.strip_prefix("iff:")) {
                    tuns.push(iff.trim().to_string());
                }
            }
        }
        if sockets.is_empty() && tuns.is_empty() {
            continue;
        }
        let comm = fs::read_to_string(dir.join("comm")).unwrap_or_default().trim().to_string();
        list.push(ProcInfo { pid: pid.to_string(), comm, ours, sockets, tuns });
    }
    list
}

/// inode сокетов, слушающих порт: TCP в состоянии LISTEN, UDP без соединения; любой адрес.
fn listening_inodes(table: &str, port: u16, udp: bool) -> Vec<String> {
    let want = if udp { "07" } else { "0A" };
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let p = f.get(1)?.rsplit_once(':')?.1;
            (u16::from_str_radix(p, 16).ok()? == port && f.get(3) == Some(&want)).then(|| f.get(9).map(|s| s.to_string()))?
        })
        .collect()
}

/// Конфликт из снимка /proc: чужой слушатель на порту прокси/DNS или TUN другого ядра VPN.
fn find_conflict(c: &Config, procs: &[ProcInfo], tables: &[(&str, bool)]) -> Option<String> {
    let mut ports = vec![c.vpn_port];
    if c.vpn_dns {
        ports.push(DNS_PORT);
    }
    for port in ports {
        for (table, udp) in tables {
            for inode in listening_inodes(table, port, *udp) {
                match procs.iter().find(|p| p.sockets.contains(&inode)) {
                    Some(p) if p.ours => {}
                    Some(p) if p.comm == "FlClashCore" => return Some(flclash_message()),
                    Some(p) => return Some(t!("порт {0} занят: {1} (PID {2}) — освободи его или смени vpn_port", port, p.comm, p.pid)),
                    None => return Some(t!("порт {0} занят другим процессом — освободи его или смени vpn_port", port)),
                }
            }
        }
    }
    if c.vpn_tun {
        for p in procs.iter().filter(|p| !p.ours && VPN_CORES.contains(&p.comm.as_str())) {
            if let Some(dev) = p.tuns.iter().find(|d| d.as_str() != TUN_DEV) {
                if p.comm == "FlClashCore" {
                    return Some(flclash_message());
                }
                return Some(t!("работает другой VPN с TUN {0}: {1} (PID {2}) — выключи его, иначе два VPN помешают друг другу", dev, p.comm, p.pid));
            }
        }
    }
    None
}

fn flclash_message() -> String {
    t!("запущен FlClash — закрой его (и выключи его автозапуск), иначе два VPN помешают друг другу").into()
}

/// Реальный конфликт перед запуском: чужой TUN другого ядра VPN или занятый порт прокси либо DNS.
/// Процесс без TUN и без такого порта запуску не мешает, как бы он ни назывался.
pub fn conflict(c: &Config) -> Option<String> {
    let procs = processes();
    let read = |p: &str| fs::read_to_string(p).unwrap_or_default();
    let (tcp, tcp6, udp, udp6) = (read("/proc/net/tcp"), read("/proc/net/tcp6"), read("/proc/net/udp"), read("/proc/net/udp6"));
    find_conflict(c, &procs, &[(&tcp, false), (&tcp6, false), (&udp, true), (&udp6, true)])
}

/// Применить изменения настроек: пересобрать конфиг и перезагрузить работающее ядро.
pub fn apply(c: &Config, user: Option<&UserContext>, log: Log) -> Result<(), String> {
    let (changed, latest) = {
        let _vpn_files = vpn_config_lock(true)?;
        let latest = if Path::new(&conf_path()).exists() { Config::load(c.mirrors.clone())? } else { c.clone() };
        (write_config_locked(&latest)?, latest)
    };
    let c = &latest;
    if !service_active() {
        log(t!("конфиг собран; VPN не запущен"));
        sysproxy(c, user).map_err(|error| format!("VPN configuration saved, but user proxy failed: {error}; retry: cm vpn restart"))?;
        return Ok(());
    }
    if changed {
        // TUN включается/выключается надёжнее перезапуском, остальное — горячей перезагрузкой
        let tun_now = snapshot().tun;
        if tun_now != c.vpn_tun {
            restart(c)?;
            log(t!("VPN перезапущен"));
        } else {
            reload()?;
            log(t!("конфиг перезагружен"));
        }
    } else {
        log(t!("конфиг не изменился"));
    }
    sysproxy(c, user).map_err(|error| format!("VPN core applied, but user proxy failed: {error}; retry: cm vpn restart"))?;
    Ok(())
}

/// Configuration is already persisted; absent subscriptions are an explicit pending state.
pub fn apply_saved(c: &Config, user: Option<&UserContext>, log: Log) -> Result<(), String> {
    if load_subs()?.list.is_empty() {
        log("settings saved; VPN pending until a subscription is added");
        return Ok(());
    }
    let result = apply(c, user, log).map_err(|error| format!("settings saved, but not applied: {error}; retry: cm vpn restart"));
    match &result { Ok(()) => resolve_failure(), Err(error) => record_failure("settings", error) }
    result
}

pub fn apply_saved_mode(c: &Config, user: Option<&UserContext>, log: Log) -> Result<(), String> {
    let _vpn_files = vpn_config_lock(true)?;
    if load_subs()?.list.is_empty() { log("settings saved; VPN pending until a subscription is added"); return Ok(()); }
    let latest = if Path::new(&conf_path()).exists() { Config::load(c.mirrors.clone())? } else { c.clone() };
    write_config_locked(&latest).and_then(|_| if running() { set_mode(latest.vpn_mode_name()) } else { Ok(()) })
        .and_then(|_| sysproxy(&latest, user).map(|_| ()))
        .map_err(|error| format!("settings saved, but not applied: {error}; retry: cm vpn restart"))
}

/// Режим «только прокси»: включить системный прокси GNOME пользователю; в TUN — выключить.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SysproxyStatus { BackgroundSkipped, Applied { uid: u32, enabled: bool, port: u16 } }

pub fn sysproxy(c: &Config, user: Option<&UserContext>) -> Result<SysproxyStatus, String> {
    let Some(user) = user else { return Ok(SysproxyStatus::BackgroundSkipped); };
    if !have("gsettings") { return Err(format!("UID {}: gsettings is unavailable", user.uid)); }
    let runtime = user.runtime_dir.to_string_lossy().into_owned();
    let bus = format!("unix:path={runtime}/bus");
    let on = !c.vpn_tun && service_active();
    let mut cmds: Vec<Vec<String>> = vec![vec!["org.gnome.system.proxy".into(), "mode".into(), if on { "manual" } else { "none" }.into()]];
    if on {
        for kind in ["http", "https", "socks"] {
            let schema = format!("org.gnome.system.proxy.{kind}");
            cmds.push(vec![schema.clone(), "host".into(), "127.0.0.1".into()]);
            cmds.push(vec![schema, "port".into(), c.vpn_port.to_string()]);
        }
        cmds.push(vec!["org.gnome.system.proxy".into(), "ignore-hosts".into(), "['localhost', '127.0.0.0/8', '::1', '192.168.0.0/16', '10.0.0.0/8', '172.16.0.0/12']".into()]);
    }
    for a in cmds {
        let result = if unsafe { libc::geteuid() } == user.uid {
            let mut args = vec!["set"]; args.extend(a.iter().map(String::as_str));
            run(true, &[("XDG_RUNTIME_DIR", &runtime), ("DBUS_SESSION_BUS_ADDRESS", &bus)], "gsettings", &args)
        } else {
            let runtime_env = format!("XDG_RUNTIME_DIR={runtime}");
            let bus_env = format!("DBUS_SESSION_BUS_ADDRESS={bus}");
            let mut args = vec!["-u", user.name.as_str(), "--", "env", &runtime_env, &bus_env, "gsettings", "set"];
            args.extend(a.iter().map(String::as_str));
            run(true, &[], "runuser", &args)
        };
        result.map_err(|error| format!("UID {} ({}): {error}", user.uid, user.name))?;
    }
    Ok(SysproxyStatus::Applied { uid: user.uid, enabled: on, port: c.vpn_port })
}

// ======================= ядро: mihomo, обновление по релизам FlClash =======================

fn cpu_level() -> &'static str {
    let info = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let flags: Vec<&str> = info.lines().find(|l| l.starts_with("flags")).map(|l| l.split_whitespace().collect()).unwrap_or_default();
    let has = |f: &str| flags.contains(&f);
    if has("avx2") && has("bmi2") && has("fma") && has("movbe") {
        "v3"
    } else if has("sse4_2") && has("popcnt") && has("ssse3") {
        "v2"
    } else {
        "v1"
    }
}

pub fn core_version() -> Option<String> {
    let (s, code) = out(&core_bin(), &["-v"]);
    if code != 0 {
        return None;
    }
    s.split_whitespace().find(|w| w.starts_with('v') && w[1..].starts_with(|c: char| c.is_ascii_digit())).map(String::from)
}

fn gh_latest(repo: &str, port: u16) -> Result<serde_json::Value, String> {
    let r = get(&format!("https://api.github.com/repos/{repo}/releases/latest"), 20, port)?;
    serde_json::from_str(&read_text(r, 4 << 20)?).map_err(|e| e.to_string())
}

/// Предел скачанного архива ядра и отдельно — распакованного бинарника (mihomo — десятки мегабайт).
const CORE_GZ_MAX: u64 = 64 << 20;
const CORE_BIN_MAX: u64 = 128 << 20;

/// Распаковка gzip не больше `max` байт: лишний байт сверх предела — ошибка до записи на диск.
fn gunzip_limited(gz: &[u8], max: u64) -> Result<Vec<u8>, String> {
    let mut bin = vec![];
    flate2::read::GzDecoder::new(gz).take(max.saturating_add(1)).read_to_end(&mut bin).map_err(|e| t!("распаковка: {0}", e))?;
    if bin.len() as u64 > max {
        return Err(t!("распакованное ядро больше {0} — установка отменена, прежнее ядро оставлено", fmt_bytes(max)));
    }
    Ok(bin)
}

/// Сигнал от FlClash: номер его последнего релиза; и актуальный релиз mihomo.
pub fn core_check(c: &Config, log: Log) -> Result<VpnState, String> {
    let fl = gh_latest("chen08209/FlClash", c.vpn_port)?;
    let mh = gh_latest("MetaCubeX/mihomo", c.vpn_port)?;
    let mut st = load_state();
    st.flclash_tag = fl["tag_name"].as_str().unwrap_or("").into();
    st.core_latest = mh["tag_name"].as_str().unwrap_or("").into();
    st.core_version = core_version().unwrap_or_default();
    st.checked = now();
    let _ = save_json("vpn.json", &st);
    log(&t!("FlClash: {} · mihomo: установлен {}, последний {}", st.flclash_tag, if st.core_version.is_empty() { "—" } else { &st.core_version }, st.core_latest));
    Ok(st)
}

/// Проверка SHA-256 digest релиза mihomo до распаковки и запуска кандидата.
fn verify_core_gz_digest(raw_digest: &str, gz: &[u8]) -> Result<(), String> {
    let digest = raw_digest.strip_prefix("sha256:").ok_or(t!("неверный формат SHA-256 digest; установка отменена"))?;
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(t!("неверный формат SHA-256 digest; установка отменена").into());
    }
    let digest = digest.to_ascii_lowercase();
    use sha2::Digest;
    let got: String = sha2::Sha256::digest(gz).iter().map(|b| format!("{b:02x}")).collect();
    if got != digest {
        return Err(t!("контрольная сумма не совпала (ожидалась {0}, получена {1})", digest, got));
    }
    Ok(())
}

/// Скачать и поставить mihomo (сборка под процессор, проверка SHA-256). true — ядро сменилось.
pub fn core_install(c: &Config, log: Log, force: bool) -> Result<bool, String> {
    let _vpn_files = vpn_config_lock(true)?;
    let rel = gh_latest("MetaCubeX/mihomo", c.vpn_port)?;
    let tag = rel["tag_name"].as_str().ok_or(t!("нет tag_name в релизе mihomo"))?.to_string();
    let cur = core_version();
    if !force && cur.as_deref() == Some(tag.as_str()) {
        log(&t!("ядро mihomo {0} уже актуально", tag));
        return Ok(false);
    }
    let name = if cfg!(target_arch = "aarch64") { format!("mihomo-linux-arm64-{tag}.gz") } else { format!("mihomo-linux-amd64-{}-{tag}.gz", cpu_level()) };
    let asset = rel["assets"].as_array().and_then(|a| a.iter().find(|x| x["name"] == name.as_str())).ok_or(t!("в релизе нет {0}", name))?;
    let raw_digest = asset["digest"].as_str().ok_or(t!("в релизе нет SHA-256 digest; установка отменена"))?;
    let url = asset["browser_download_url"].as_str().ok_or(t!("нет ссылки на файл"))?;
    log(&t!("скачиваю {0}...", name));
    let gz = read_progress(get(url, 600, c.vpn_port)?, CORE_GZ_MAX, log)?;
    verify_core_gz_digest(raw_digest, &gz)?;
    log(t!("контрольная сумма SHA-256 совпала"));
    let bin = gunzip_limited(&gz, CORE_BIN_MAX)?;
    private_dir(&format!("{}/bin", home()))?;
    let tmp = format!("{}.new", core_bin());
    atomic_write(Path::new(&tmp), &bin, 0o755).map_err(|e| e.to_string())?;
    let (ver, code) = out(&tmp, &["-v"]);
    if code != 0 {
        let _ = fs::remove_file(&tmp);
        return Err(t!("новое ядро не запускается — оставляю прежнее").into());
    }
    fs::rename(&tmp, core_bin()).map_err(|e| e.to_string())?;
    log(&t!("ядро: {}", ver.lines().next().unwrap_or("").trim()));
    let mut st = load_state();
    st.core_version = tag.clone();
    st.core_latest = tag;
    let _ = save_json("vpn.json", &st);
    Ok(true)
}

// ======================= геофайлы =======================

pub fn geo_files() -> Vec<(String, i64, u64)> {
    GEO.iter()
        .map(|(f, _)| {
            let m = fs::metadata(format!("{}/{f}", home())).ok();
            let t = m.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
            (f.to_string(), t, m.map(|m| m.len()).unwrap_or(0))
        })
        .collect()
}

struct ProtoField<'a> {
    number: u32,
    wire: u8,
    bytes: &'a [u8],
    integer: Option<u64>,
}

fn proto_varint(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in 0..10 {
        let byte = *data.get(*pos)?;
        *pos += 1;
        if shift == 9 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 0x7f) << (shift * 7);
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn next_proto_field<'a>(data: &'a [u8], pos: &mut usize) -> Result<Option<ProtoField<'a>>, String> {
    if *pos == data.len() {
        return Ok(None);
    }
    let key = proto_varint(data, pos).ok_or(t!("неверная структура protobuf"))?;
    let number = key >> 3;
    let wire = (key & 7) as u8;
    if number == 0 || number >= (1 << 29) {
        return Err(t!("неверная структура protobuf").into());
    }
    let (bytes, integer) = match wire {
        0 => (&[][..], Some(proto_varint(data, pos).ok_or(t!("неверная структура protobuf"))?)),
        1 => {
            let end = pos.checked_add(8).filter(|end| *end <= data.len()).ok_or(t!("обрезанная структура protobuf"))?;
            let bytes = &data[*pos..end];
            *pos = end;
            (bytes, None)
        }
        2 => {
            let len: usize = proto_varint(data, pos).and_then(|n| n.try_into().ok()).ok_or(t!("неверная длина protobuf"))?;
            let end = pos.checked_add(len).filter(|end| *end <= data.len()).ok_or(t!("обрезанная структура protobuf"))?;
            let bytes = &data[*pos..end];
            *pos = end;
            (bytes, None)
        }
        5 => {
            let end = pos.checked_add(4).filter(|end| *end <= data.len()).ok_or(t!("обрезанная структура protobuf"))?;
            let bytes = &data[*pos..end];
            *pos = end;
            (bytes, None)
        }
        _ => return Err(t!("неподдерживаемая структура protobuf").into()),
    };
    Ok(Some(ProtoField { number: number as u32, wire, bytes, integer }))
}

fn validate_geo_domain(data: &[u8]) -> Result<(), String> {
    let mut pos = 0;
    let mut value = false;
    while let Some(field) = next_proto_field(data, &mut pos)? {
        if field.number == 2 {
            if field.wire != 2 || field.bytes.is_empty() || std::str::from_utf8(field.bytes).is_err() {
                return Err(t!("неверный домен в GeoSite").into());
            }
            value = true;
        }
    }
    if value { Ok(()) } else { Err(t!("пустой домен в GeoSite").into()) }
}

fn validate_geo_cidr(data: &[u8]) -> Result<(), String> {
    let mut pos = 0;
    let mut ip = None;
    let mut prefix = None;
    while let Some(field) = next_proto_field(data, &mut pos)? {
        match field.number {
            1 => {
                if field.wire != 2 || !matches!(field.bytes.len(), 4 | 16) {
                    return Err(t!("неверный адрес в GeoIP").into());
                }
                ip = Some(field.bytes.len());
            }
            2 => {
                if field.wire != 0 {
                    return Err(t!("неверная маска в GeoIP").into());
                }
                prefix = field.integer;
            }
            _ => {}
        }
    }
    let Some(ip_len) = ip else { return Err(t!("в GeoIP нет адреса").into()) };
    if prefix.map(|n| n > (ip_len * 8) as u64).unwrap_or(false) {
        return Err(t!("неверная маска в GeoIP").into());
    }
    Ok(())
}

fn validate_geo_dat(data: &[u8], site: bool) -> Result<(), String> {
    let mut pos = 0;
    let mut entries = 0;
    while let Some(field) = next_proto_field(data, &mut pos)? {
        if field.number != 1 {
            continue;
        }
        if field.wire != 2 {
            return Err(t!("неверная запись GeoIP/GeoSite").into());
        }
        let mut entry_pos = 0;
        let mut country = false;
        let mut values = 0;
        while let Some(entry) = next_proto_field(field.bytes, &mut entry_pos)? {
            match entry.number {
                1 => {
                    if entry.wire != 2 || entry.bytes.is_empty() || std::str::from_utf8(entry.bytes).is_err() {
                        return Err(t!("неверный код GeoIP/GeoSite").into());
                    }
                    country = true;
                }
                2 => {
                    if entry.wire != 2 {
                        return Err(t!("неверная запись GeoIP/GeoSite").into());
                    }
                    if site {
                        validate_geo_domain(entry.bytes)?;
                    } else {
                        validate_geo_cidr(entry.bytes)?;
                    }
                    values += 1;
                }
                _ => {}
            }
        }
        if !country || values == 0 {
            return Err(t!("неполная запись GeoIP/GeoSite").into());
        }
        entries += 1;
    }
    if entries == 0 {
        return Err(t!("в GeoIP/GeoSite нет записей").into());
    }
    Ok(())
}

fn mmdb_size(data: &[u8], pos: &mut usize, code: u8) -> Result<usize, String> {
    let (extra, base) = match code {
        0..=28 => return Ok(code as usize),
        29 => (1, 29usize),
        30 => (2, 285usize),
        31 => (3, 65_821usize),
        _ => return Err(t!("неверный размер поля MaxMind DB").into()),
    };
    let end = pos.checked_add(extra).filter(|end| *end <= data.len()).ok_or(t!("обрезанные метаданные MaxMind DB"))?;
    let mut value = 0usize;
    for byte in &data[*pos..end] {
        value = (value << 8) | *byte as usize;
    }
    *pos = end;
    base.checked_add(value).ok_or_else(|| t!("неверный размер поля MaxMind DB").into())
}

fn mmdb_skip_value(data: &[u8], pos: &mut usize, depth: usize) -> Result<u8, String> {
    if depth > 32 {
        return Err(t!("слишком глубокие метаданные MaxMind DB").into());
    }
    let control = *data.get(*pos).ok_or(t!("обрезанные метаданные MaxMind DB"))?;
    *pos += 1;
    let mut kind = control >> 5;
    let size_code = control & 0x1f;
    if kind == 0 {
        kind = data.get(*pos).copied().and_then(|n| n.checked_add(7)).ok_or(t!("обрезанные метаданные MaxMind DB"))?;
        *pos += 1;
        if !(8..=15).contains(&kind) {
            return Err(t!("неизвестный тип метаданных MaxMind DB").into());
        }
    }
    if kind == 1 {
        let width = ((size_code >> 3) + 1) as usize;
        let end = pos.checked_add(width).filter(|end| *end <= data.len()).ok_or(t!("обрезанный указатель MaxMind DB"))?;
        *pos = end;
        return Ok(kind);
    }
    let size = mmdb_size(data, pos, size_code)?;
    match kind {
        2 => {
            let end = pos.checked_add(size).filter(|end| *end <= data.len()).ok_or(t!("обрезанная строка MaxMind DB"))?;
            std::str::from_utf8(&data[*pos..end]).map_err(|_| t!("неверная строка метаданных MaxMind DB"))?;
            *pos = end;
        }
        3 | 15 => {
            let expected = if kind == 3 { 8 } else { 4 };
            if size != expected {
                return Err(t!("неверный размер числа MaxMind DB").into());
            }
            let end = pos.checked_add(size).filter(|end| *end <= data.len()).ok_or(t!("обрезанное число MaxMind DB"))?;
            *pos = end;
        }
        4 => {
            let end = pos.checked_add(size).filter(|end| *end <= data.len()).ok_or(t!("обрезанные байты MaxMind DB"))?;
            *pos = end;
        }
        5 | 6 | 8 | 9 | 10 => {
            let max = match kind { 5 => 2, 6 | 8 => 4, 9 => 8, _ => 16 };
            if size > max {
                return Err(t!("неверный размер целого MaxMind DB").into());
            }
            let end = pos.checked_add(size).filter(|end| *end <= data.len()).ok_or(t!("обрезанное целое MaxMind DB"))?;
            *pos = end;
        }
        7 => {
            if size > data.len().saturating_sub(*pos) / 2 {
                return Err(t!("обрезанная карта метаданных MaxMind DB").into());
            }
            for _ in 0..size {
                if mmdb_skip_value(data, pos, depth + 1)? != 2 {
                    return Err(t!("ключ метаданных MaxMind DB не является строкой").into());
                }
                mmdb_skip_value(data, pos, depth + 1)?;
            }
        }
        11 => {
            if size > data.len().saturating_sub(*pos) {
                return Err(t!("обрезанный список метаданных MaxMind DB").into());
            }
            for _ in 0..size {
                mmdb_skip_value(data, pos, depth + 1)?;
            }
        }
        14 if size <= 1 => {}
        12 | 13 | 14 => return Err(t!("неподдерживаемый тип метаданных MaxMind DB").into()),
        _ => return Err(t!("неизвестный тип метаданных MaxMind DB").into()),
    }
    Ok(kind)
}

fn validate_mmdb(data: &[u8]) -> Result<(), String> {
    const MARKER: &[u8] = b"\xab\xcd\xefMaxMind.com";
    let start = data.len().saturating_sub((128 << 10) + MARKER.len());
    let Some(relative) = data[start..].windows(MARKER.len()).rposition(|w| w == MARKER) else {
        return Err(t!("нет сигнатуры MaxMind DB").into());
    };
    let metadata = &data[start + relative + MARKER.len()..];
    if metadata.is_empty() || metadata[0] >> 5 != 7 || metadata[0] & 0x1f == 0 {
        return Err(t!("повреждённый раздел метаданных MaxMind DB").into());
    }
    let mut pos = 0;
    if mmdb_skip_value(metadata, &mut pos, 0)? != 7 || pos != metadata.len() {
        return Err(t!("повреждённый раздел метаданных MaxMind DB").into());
    }
    Ok(())
}

fn save_geo_result(path: &Path, remote: &str, fetched: Result<Vec<u8>, String>) -> Result<(), String> {
    let data = fetched?;
    validate_geo_file(remote, &data)?;
    atomic_write(path, &data, 0o644).map_err(|e| e.to_string())
}

fn validate_geo_file(remote: &str, data: &[u8]) -> Result<(), String> {
    if data.len() <= 1024 {
        return Err(t!("файл слишком мал").into());
    }
    match remote {
        "geoip.metadb" | "ASN.mmdb" => validate_mmdb(data),
        "GeoSite.dat" => validate_geo_dat(data, true),
        "GeoIP.dat" => validate_geo_dat(data, false),
        _ => Err(t!("неизвестный формат геофайла").into()),
    }
}

pub fn geo_update(c: &Config, log: Log, only_missing: bool) -> Result<(), String> {
    let _vpn_files = vpn_config_lock(true)?;
    private_dir(&home())?;
    let mut errs = vec![];
    for (file, remote) in GEO {
        let path = format!("{}/{file}", home());
        if only_missing && Path::new(&path).exists() {
            continue;
        }
        log(&t!("геофайл {0}...", remote));
        match get(&format!("{GEO_BASE}/{remote}"), 300, c.vpn_port)
            .and_then(|r| {
                if r.status() != 200 {
                    return Err(t!("HTTP {} вместо полного файла", r.status()));
                }
                read_limited(r, 100 << 20)
            })
            .and_then(|b| validate_geo_file(remote, &b).map(|()| b))
        {
            Ok(b) => {
                if let Err(e) = save_geo_result(Path::new(&path), remote, Ok(b)) {
                    errs.push(format!("{remote}: {e}"));
                }
            }
            Err(e) => errs.push(format!("{remote}: {e}")),
        }
    }
    if errs.is_empty() {
        let mut st = load_state();
        st.geo_updated = now();
        let _ = save_json("vpn.json", &st);
        if service_active() && !only_missing {
            let _ = reload();
        }
        Ok(())
    } else {
        Err(errs.join("; "))
    }
}

// ======================= подготовка к запуску и обслуживание =======================

/// Скачать недостающее: ядро и геофайлы. Работает и при запущенном FlClash — тогда качает через его прокси.
pub fn fetch_missing(c: &Config, log: Log) -> Result<(), String> {
    if core_version().is_none() {
        log(t!("ядра ещё нет — ставлю mihomo"));
        core_install(c, log, true)?;
    }
    if let Err(e) = geo_update(c, log, true) {
        log(&t!("геофайлы не скачались ({0}) — mihomo попробует сам", e));
    }
    Ok(())
}

/// Вызывается службой перед стартом ядра (ExecStartPre): только проверка конфликта, сборка конфига и `mihomo -t`.
/// Загрузки здесь нет — медленная сеть не должна упираться в TimeoutStartSec; недостающее качает `cm vpn start`.
pub fn prepare(c: &Config, log: Log) -> Result<(), String> {
    let result = prepare_inner(c, log);
    if let Err(error) = &result { record_failure("prepare", error); }
    result
}

fn prepare_inner(c: &Config, log: Log) -> Result<(), String> {
    check_port(c)?;
    if let Some(w) = conflict(c) {
        return Err(w);
    }
    if core_version().is_none() {
        return Err(t!("ядра mihomo нет — выполни: sudo cm vpn core update").into());
    }
    let _vpn_files = vpn_config_lock(true)?;
    let latest = if Path::new(&conf_path()).exists() { Config::load(c.mirrors.clone())? } else { c.clone() };
    let candidate = build_config(&latest)?;
    validate_candidate(&candidate)?;
    write_private(&config_path(), candidate.as_bytes())?;
    log(t!("конфиг проверен"));
    Ok(())
}

/// Фоновое обслуживание (из cm auto): подписки по сроку, ядро по сигналу FlClash.
pub fn maintain(c: &Config, log: Log) {
    let changed = {
        let _vpn_files = match vpn_config_lock(true) { Ok(lock) => lock, Err(error) => { log(&error); return; } };
        let _lock = match subscriptions_lock(true) {
            Ok(lock) => lock,
            Err(error) => { log(&error); return; }
        };
        let subs = match load_subs() { Ok(subs) => subs, Err(error) => { log(&error); return; } };
        if subs.list.is_empty() { return; }
        match update_subs_locked(c, log, false) {
            Ok(changed) => changed,
            Err(error) => { log(&t!("VPN: подписки не обновлены: {0}", error)); false }
        }
    };
    if changed { if let Err(error) = apply(c, None, log) { log(&format!("VPN: {error}")); } }
    let st = load_state();
    if !elapsed_at_least(st.checked, hours_secs(c.vpn_core_check_h)) && core_version().is_some() {
        return;
    }
    let Ok(st) = core_check(c, log) else { return };
    let signal = st.flclash_tag != st.flclash_applied;
    if signal || core_version().is_none() {
        log(&t!("новый релиз FlClash {} — обновляю ядро mihomo", st.flclash_tag));
        match core_install(c, log, false) {
            Ok(changed) => {
                let active = service_active();
                let restart_result = if changed && active { restart(c) } else { Ok(()) };
                let mut s2 = load_state();
                if let Some(event) = note_core_update(&mut s2, changed, active, restart_result, &st.flclash_tag) {
                    log(&event);
                }
                let _ = save_json("vpn.json", &s2);
            }
            Err(e) => log(&t!("ядро не обновилось: {0}", e)),
        }
    }
}

#[cfg(test)]
mod contract_tests {

    #[test]
    fn auto_group_label_and_rules_follow_language() {
        crate::i18n::set(crate::i18n::Lang::Zh);
        assert_eq!(label(AUTO_GROUP), "⚡ 自动");
        assert_eq!(label("🇩🇪 DE-1"), "🇩🇪 DE-1");
        assert!(rules_template().contains("# 示例：") && rules_template().contains("GEOSITE,youtube,⚡ 自动"));
        crate::i18n::set(crate::i18n::Lang::Ru);
        assert_eq!(label(AUTO_GROUP), "⚡ Авто");
        // в правилах группа на любом языке и прежнее имя — постоянный идентификатор
        for n in ["⚡ Авто", "⚡ 自动", "⚡ تلقائي", "⚡ Auto"] {
            assert!(is_auto_group(n), "{n}");
        }
        assert!(!is_auto_group("Proxy"));
    }

    use super::*;
    use std::path::PathBuf;

    const SECRET: &str = "SUPERSECRET_SUB_TOKEN_XYZ";

    struct EnvGuard {
        _iso: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(String, Option<String>)>,
        cleanup: PathBuf,
    }

    impl EnvGuard {
        fn vpn_dirs() -> Self {
            let _iso = crate::common::contract_fixtures::isolation_lock();
            let base = std::env::temp_dir().join(format!("cm-vpn-test-{}-{}", std::process::id(), now()));
            let etc = base.join("etc");
            let home = base.join("home");
            let state = base.join("state");
            fs::create_dir_all(&etc).unwrap();
            fs::create_dir_all(&home).unwrap();
            fs::create_dir_all(&state).unwrap();
            let mut g = Self::set_vars(_iso, [("CM_VPN_ETC", etc.to_str().unwrap()), ("CM_VPN_HOME", home.to_str().unwrap()), ("CM_STATE_DIR", state.to_str().unwrap())]);
            g.cleanup = base;
            g
        }

        fn set_vars(iso: std::sync::MutexGuard<'static, ()>, pairs: [(&str, &str); 3]) -> Self {
            let mut saved = Vec::new();
            for (k, v) in pairs {
                saved.push((k.to_string(), std::env::var(k).ok()));
                unsafe {
                    std::env::set_var(k, v);
                }
            }
            Self { _iso: iso, saved, cleanup: PathBuf::new() }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.saved {
                match v {
                    Some(val) => unsafe {
                        std::env::set_var(k, val);
                    },
                    None => unsafe {
                        std::env::remove_var(k);
                    },
                }
            }
            if !self.cleanup.as_os_str().is_empty() {
                let _ = fs::remove_dir_all(&self.cleanup);
            }
        }
    }

    fn assert_no_secret(text: &str) {
        assert!(!text.contains(SECRET), "утечка секрета в: {text}");
        assert!(!text.contains("user:pass@"), "утечка userinfo в: {text}");
    }

    // --- SEC-01 ---
    #[test]
    fn sec01_safe_ureq_error_hides_url() {
        let response = ureq::Response::new(401, SECRET, SECRET).unwrap();
        let err = ureq::Error::Status(401, response);
        let msg = safe_ureq_error(&err);
        assert_eq!(msg, "HTTP 401");
        assert_no_secret(&msg);
    }

    #[test]
    fn sec01_transport_error_hides_url() {
        let url = format!("broken/url/{SECRET}");
        let err = agent(3, None).get(&url).call().unwrap_err();
        let msg = safe_ureq_error(&err);
        assert!(msg.contains("неверный"), "msg={msg}");
        assert_no_secret(&msg);
    }

    #[test]
    fn sec01_scrub_stored_error_and_publish() {
        let _g = EnvGuard::vpn_dirs();
        let subs_path = format!("{}/subs.json", etc());
        let leaked = format!("напрямую: GET {SECRET} failed https://{SECRET}@host/");
        let raw = serde_json::json!({
            "active": "a1",
            "list": [{
                "id": "a1",
                "name": "t",
                "url": format!("https://example.com/{SECRET}"),
                "error": leaked
            }]
        });
        write_private(&subs_path, raw.to_string().as_bytes()).unwrap();
        let subs = load_subs().unwrap();
        assert_no_secret(&subs.list[0].error);
        assert!(subs.list[0].error.contains("скрыт") || !subs.list[0].error.contains("://"));
        publish_state(&subs);
        let st = load_state();
        assert_no_secret(&st.subs[0].error);
        let vpn_json = fs::read_to_string(format!("{}/vpn.json", state_dir())).unwrap();
        assert_no_secret(&vpn_json);
    }

    #[test]
    fn sec01_fetch_failure_never_leaks_subscription_url() {
        let _g = EnvGuard::vpn_dirs();
        let url = format!("http://user:pass@example.invalid/sub/{SECRET}?token={SECRET}");
        let logs = std::sync::Mutex::new(Vec::<String>::new());
        let log = |s: &str| logs.lock().unwrap().push(s.to_string());
        let subs_path = format!("{}/subs.json", etc());
        let raw = serde_json::json!({
            "active": "s1",
            "list": [{
                "id": "s1",
                "name": "test",
                "url": url,
                "updated": 0
            }]
        });
        write_private(&subs_path, raw.to_string().as_bytes()).unwrap();
        let c = Config::defaults(vec![]);
        update_subs(&c, &log, true).unwrap();
        let subs = load_subs().unwrap();
        assert_no_secret(&subs.list[0].error);
        for line in logs.lock().unwrap().iter() {
            assert_no_secret(line);
        }
        let vpn_json = fs::read_to_string(format!("{}/vpn.json", state_dir())).unwrap();
        assert_no_secret(&vpn_json);
    }

    // --- SEC-02 ---
    #[test]
    fn sec02_rejects_missing_or_bad_digest() {
        assert!(verify_core_gz_digest("sha256:abc", b"data").is_err());
        assert!(verify_core_gz_digest("md5:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", b"x").is_err());
        let empty = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
        assert!(verify_core_gz_digest(empty, b"not-zero").is_err());
    }

    #[test]
    fn sec02_accepts_matching_digest() {
        use sha2::Digest;
        let payload = b"fake-gz-payload";
        let hash: String = sha2::Sha256::digest(payload).iter().map(|b| format!("{b:02x}")).collect();
        verify_core_gz_digest(&format!("sha256:{hash}"), payload).unwrap();
    }

    // --- PARSE-01 ---
    #[test]
    fn parse01_pct_decode_utf8_percent() {
        assert_eq!(pct_decode("%D0%9F%D1%80%D0%BE%D1%84%D0%B8%D0%BB%D1%8C"), "Профиль");
    }

    #[test]
    fn parse01_pct_decode_malformed_percent_does_not_panic() {
        let out = pct_decode("name%ZZ%GG%");
        assert!(out.contains('%'));
    }

    // --- GEO-01 ---
    #[test]
    fn geo01_rejects_truncated_geo_file() {
        assert!(validate_geo_file("GeoIP.dat", &[0u8; 2048]).is_err());
    }

    // --- SEC-03 ---
    #[test]
    fn sec03_rejects_http_subscription_url() {
        assert!(subscription_url("http://example.com/sub").is_err());
        assert!(subscription_url("https://example.com/sub").is_ok());
    }

    // --- SEC-05 ---
    #[test]
    fn sec05_strips_control_chars_from_profile_name() {
        assert_eq!(sanitize_profile_name("test\u{202e}name"), "testname");
    }

    // --- SEC-06 ---
    #[test]
    fn vpn_rules_io_failure_is_not_empty_rules() {
        let _g = EnvGuard::vpn_dirs();
        let path = rules_path();
        fs::create_dir(&path).unwrap();
        assert!(user_rules().unwrap_err().contains("rules.txt"));
        assert!(edit_rules().is_err());
    }

    #[test]
    fn vpn_rules_existing_file_is_read() {
        let _g = EnvGuard::vpn_dirs();
        fs::write(rules_path(), "# comment\nDOMAIN-SUFFIX,example.org,DIRECT\n").unwrap();
        assert_eq!(user_rules().unwrap(), vec!["DOMAIN-SUFFIX,example.org,DIRECT"]);
    }

    #[test]
    fn data02_missing_subs_is_ok_corrupt_is_error() {
        let _g = EnvGuard::vpn_dirs();
        assert!(load_subs().unwrap().list.is_empty());
        write_private(&format!("{}/subs.json", etc()), b"{not-json").unwrap();
        assert!(load_subs().is_err());
    }

    // --- VPN-02 ---
    #[test]
    fn vpn02_proc_ipv6_addr_parses_loopback_listener() {
        // ::1 в формате /proc/net/tcp6 (native endian по словам)
        let raw = "00000000000000000000000001000000";
        assert!(proc_ipv6_addr(raw).unwrap().is_loopback());
    }

    #[test]
    fn vpn02_listen_ports_keep_local_and_drop_foreign() {
        let header = "sl local rem st tx rx tr retr uid timeout inode";
        let tcp = format!(
            "{header}\n 0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000 0 0 42 1 0 100 0 0 10 0\n 1: 00000000:2382 00000000:0000 0A 00000000:00000000 00:00000000 00000000 0 0 42 1 0 100 0 0 10 0\n 2: 0100007F:1F92 08080808:0050 01 00000000:00000000 00:00000000 00000000 0 0 42 1 0 100 0 0 10 0\n 3: 08080808:1F93 00000000:0000 0A 00000000:00000000 00:00000000 00000000 0 0 42 1 0 100 0 0 10 0\n"
        );
        let tcp6 = format!(
            "{header}\n 0: 00000000000000000000000001000000:1F91 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000 0 0 43 1 0 100 0 0 10 0\n 1: 00000000000000000000000001000000:1F94 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000 0 0 99 1 0 100 0 0 10 0\n"
        );
        let inodes = ["42".to_string(), "43".to_string()].into_iter().collect();
        let ports = flclash_listen_ports(&tcp, &tcp6, &inodes);
        assert!(ports.contains(&(0x1F90, false)), "{ports:?}");
        assert!(ports.contains(&(0x1F91, true)), "{ports:?}");
        assert!(!ports.iter().any(|(port, _)| *port == 0x2382 || *port == 0x1F92 || *port == 0x1F93 || *port == 0x1F94), "{ports:?}");
    }

    #[test]
    fn vpn03_restart_failure_is_recorded_and_success_applies_tag() {
        let mut failed = VpnState { core_version: "v1.2.3".into(), ..Default::default() };
        let event = note_core_update(&mut failed, true, true, Err("код 1".into()), "FlClash 9").unwrap();
        assert!(event.contains("перезапуск не удался"));
        assert!(event.contains("код 1"));
        assert!(failed.flclash_applied.is_empty());
        let mut ok = VpnState { core_version: "v1.2.3".into(), ..Default::default() };
        assert!(note_core_update(&mut ok, true, true, Ok(()), "FlClash 9").is_none());
        assert_eq!(ok.flclash_applied, "FlClash 9");
        assert!(ok.event.contains("обновлено до v1.2.3"));
        let mut inactive = VpnState { core_version: "v1.2.3".into(), ..Default::default() };
        assert!(note_core_update(&mut inactive, true, false, Ok(()), "FlClash 9").is_none());
        assert!(inactive.event.contains("не запущена"));
    }

    fn proto_varint(mut n: usize) -> Vec<u8> {
        let mut out = vec![];
        loop {
            let mut byte = (n & 0x7f) as u8;
            n >>= 7;
            if n != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if n == 0 {
                break;
            }
        }
        out
    }

    fn proto_bytes(field: u32, data: &[u8]) -> Vec<u8> {
        let mut out = proto_varint(((field as usize) << 3) | 2);
        out.extend(proto_varint(data.len()));
        out.extend_from_slice(data);
        out
    }

    fn sample_geoip() -> Vec<u8> {
        let mut cidr = proto_bytes(1, &[1, 2, 3, 0]);
        cidr.extend(proto_varint((2 << 3) | 0));
        cidr.push(24);
        let mut entry = proto_bytes(1, b"US");
        entry.extend(proto_bytes(2, &cidr));
        entry.extend(proto_bytes(15, &vec![b'x'; 1200]));
        proto_bytes(1, &entry)
    }

    #[test]
    fn geo01_truncated_download_keeps_previous_file() {
        let base = std::env::temp_dir().join(format!("cm-geo-{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let path = base.join("GeoIP.dat");
        fs::write(&path, b"KEEP-PREVIOUS").unwrap();
        let valid = sample_geoip();
        assert!(valid.len() > 1024);
        assert!(validate_geo_file("GeoIP.dat", &valid).is_ok());
        let truncated = valid[..100].to_vec();
        assert!(save_geo_result(&path, "GeoIP.dat", Ok(truncated)).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"KEEP-PREVIOUS");
        assert!(save_geo_result(&path, "GeoIP.dat", Err("ответ превышает лимит".into())).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"KEEP-PREVIOUS");
        save_geo_result(&path, "GeoIP.dat", Ok(valid.clone())).unwrap();
        assert_eq!(fs::read(&path).unwrap(), valid);
        let huge = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\nx", (100 << 20) + 1);
        let response: ureq::Response = huge.parse().unwrap();
        let err = read_limited(response, 100 << 20).unwrap_err();
        assert!(err.contains("лимит"), "{err}");
        let short = "HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort".parse::<ureq::Response>().unwrap();
        let err = read_limited(short, 100 << 20).unwrap_err();
        assert!(err.contains("closed before all bytes"), "{err}");
        let _ = fs::remove_dir_all(&base);
    }

    // --- DATA-01 ---
    #[test]
    fn data01_subscriptions_lock_serializes() {
        let _g = EnvGuard::vpn_dirs();
        let l1 = subscriptions_lock(false).unwrap();
        assert!(subscriptions_lock(false).is_err());
        drop(l1);
        assert!(subscriptions_lock(false).is_ok());
    }

    #[test]
    fn vpn_config_lock_needs_only_writable_vpn_home() {
        let _g = EnvGuard::vpn_dirs();
        // ExecStartPre cannot write to the parent state directory. Make it
        // impossible to open anything there even when tests run as root.
        let state = state_dir();
        fs::remove_dir(&state).unwrap();
        fs::write(&state, b"unwritable parent fixture").unwrap();
        let first = vpn_config_lock(false).unwrap();
        assert!(vpn_config_lock(false).is_err());
        assert!(Path::new(&home()).join(".vpn-config.lock").is_file());
        drop(first);
        assert!(vpn_config_lock(false).is_ok());
        assert!(lock(false).is_err());
    }

    // --- VPN-01 ---
    #[test]
    fn vpn01_delete_last_subscription_clears_list() {
        let _g = EnvGuard::vpn_dirs();
        let subs_path = format!("{}/subs.json", etc());
        let raw = serde_json::json!({
            "active": "a1",
            "list": [{"id":"a1","name":"one","url":"https://example.com/sub","interval_h":24,"updated":0,"nodes":1,"kind":"yaml"}]
        });
        write_private(&subs_path, raw.to_string().as_bytes()).unwrap();
        fs::create_dir_all(format!("{}/profiles", home())).unwrap();
        fs::write(format!("{}/profiles/a1.yaml", home()), "proxies: []\n").unwrap();
        let name = delete_sub(&SubRef::Index(0)).unwrap();
        assert_eq!(name, "one");
        assert!(load_subs().unwrap().list.is_empty());
    }

    // --- DATA-03 ---
    #[test]
    fn data03_delete_sub_persists_json_before_profile_removal() {
        let _g = EnvGuard::vpn_dirs();
        let subs_path = format!("{}/subs.json", etc());
        write_private(
            &subs_path,
            br#"{"active":"x","list":[{"id":"x","name":"n","url":"https://example.com/s","interval_h":1,"updated":0,"nodes":0,"kind":"yaml"}]}"#,
        )
        .unwrap();
        delete_sub(&SubRef::Index(0)).unwrap();
        let on_disk = fs::read_to_string(&subs_path).unwrap();
        assert!(on_disk.contains("\"list\":[]") || on_disk.contains("\"list\": []"));
    }

    // ---------- 0.2.7 ----------

    fn write_subs(json: serde_json::Value) {
        write_private(&format!("{}/subs.json", etc()), json.to_string().as_bytes()).unwrap();
    }

    fn switch_fixture(core_script: &str) {
        two_subs();
        fs::create_dir_all(format!("{}/profiles", home())).unwrap();
        fs::create_dir_all(format!("{}/bin", home())).unwrap();
        for id in ["a1", "b2", "c3"] {
            fs::write(format!("{}/profiles/{id}.yaml", home()), clash_profile("")).unwrap();
        }
        fs::write(core_bin(), format!("#!/bin/sh\n{core_script}\n")).unwrap();
        fs::set_permissions(core_bin(), fs::Permissions::from_mode(0o700)).unwrap();
        write_private(&config_path(), b"previous configuration").unwrap();
    }
    #[test]
    fn subscription_validation_failure_preserves_choice_and_config() {
        let _g = EnvGuard::vpn_dirs();
        switch_fixture("echo 'bad group'; exit 1");
        let result = switch_subscription(&SubRef::Id("b2".into()), &Config::defaults(vec![]), true, &mut || panic!("invalid configuration must never reach running core"));
        assert!(result.unwrap_err().contains("bad group"));
        assert_eq!(load_subs().unwrap().active, "a1");
        assert_eq!(fs::read_to_string(config_path()).unwrap(), "previous configuration");
        let failure = last_failure().unwrap();
        assert_eq!(failure.stage, "validate");
        assert_eq!(failure.subscription, "two");
        assert!(failure.resolved.is_none());
        assert!(fs::read_dir(home()).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".candidate-")));
    }
    #[test]
    fn failed_reload_restores_disk_and_running_core() {
        let _g = EnvGuard::vpn_dirs();
        switch_fixture("exit 0");
        let mut calls = 0;
        let result = switch_subscription(&SubRef::Id("b2".into()), &Config::defaults(vec![]), true, &mut || {
            calls += 1;
            if calls == 1 {
                assert_ne!(fs::read_to_string(config_path()).unwrap(), "previous configuration");
                assert_eq!(load_subs().unwrap().active, "a1", "selection is not committed before reload");
                Err("reload rejected".into())
            } else {
                assert_eq!(fs::read_to_string(config_path()).unwrap(), "previous configuration");
                Ok(())
            }
        });
        assert!(result.unwrap_err().contains("restored"));
        assert_eq!(calls, 2);
        assert_eq!(load_subs().unwrap().active, "a1");
        assert_eq!(last_failure().unwrap().stage, "apply");
    }
    #[test]
    fn rollback_failure_is_explicit_and_success_resolves_saved_error() {
        let _g = EnvGuard::vpn_dirs();
        switch_fixture("exit 0");
        let result = switch_subscription(&SubRef::Id("b2".into()), &Config::defaults(vec![]), true, &mut || Err("core rejected".into()));
        assert!(result.unwrap_err().contains("rollback failed"));
        assert_eq!(load_subs().unwrap().active, "a1");
        assert_eq!(switch_subscription(&SubRef::Id("b2".into()), &Config::defaults(vec![]), true, &mut || Ok(())).unwrap(), "two");
        assert_eq!(load_subs().unwrap().active, "b2");
        assert_eq!(load_state().subs.iter().find(|s| s.active).unwrap().id, "b2");
        assert!(last_failure().unwrap().resolved.is_some());
    }
    #[test]
    fn failed_commit_after_rename_restores_subscription_and_runtime() {
        let _g = EnvGuard::vpn_dirs();
        switch_fixture("exit 0");
        let mut commits = 0;
        let mut reloads = 0;
        let result = switch_subscription_with(&SubRef::Id("b2".into()), &Config::defaults(vec![]), true,
            &mut || { reloads += 1; Ok(()) },
            &mut |subs| {
                commits += 1;
                save_subs(subs)?;
                if commits == 1 { Err("directory sync failed after rename".into()) } else { Ok(()) }
            });
        assert!(result.unwrap_err().contains("restored"));
        assert_eq!(commits, 2);
        assert_eq!(reloads, 2);
        assert_eq!(load_subs().unwrap().active, "a1");
        assert_eq!(fs::read_to_string(config_path()).unwrap(), "previous configuration");
        assert_eq!(load_state().subs.iter().find(|s| s.active).unwrap().id, "a1");
        assert_eq!(last_failure().unwrap().stage, "save");
    }
    #[test]
    fn persisted_diagnostic_redacts_profile_credentials_and_subscription_urls() {
        let _g = EnvGuard::vpn_dirs();
        switch_fixture("exit 0");
        let sub = load_subs().unwrap().list[1].clone();
        fs::write(profile_path(&sub, "clash"), clash_profile(&format!("payload: {{uuid: {SECRET}}}\n"))).unwrap();
        store_failure_locked("validate", Some(&sub), &format!("invalid uuid {SECRET}"));
        let failure = last_failure().unwrap();
        assert_no_secret(&failure.reason);
        store_failure_locked("validate", Some(&sub), &format!("GET {}?token={SECRET} failed", sub.url));
        assert_no_secret(&last_failure().unwrap().reason);
        store_failure_locked("validate", Some(&sub), &format!("GET {}?token=QUERY_ONLY_CREDENTIAL failed", sub.url));
        assert!(!last_failure().unwrap().reason.contains("QUERY_ONLY_CREDENTIAL"));
        use std::os::unix::fs::MetadataExt;
        assert_eq!(fs::metadata(failure_path()).unwrap().mode() & 0o777, 0o600);
    }
    #[test]
    fn readiness_retries_and_has_a_deadline() {
        let mut calls = 0;
        wait_ready_with(Duration::from_secs(1), || { calls += 1; if calls < 3 { Err("starting".into()) } else { Ok(()) } }).unwrap();
        assert_eq!(calls, 3);
        let before = std::time::Instant::now();
        let error = wait_ready_with(Duration::from_millis(150), || Err("not listening".into())).unwrap_err();
        assert!(error.contains("API is not ready"));
        assert!(before.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn api_readiness_does_not_claim_remote_server_connectivity() {
        let _isolation = crate::common::contract_fixtures::isolation_lock();
        crate::i18n::set(crate::i18n::Lang::Ru);
        let mut snapshot = Snapshot { running: true, groups: vec![Group { name: "Proxy".into(), kind: "Selector".into(), now: "n1".into(), ..Default::default() }], ..Default::default() };
        assert!(snapshot.health_line().contains("ещё не проверена"));
        snapshot.delay.insert("n1".into(), 0);
        assert!(snapshot.health_line().contains("ошибкой"));
        snapshot.delay.insert("n1".into(), 42);
        assert!(snapshot.health_line().contains("42 мс"));
        snapshot.running = false;
        assert!(snapshot.health_line().contains("не готов"));
    }

    fn two_subs() {
        write_subs(serde_json::json!({
            "active": "a1",
            "list": [
                {"id":"a1","name":"one","url":"https://example.com/1","kind":"clash"},
                {"id":"b2","name":"two","url":"https://example.com/2","kind":"clash"},
                {"id":"c3","name":"three","url":"https://example.com/3","kind":"clash"}
            ]
        }));
    }

    /// B22: подтверждение адресует id; изменение списка между подтверждением и удалением не задевает другую запись.
    #[test]
    fn b22_delete_and_use_by_id_survive_list_change() {
        let _g = EnvGuard::vpn_dirs();
        two_subs();
        // пользователь подтвердил «three» (строка 3); другой процесс удалил «one»
        delete_sub(&SubRef::Id("a1".into())).unwrap();
        assert_eq!(delete_sub(&SubRef::Id("c3".into())).unwrap(), "three");
        let left = load_subs().unwrap();
        assert_eq!(left.list.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["b2"]);
        // записи уже нет — ничего не удаляется и не активируется
        assert!(delete_sub(&SubRef::Id("c3".into())).is_err());
        assert!(use_sub(&SubRef::Id("a1".into()), &Config::defaults(vec![])).is_err());
        assert_eq!(load_subs().unwrap().list.len(), 1);
        assert_eq!(load_subs().unwrap().active, "b2");
        assert_eq!(load_state().subs[0].id, "b2", "id публикуется для TUI");
        assert_eq!(SubRef::parse("id:b2"), Some(SubRef::Id("b2".into())));
        assert_eq!(SubRef::parse("2"), Some(SubRef::Index(1)));
        assert_eq!(SubRef::parse("0"), None);
        assert_eq!(SubRef::parse("id:"), None);
    }

    fn clash_profile(extra: &str) -> String {
        format!("proxies:\n  - {{name: n1, type: ss, server: 1.2.3.4, port: 443, cipher: aes-128-gcm, password: x}}\nproxy-groups:\n  - {{name: Proxy, type: select, proxies: [n1]}}\nrules:\n  - MATCH,Proxy\n{extra}")
    }

    fn build_with(profile: &str, c: &Config) -> Result<Mapping, String> {
        write_subs(serde_json::json!({"active":"p1","list":[{"id":"p1","name":"p","url":"https://example.com/p","kind":"clash"}]}));
        fs::create_dir_all(format!("{}/profiles", home())).unwrap();
        fs::write(format!("{}/profiles/p1.yaml", home()), profile).unwrap();
        let y = build_config(c)?;
        match serde_yaml::from_str::<Value>(&y).unwrap() {
            Value::Mapping(m) => Ok(m),
            _ => panic!("не mapping"),
        }
    }

    /// B01: listeners, свой DNS, skip-auth-prefixes и прочие ключи подписки не попадают в конфиг.
    #[test]
    fn russian_auto_setting_preserves_manual_nodes_and_provider_filtering() {
        let _g = EnvGuard::vpn_dirs();
        let profile = clash_profile("proxy-providers:\n  remote: {type: http, url: 'https://example.com/nodes', path: ./providers/remote.yaml}\n");
        let mut c = Config::defaults(vec![]);
        let allowed = build_with(&profile, &c).unwrap();
        let filter = |m: &Mapping| m["proxy-groups"].as_sequence().unwrap().iter()
            .find(|g| g["name"].as_str() == Some(AUTO_GROUP)).unwrap()["exclude-filter"].as_str().unwrap().to_string();
        assert!(!filter(&allowed).contains("🇷🇺"));
        c.set("vpn_auto_allow_ru", "0").unwrap();
        assert!(c.set("vpn_auto_allow_ru", "2").is_err());
        let excluded = build_with(&profile, &c).unwrap();
        assert!(filter(&excluded).contains("🇷🇺"));
        assert_eq!(allowed["proxies"], excluded["proxies"]);
        assert_eq!(allowed["proxy-providers"], excluded["proxy-providers"]);
        let groups = excluded["proxy-groups"].as_sequence().unwrap();
        assert_eq!(groups[0]["include-all"], Value::Bool(true));
        let manual = groups.iter().find(|g| g["name"].as_str() == Some("Proxy")).unwrap();
        assert!(manual.get("exclude-filter").is_none());
        c.set("vpn_auto_allow_ru", "1").unwrap();
        assert_eq!(filter(&build_with(&profile, &c).unwrap()), filter(&allowed));
        let mut legacy = serde_json::to_value(&c).unwrap();
        legacy.as_object_mut().unwrap().remove("vpn_auto_allow_ru");
        assert!(serde_json::from_value::<Config>(legacy).unwrap().vpn_auto_allow_ru);
    }

    #[test]
    fn b01_hostile_profile_keys_are_dropped() {
        let _g = EnvGuard::vpn_dirs();
        let hostile = "listeners:\n  - {name: open, type: mixed, port: 7777, listen: 0.0.0.0}\ndns: {enable: true, listen: 0.0.0.0:53}\nskip-auth-prefixes: [0.0.0.0/0]\nexternal-controller: 0.0.0.0:9090\nexternal-controller-cors: {allow-origins: ['*']}\nhosts: {a: 1.1.1.1}\nsniffer: {enable: true}\niptables: {enable: true}\nebpf: {redirect-to-tun: [eth0]}\nauthentication: ['u:p']\nexternal-ui: /etc\n";
        let mut c = Config::defaults(vec![]);
        c.vpn_dns = false;
        let m = build_with(&clash_profile(hostile), &c).unwrap();
        for key in ["listeners", "dns", "skip-auth-prefixes", "external-controller", "external-controller-cors", "hosts", "sniffer", "iptables", "ebpf", "authentication", "external-ui"] {
            assert!(!m.contains_key(key), "{key} остался");
        }
        assert_eq!(m.get("bind-address").and_then(Value::as_str), Some("127.0.0.1"));
        assert_eq!(m.get("geo-auto-update").and_then(Value::as_bool), Some(false));
        assert!(m.get("proxies").and_then(Value::as_sequence).map(|s| s.len() == 1).unwrap_or(false));
        // B02: только Unix-сокет в каталоге службы
        assert!(m.get("external-controller-unix").and_then(Value::as_str).unwrap().starts_with(&home()));
        assert!(!m.contains_key("secret"), "секрет не нужен: API только через сокет в закрытом каталоге");
        assert!(!Path::new(&format!("{}/secret", etc())).exists(), "сборка конфига ничего не пишет в /etc");
        c.vpn_dns = true;
        c.vpn_allow_lan = true;
        let m = build_with(&clash_profile(hostile), &c).unwrap();
        assert_eq!(m.get("dns").and_then(|d| d.get("listen")).and_then(Value::as_str), Some("127.0.0.1:1053"), "DNS — свой, не из подписки");
        assert_eq!(m.get("bind-address").and_then(Value::as_str), Some("*"));
    }

    /// B01: provider с путём вне каталога профиля или файловый без пути отвергается.
    #[test]
    fn b01_provider_paths_are_confined() {
        let _g = EnvGuard::vpn_dirs();
        let c = Config::defaults(vec![]);
        for bad in ["/etc/shadow", "../secret", "bin/mihomo", "config.yaml", "providers/../../etc/x", "profiles", "rule-sets", "rule-sets/../../etc/x", "/rule-sets/x.mrs"] {
            let p = format!("proxy-providers:\n  x: {{type: file, path: '{bad}'}}\n");
            assert!(build_with(&clash_profile(&p), &c).is_err(), "{bad}");
        }
        assert!(build_with(&clash_profile("rule-providers:\n  r: {type: file}\n"), &c).is_err());
        assert!(build_with(&clash_profile("rule-providers:\n  r: {type: exec, path: ./rules/a}\n"), &c).is_err());
        let ok = "proxy-providers:\n  x: {type: http, url: 'https://e.com/p', path: ./providers/x.yaml}\nrule-providers:\n  r: {type: http, url: 'https://e.com/r', behavior: domain}\n";
        let m = build_with(&clash_profile(ok), &c).unwrap();
        assert!(m.contains_key("proxy-providers") && m.contains_key("rule-providers"));
    }

    #[test]
    fn rule_sets_provider_is_isolated_without_changing_format() {
        let _g = EnvGuard::vpn_dirs();
        let c = Config::defaults(vec![]);
        let profile = clash_profile("rule-providers:\n  telegram_domains: {type: http, url: 'https://example.com/telegram_domains.mrs', behavior: domain, format: mrs, path: ./rule-sets/telegram_domains.mrs}\n");
        let m = build_with(&profile, &c).unwrap();
        let provider = &m["rule-providers"]["telegram_domains"];
        assert!(provider["path"].as_str().unwrap().starts_with("./providers/rule-providers/"));
        assert!(provider["path"].as_str().unwrap().ends_with("/rule-sets/telegram_domains.mrs"));
        assert_eq!(provider["format"].as_str(), Some("mrs"));
    }

    #[test]
    fn custom_http_provider_directories_are_relocated_to_cache() {
        let _g = EnvGuard::vpn_dirs();
        let c = Config::defaults(vec![]);
        for path in ["./oisd/big.mrs", "./custom/nested/telegram.mrs", "cache.yaml", "bin/mihomo"] {
            let profile = clash_profile(&format!("rule-providers:\n  oisd_big: {{type: http, url: 'https://example.com/big.mrs', behavior: domain, format: mrs, path: '{path}'}}\n"));
            let m = build_with(&profile, &c).unwrap();
            let provider = &m["rule-providers"]["oisd_big"];
            let cache = provider["path"].as_str().unwrap();
            assert!(cache.starts_with("./providers/rule-providers/") && provider_path_ok(cache), "{cache}");
            assert!(cache.ends_with(path.trim_start_matches("./")), "{cache}");
            assert_eq!(provider["format"].as_str(), Some("mrs"));
            assert_eq!(provider["url"].as_str(), Some("https://example.com/big.mrs"));
            assert_eq!(build_with(&profile, &c).unwrap(), m, "кэш стабилен при повторной сборке");
        }
        assert_ne!(provider_cache_path("./oisd/big.mrs", "rule-providers", "https://a.example/big"), provider_cache_path("./oisd/big.mrs", "rule-providers", "https://b.example/big"));
        for path in ["/etc/shadow", "../secret", "oisd/../../config.yaml", "./", "", "oisd/bad\nfile"] {
            assert!(provider_cache_path(path, "rule-providers", "https://example.com/big").is_none(), "{path}");
        }
        for path in ["/etc/shadow", "../secret", "oisd/../../config.yaml"] {
            let profile = clash_profile(&format!("rule-providers:\n  r: {{type: http, url: 'https://example.com/big', path: '{path}'}}\n"));
            assert!(build_with(&profile, &c).is_err(), "{path}");
        }
    }

    #[test]
    fn http_cache_isolated_for_existing_paths_headers_and_names() {
        let _g = EnvGuard::vpn_dirs();
        let c = Config::defaults(vec![]);
        let path = |name: &str, url: &str, token: &str| {
            let profile = clash_profile(&format!("rule-providers:\n  {name}: {{type: http, url: '{url}', behavior: domain, path: ./rules/shared.yaml, header: {{Authorization: ['{token}']}}}}\n"));
            build_with(&profile, &c).unwrap()["rule-providers"][name]["path"].as_str().unwrap().to_string()
        };
        let original = path("a", "https://a.example/rules", "first");
        assert_ne!(original, path("a", "https://b.example/rules", "first"));
        assert_ne!(original, path("b", "https://a.example/rules", "first"));
        assert_ne!(original, path("a", "https://a.example/rules", "second"));
        assert!(!original.contains("first"));
    }

    #[test]
    fn file_provider_links_follow_cache_and_ambiguous_links_fail() {
        let _g = EnvGuard::vpn_dirs();
        let c = Config::defaults(vec![]);
        let profile = "rule-providers:\n  remote: {type: http, url: 'https://example.com/a', behavior: domain, path: ./custom/a.yaml}\n  local: {type: file, behavior: domain, path: custom/a.yaml}\n";
        let m = build_with(&clash_profile(profile), &c).unwrap();
        assert_eq!(m["rule-providers"]["remote"]["path"], m["rule-providers"]["local"]["path"]);
        let ambiguous = format!("{profile}  other: {{type: http, url: 'https://other.example/a', behavior: domain, path: custom/a.yaml}}\n");
        assert!(build_with(&clash_profile(&ambiguous), &c).unwrap_err().contains("неоднозначный"));
    }

    #[test]
    fn local_provider_file_errors_and_symlinks_are_checked() {
        let _g = EnvGuard::vpn_dirs();
        let c = Config::defaults(vec![]);
        let profile = clash_profile("rule-providers:\n  local: {type: file, behavior: domain, path: ./rules/local.yaml}\n");
        assert!(build_with(&profile, &c).unwrap_err().contains("файл не найден"));
        fs::create_dir_all(format!("{}/rules", home())).unwrap();
        fs::write(format!("{}/rules/local.yaml", home()), "payload: [example.com]\n").unwrap();
        assert!(build_with(&profile, &c).is_ok());
        fs::remove_file(format!("{}/rules/local.yaml", home())).unwrap();
        std::os::unix::fs::symlink("/etc/hosts", format!("{}/rules/local.yaml", home())).unwrap();
        assert!(build_with(&profile, &c).unwrap_err().contains("вне каталога"));
    }

    #[test]
    fn provider_types_formats_and_references_are_checked() {
        let _g = EnvGuard::vpn_dirs();
        let c = Config::defaults(vec![]);
        for (provider, message) in [
            ("{type: http, behavior: domain}", "URL"),
            ("{type: http, url: 'file:///etc/hosts', behavior: domain}", "URL"),
            ("{type: http, url: 'https://example.com/r', behavior: classical, format: mrs}", "behavior"),
            ("{type: http, url: 'https://example.com/r', behavior: domain, format: wrong}", "формат"),
        ] {
            let profile = clash_profile(&format!("rule-providers:\n  invalid: {provider}\n"));
            assert!(build_with(&profile, &c).unwrap_err().contains(message));
        }
        let inline = clash_profile("rule-providers:\n  inline: {type: inline, behavior: domain, path: /unused, payload: [example.com]}\n");
        assert!(!build_with(&inline, &c).unwrap()["rule-providers"]["inline"].as_mapping().unwrap().contains_key("path"));
        assert!(build_with(&clash_profile("sub-rules:\n  local: ['RULE-SET,missing,Proxy']\n"), &c).is_err());
        assert!(build_with(&clash_profile("proxy-groups:\n  - {name: Group, type: select, use: [missing]}\n"), &c).unwrap_err().contains("missing"));
    }

    #[test]
    fn doctor_reports_each_profile_without_changing_selection_or_config() {
        let _g = EnvGuard::vpn_dirs();
        switch_fixture("echo 'core rejected fixture' >&2; exit 1");
        let previous = fs::read_to_string(config_path()).unwrap();
        let checks = doctor(&Config::defaults(vec![]), None).unwrap();
        assert_eq!(checks.len(), load_subs().unwrap().list.len());
        assert!(checks.iter().all(|c| c.error.as_deref().unwrap().contains("core rejected")), "{checks:?}");
        assert_eq!(load_subs().unwrap().active, "a1");
        assert_eq!(fs::read_to_string(config_path()).unwrap(), previous);
        assert_eq!(doctor(&Config::defaults(vec![]), Some(&SubRef::Id("b2".into()))).unwrap().len(), 1);
        assert!(fs::read_dir(home()).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".candidate-")));
    }

    #[test]
    fn prepare_validation_failure_keeps_previous_config() {
        let _g = EnvGuard::vpn_dirs();
        switch_fixture("if [ \"$1\" = '-v' ]; then echo 'mihomo v1.19.0'; exit 0; fi; echo 'bad rule provider' >&2; exit 1");
        let mut c = Config::defaults(vec![]);
        c.vpn_tun = false;
        c.vpn_port = 47_123;
        c.vpn_dns = false;
        let error = prepare(&c, &|_| {}).unwrap_err();
        assert!(error.contains("bad rule provider"), "{error}");
        assert_eq!(fs::read_to_string(config_path()).unwrap(), "previous configuration");
    }

    #[test]
    fn diagnostic_hides_provider_headers_and_age_secrets() {
        let profile = serde_yaml::from_str::<Value>("proxy-providers:\n  a: {type: http, age-secret-key: 'AGE-SECRET-123', header: {Authorization: ['Bearer abcsecret'], X-Token: ['headersecret']}}\n").unwrap();
        let mut text = "invalid AGE-SECRET-123 Bearer abcsecret headersecret".to_string();
        redact_credentials(&profile, &mut text);
        assert!(!text.contains("SECRET-123") && !text.contains("abcsecret") && !text.contains("headersecret"));
    }

    #[test]
    fn subscription_negotiation_skips_placeholder_and_html_then_accepts_mihomo() {
        let placeholder = "proxies: [{name: 'unsupported client', type: ss, server: 127.0.0.1, port: 1}]";
        let mut attempted = Vec::new();
        let (body, (metadata, kind, nodes)) = negotiate_subscription("Custom/1", |agent| {
            attempted.push(agent.to_string());
            Ok((match attempted.len() { 1 => placeholder.into(), 2 => "<html>subscription page</html>".into(), _ => clash_profile("") }, attempted.len()))
        }).unwrap();
        assert_eq!(attempted, ["Custom/1", UA, "ClashMeta/1.19.32"]);
        assert_eq!((metadata, kind.as_str(), nodes), (3, "clash", 1));
        assert!(!body.contains("unsupported client"));
    }

    #[test]
    fn subscription_negotiation_accepts_uri_base64_and_stops_on_auth_failure() {
        let mut calls = 0;
        let (_, (_, kind, nodes)) = negotiate_subscription("", |_| {
            calls += 1;
            Ok(("dmxlc3M6Ly91c2VyQGV4YW1wbGUuY29tOjQ0MyNOb2Rl".into(), ()))
        }).unwrap();
        assert_eq!((calls, kind.as_str(), nodes), (1, "uri", 1));
        calls = 0;
        let error = negotiate_subscription::<()>("", |_| { calls += 1; Err("HTTP 403".into()) }).unwrap_err();
        assert_eq!(error, "HTTP 403");
        assert_eq!(calls, 1);
        calls = 0;
        assert!(negotiate_subscription("", |_| { calls += 1; Ok(("<html>unsupported</html>".into(), ())) }).is_err());
        assert_eq!(calls, SUBSCRIPTION_AGENTS.len());
    }

    #[test]
    fn unsupported_client_placeholder_and_html_are_not_valid_subscriptions() {
        let _g = EnvGuard::vpn_dirs();
        let body = clash_profile("").replace("name: n1", "name: Приложение не поддерживается");
        assert!(classify_sub(&body).unwrap_err().contains("User-Agent"));
        assert!(build_with(&body, &Config::defaults(vec![])).unwrap_err().contains("заглушку"));
        assert!(classify_sub("<html><a href=\"https://example.com\">Error</a></html>").is_err());
        assert!(classify_sub("url: https://example.com/rules\ninvalid: [").is_err());
        assert_eq!(classify_sub("vless://example-user@example.com:443#Node\n").unwrap(), ("uri".into(), 1));
    }

    #[test]
    fn core_validation_keeps_real_error_before_generic_failure_line() {
        assert_eq!(core_validation_reason("", "level=error msg=\"proxy group: node not found\"\nconfiguration file /tmp/test.yaml test failed\n"), "level=error msg=\"proxy group: node not found\"");
        assert_eq!(core_validation_reason("bad file\n", "configuration file /tmp/test.yaml test failed\n"), "bad file");
    }

    #[test]
    fn http_provider_empty_path_uses_private_default_cache() {
        let _g = EnvGuard::vpn_dirs();
        let c = Config::defaults(vec![]);
        let profile = clash_profile("rule-providers:\n  remote: {type: http, url: 'https://example.com/r', behavior: domain, path: ''}\n");
        let m = build_with(&profile, &c).unwrap();
        assert!(m["rule-providers"]["remote"]["path"].as_str().unwrap().ends_with("/provider.yaml"));
        let wrong = profile.replace("path: ''", "path: 123");
        assert!(build_with(&wrong, &c).is_err());
    }

    #[test]
    fn per_subscription_user_agent_is_validated_and_persisted_privately() {
        let _g = EnvGuard::vpn_dirs();
        two_subs();
        let reference = SubRef::Id("b2".into());
        set_user_agent(&reference, "mihomo/1.19.0").unwrap();
        let subs = load_subs().unwrap();
        assert_eq!(subs.active, "a1");
        assert_eq!(subs.list[0].user_agent, "");
        assert_eq!(subs.list[1].user_agent, "mihomo/1.19.0");
        for agent in ["", "bad\nheader", "bad\rheader", "кириллица"] { assert!(set_user_agent(&reference, agent).is_err()); }
        assert!(set_user_agent(&reference, &"x".repeat(513)).is_err());
        assert_eq!(load_subs().unwrap().list[1].user_agent, "mihomo/1.19.0");
        assert!(!fs::read_to_string(format!("{}/vpn.json", state_dir())).unwrap().contains("user_agent"));
    }

    #[test]
    fn subscription_request_uses_selected_user_agent() {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut headers = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() { break; }
                headers.push_str(&line);
            }
            std::io::Write::write_all(&mut stream, b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
            headers
        });
        let _response = get_with_redirects_user_agent(&format!("http://{address}/profile"), 3, 7890, 0, "mihomo/1.19.0").unwrap();
        assert!(worker.join().unwrap().to_ascii_lowercase().contains("user-agent: mihomo/1.19.0\r\n"));
    }

    /// B23: «миллиард смешков» укладывается в лимит тела, но отвергается бюджетом до сборки конфига.
    #[test]
    fn b23_alias_bomb_is_rejected_by_budget() {
        let mut bomb = String::from("a: &a [x, x, x, x, x, x, x, x, x, x]\n");
        for i in 1..9 {
            let prev = (b'a' + i - 1) as char;
            let cur = (b'a' + i) as char;
            bomb += &format!("{cur}: &{cur} [*{prev}, *{prev}, *{prev}, *{prev}, *{prev}, *{prev}, *{prev}, *{prev}, *{prev}, *{prev}]\n");
        }
        assert!(bomb.len() < 4096);
        let t0 = std::time::Instant::now();
        assert!(parse_profile(&bomb).is_err());
        assert!(t0.elapsed() < std::time::Duration::from_secs(5));
        // размножение, которое проходит предел повторов serde_yaml, останавливает наш бюджет узлов
        let base = "base: &b [".to_string() + &vec!["1"; 20_000].join(",") + "]\nx: [" + &vec!["*b"; 60].join(",") + "]\n";
        assert!(parse_profile(&base).unwrap_err().contains("больше"), "узлы");
        let big = "s: &s '".to_string() + &"x".repeat(1 << 20) + "'\nx: [" + &vec!["*s"; 70].join(",") + "]\n";
        assert!(parse_profile(&big).unwrap_err().contains("больше"), "байты");
        assert!(classify_sub(&format!("proxies: [1]\n{bomb}")).map(|(k, _)| k != "clash").unwrap_or(true));
        let deep = "a: ".to_string() + &"[".repeat(200) + &"]".repeat(200);
        assert!(parse_profile(&deep).is_err());
        let many_aliases = "a: &a 1\nb: [".to_string() + &vec!["*a"; YAML_MAX_ALIASES + 1].join(", ") + "]\n";
        assert!(parse_profile(&many_aliases).unwrap_err().contains("алиасов"));
        // обычные якоря и слияние << работают
        let v = parse_profile("base: &b {type: select}\ng: {<<: *b, name: G}\n").unwrap();
        assert_eq!(v["g"]["type"].as_str(), Some("select"));
    }

    /// B13: порт прокси, совпадающий с DNS, не попадает в YAML; конфиг cm при этом читается.
    #[test]
    fn b13_port_conflict_is_reported_by_vpn_build() {
        let _g = EnvGuard::vpn_dirs();
        let mut c = Config::defaults(vec![]);
        c.vpn_port = DNS_PORT;
        assert!(build_with(&clash_profile(""), &c).unwrap_err().contains("DNS"));
        c.vpn_dns = false;
        assert!(build_with(&clash_profile(""), &c).is_ok());
        c.vpn_port = 80;
        assert!(check_port(&c).is_err());
        c.vpn_port = 9097;
        assert!(check_port(&c).is_ok(), "9097 больше не занят контроллером");
    }

    /// B15: счётчики — целые без потери точности и без переполнения.
    #[test]
    fn b15_userinfo_counters_are_exact_and_safe() {
        let i = parse_userinfo("upload=9007199254740993; download=1; total=18446744073709551615; expire=1700000000");
        assert_eq!(i.upload, 9_007_199_254_740_993, "2^53+1 не округляется");
        assert_eq!(i.total, u64::MAX);
        let i = parse_userinfo("upload=18446744073709551615; download=18446744073709551615; total=18446744073709551615");
        assert!(i.nearly_exhausted());
        assert_eq!(fmt_bytes_wide(i.used()), fmt_bytes(u64::MAX));
        let i = parse_userinfo("upload=-5; download=1.5; total=1e400; expire=-1");
        assert_eq!((i.upload, i.download, i.total, i.expire), (0, 0, 0, 0));
        assert_eq!(parse_userinfo("upload=1.0e3").upload, 1000);
        let small = SubInfo { upload: 50, download: 39, total: 100, expire: 0 };
        assert!(!small.nearly_exhausted());
        assert!(SubInfo { download: 40, ..small }.nearly_exhausted());
    }

    /// B03: распаковка останавливается на пределе, до записи бинарника.
    #[test]
    fn b03_gunzip_is_limited() {
        use std::io::Write;
        let mut enc = flate2::write::GzEncoder::new(vec![], flate2::Compression::best());
        enc.write_all(&vec![0u8; 4 << 20]).unwrap();
        let gz = enc.finish().unwrap();
        assert!(gz.len() < 64 << 10);
        assert!(gunzip_limited(&gz, 1 << 20).unwrap_err().contains("больше"));
        assert_eq!(gunzip_limited(&gz, 4 << 20).unwrap().len(), 4 << 20);
    }

    /// B25: огромный интервал из заголовка не переполняет срок проверки.
    #[test]
    fn b25_huge_interval_does_not_overflow() {
        assert_eq!(hours_secs(i64::MAX), MAX_HOURS * 3600);
        assert_eq!(hours_secs(-5), 3600);
        assert!(!elapsed_at_least(now(), hours_secs(i64::MAX)));
        assert!(elapsed_at_least(i64::MIN, 1), "без переполнения на вычитании");
        let _g = EnvGuard::vpn_dirs();
        write_subs(serde_json::json!({"active":"a","list":[{"id":"a","name":"n","url":"https://example.invalid/s","updated": now(),"interval_h": i64::MAX}]}));
        assert!(!update_subs(&Config::defaults(vec![]), &|_| {}, false).unwrap(), "срок не подошёл, запросов нет");
    }

    fn proc_fixture(pid: &str, comm: &str, ours: bool, sockets: &[&str], tuns: &[&str]) -> ProcInfo {
        ProcInfo { pid: pid.into(), comm: comm.into(), ours, sockets: sockets.iter().map(|s| s.to_string()).collect(), tuns: tuns.iter().map(|s| s.to_string()).collect() }
    }

    /// B24: имя процесса без TUN и порта не мешает; настоящий занятый порт или чужой TUN — мешает.
    #[test]
    fn b24_conflict_needs_real_port_or_tun() {
        let c = Config::defaults(vec![]);
        let header = "sl local rem st tx rx tr retr uid timeout inode\n";
        // 0x1ED9 = 7897 (vpn_port по умолчанию), 0x041D = 1053
        let tcp = format!("{header} 0: 0100007F:1ED9 00000000:0000 0A 0:0 0:0 0 0 0 555 1\n");
        let udp = format!("{header} 0: 0100007F:041D 00000000:0000 07 0:0 0:0 0 0 0 777 1\n");
        let fake = [proc_fixture("10", "FlClashCore", false, &["1"], &[])];
        assert!(find_conflict(&c, &fake, &[(header, false), (header, true)]).is_none(), "подменённое имя без порта и TUN");
        let busy = [proc_fixture("11", "nc", false, &["555"], &[])];
        let msg = find_conflict(&c, &busy, &[(&tcp, false)]).unwrap();
        assert!(msg.contains("7897") && msg.contains("nc"), "{msg}");
        let ours = [proc_fixture("12", "mihomo", true, &["555", "777"], &["cm-vpn"])];
        assert!(find_conflict(&c, &ours, &[(&tcp, false), (&udp, true)]).is_none(), "свой mihomo — не конфликт");
        let dns = [proc_fixture("13", "dnsmasq", false, &["777"], &[])];
        assert!(find_conflict(&c, &dns, &[(&udp, true)]).unwrap().contains("1053"));
        let flclash = [proc_fixture("14", "FlClashCore", false, &[], &["FlClash"])];
        assert!(find_conflict(&c, &flclash, &[]).unwrap().contains("FlClash"));
        let mesh = [proc_fixture("15", "tailscaled", false, &[], &["tailscale0"])];
        assert!(find_conflict(&c, &mesh, &[]).is_none());
        let mut proxy = c.clone();
        proxy.vpn_tun = false;
        assert!(find_conflict(&proxy, &flclash, &[]).is_none(), "в режиме прокси TUN не мешает");
    }

    /// B02: клиент API ходит только в свой Unix-сокет, проверяет права и разбирает ответ; TCP не используется.
    #[test]
    fn b02_api_over_unix_socket() {
        use std::io::Write;
        use std::os::unix::net::UnixListener;
        let _g = EnvGuard::vpn_dirs();
        fs::set_permissions(home(), fs::Permissions::from_mode(0o700)).unwrap();
        let listener = UnixListener::bind(api_socket()).unwrap();
        // как у mihomo: сам сокет открыт всем, защищает каталог
        fs::set_permissions(api_socket(), fs::Permissions::from_mode(0o666)).unwrap();
        let server = std::thread::spawn(move || {
            let mut seen = vec![];
            for body in ["{\"version\":\"v1.19.0\"}", "chunked"] {
                let (mut s, _) = listener.accept().unwrap();
                let mut req = vec![0u8; 4096];
                let n = s.read(&mut req).unwrap();
                seen.push(String::from_utf8_lossy(&req[..n]).into_owned());
                if body == "chunked" {
                    s.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n{\"a\":\r\n2\r\n1}\r\n0\r\n\r\n").unwrap();
                } else {
                    s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).unwrap();
                }
            }
            seen
        });
        assert_eq!(api("GET", "/version", None).unwrap()["version"], "v1.19.0");
        assert_eq!(api("PUT", "/configs?force=true", Some(serde_json::json!({"path":"x"}))).unwrap()["a"], 1);
        let seen = server.join().unwrap();
        assert!(seen[0].starts_with("GET /version HTTP/1.1\r\n"));
        assert!(!seen.iter().any(|r| r.contains("Authorization")), "секрет не отправляется");
        assert!(seen[1].ends_with("{\"path\":\"x\"}"));
        // каталог сокета доступен группе — запрос не отправляется
        fs::set_permissions(home(), fs::Permissions::from_mode(0o750)).unwrap();
        assert!(api("GET", "/version", None).unwrap_err().contains("чужой"));
        fs::set_permissions(home(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_file(api_socket()).unwrap();
        fs::write(api_socket(), b"").unwrap();
        assert!(api("GET", "/version", None).is_err(), "обычный файл вместо сокета");
        assert!(parse_http_response(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc").is_err());
    }

    /// B14: prepare не качает ядро, а коротко отказывает, когда его нет.
    #[test]
    fn b14_prepare_without_core_fails_fast() {
        let _g = EnvGuard::vpn_dirs();
        let mut c = Config::defaults(vec![]);
        c.vpn_port = 47_123;
        c.vpn_dns = false;
        c.vpn_tun = false;
        let t0 = std::time::Instant::now();
        let err = prepare(&c, &|_| {}).unwrap_err();
        assert!(err.contains("core update"), "{err}");
        assert!(t0.elapsed() < std::time::Duration::from_secs(2));
    }
}
