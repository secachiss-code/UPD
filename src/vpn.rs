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

pub const SERVICE: &str = "upd-vpn.service";
pub const AUTO_GROUP: &str = "⚡ Авто";
const CONTROLLER: &str = "127.0.0.1:9097";
const TUN_DEV: &str = "upd-vpn";
pub const TEST_URL: &str = "https://www.gstatic.com/generate_204";
/// Панели подписок (Marzban, Remnawave, 3x-ui…) по User-Agent отдают конфиг для mihomo/FlClash
const UA: &str = "FlClash/0.8 mihomo/1.19 (upd)";
const GEO_BASE: &str = "https://github.com/MetaCubeX/meta-rules-dat/releases/download/latest";
/// (имя файла, которое ищет mihomo в своём каталоге; имя в релизе meta-rules-dat)
const GEO: [(&str, &str); 4] = [("geoip.metadb", "geoip.metadb"), ("GeoSite.dat", "geosite.dat"), ("GeoIP.dat", "geoip.dat"), ("ASN.mmdb", "GeoLite2-ASN.mmdb")];
static PROFILE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn etc() -> String {
    env_or("UPD_VPN_ETC", "/etc/upd/vpn")
}
pub fn home() -> String {
    env_or("UPD_VPN_HOME", "/var/lib/upd/vpn")
}
pub fn core_bin() -> String {
    format!("{}/bin/mihomo", home())
}
fn config_path() -> String {
    format!("{}/config.yaml", home())
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

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Sub {
    pub id: String,
    pub name: String,
    /// адрес подписки — секрет: хранится только в /etc/upd/vpn (0600), наружу не выводится
    pub url: String,
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
        *error = "ошибка запроса (URL скрыт)".into();
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
            ureq::ErrorKind::InvalidUrl => "неверный URL".into(),
            ureq::ErrorKind::UnknownScheme => "неподдерживаемая схема URL".into(),
            ureq::ErrorKind::Dns => "ошибка DNS".into(),
            ureq::ErrorKind::InsecureRequestHttpsOnly => "небезопасная HTTP схема".into(),
            ureq::ErrorKind::ConnectionFailed => "ошибка соединения".into(),
            ureq::ErrorKind::TooManyRedirects => "слишком много перенаправлений".into(),
            ureq::ErrorKind::BadStatus => "неверный HTTP ответ".into(),
            ureq::ErrorKind::BadHeader => "неверный HTTP заголовок".into(),
            ureq::ErrorKind::Io => "ошибка ввода-вывода".into(),
            ureq::ErrorKind::InvalidProxyUrl => "неверный адрес прокси".into(),
            ureq::ErrorKind::ProxyConnect => "ошибка соединения с прокси".into(),
            ureq::ErrorKind::ProxyUnauthorized => "ошибка авторизации прокси".into(),
            ureq::ErrorKind::HTTP => "ошибка HTTP".into(),
        },
    }
}

/// GET напрямую; если не вышло — через наш VPN (если работает), затем через прокси FlClash (если запущен).
fn get(url: &str, timeout: u64, port: u16) -> Result<ureq::Response, String> {
    get_with_redirects(url, timeout, port, 5)
}

fn get_with_redirects(url: &str, timeout: u64, port: u16, redirects: u32) -> Result<ureq::Response, String> {
    let mut errs = match agent_with_redirects(timeout, None, redirects).get(url).call() {
        Ok(r) => return Ok(r),
        Err(e) => vec![format!("напрямую: {}", safe_ureq_error(&e))],
    };
    let mut via: Vec<((u16, bool), &str)> = vec![];
    if running() {
        via.push(((port, false), "через VPN"));
    }
    via.extend(flclash_ports().into_iter().map(|p| (p, "через FlClash")));
    for ((p, ipv6), what) in via {
        match agent_with_redirects(timeout, Some((p, ipv6)), redirects).get(url).call() {
            Ok(r) => return Ok(r),
            Err(e) => errs.push(format!("{what} :{p}: {}", safe_ureq_error(&e))),
        }
    }
    Err(errs.join("; "))
}

fn subscription_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| "некорректный URL подписки".to_string())?;
    if url.scheme() != "https" {
        return Err("URL подписки должен использовать HTTPS; HTTP не поддерживается".into());
    }
    if url.host_str().filter(|host| !host.is_empty()).is_none() {
        return Err("у URL подписки отсутствует host".into());
    }
    Ok(url)
}

fn get_subscription(raw: &str, timeout: u64, port: u16) -> Result<ureq::Response, String> {
    const MAX_REDIRECTS: u32 = 5;
    let mut current = subscription_url(raw)?;
    for redirects in 0..=MAX_REDIRECTS {
        let response = get_with_redirects(current.as_str(), timeout, port, 0)?;
        if !matches!(response.status(), 301 | 302 | 303 | 307 | 308) {
            if (300..400).contains(&response.status()) {
                return Err(format!("HTTP {} вместо профиля подписки", response.status()));
            }
            return Ok(response);
        }
        if redirects == MAX_REDIRECTS {
            return Err("слишком много перенаправлений подписки".into());
        }
        let location = response.header("location").ok_or("в перенаправлении нет Location")?;
        current = subscription_url(current.join(location).map_err(|_| "неверный адрес перенаправления подписки")?.as_str())?;
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

/// Локальные mixed-порты FlClashCore; bool указывает, что listener доступен по IPv6.
fn flclash_ports() -> Vec<(u16, bool)> {
    let mut inodes = std::collections::HashSet::new();
    for e in fs::read_dir("/proc").into_iter().flatten().flatten() {
        if fs::read_to_string(e.path().join("comm")).unwrap_or_default().trim() != "FlClashCore" {
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
    let mut ports = vec![];
    for line in fs::read_to_string("/proc/net/tcp").unwrap_or_default().lines().skip(1) {
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
    for line in fs::read_to_string("/proc/net/tcp6").unwrap_or_default().lines().skip(1) {
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

fn read_limited(r: ureq::Response, max: u64) -> Result<Vec<u8>, String> {
    let expected = r.header("content-length").and_then(|v| v.parse::<u64>().ok());
    if expected.map(|size| size > max).unwrap_or(false) {
        return Err(format!("ответ превышает лимит {}", fmt_bytes(max)));
    }
    let mut b = vec![];
    r.into_reader().take(max.saturating_add(1)).read_to_end(&mut b).map_err(|e| e.to_string())?;
    if b.len() as u64 > max {
        return Err(format!("ответ превышает лимит {}", fmt_bytes(max)));
    }
    if expected.map(|size| size != b.len() as u64).unwrap_or(false) {
        return Err("ответ обрезан относительно Content-Length".into());
    }
    Ok(b)
}

/// Как read_limited, но пишет в лог прогресс каждые 10% (для больших файлов).
fn read_progress(r: ureq::Response, max: u64, log: Log) -> Result<Vec<u8>, String> {
    let total: u64 = r.header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut rd = r.into_reader().take(max);
    let (mut b, mut buf, mut step) = (vec![], [0u8; 64 << 10], 1u64);
    loop {
        let n = rd.read(&mut buf).map_err(|e| format!("загрузка прервалась на {}: {e}", fmt_bytes(b.len() as u64)))?;
        if n == 0 {
            return Ok(b);
        }
        b.extend_from_slice(&buf[..n]);
        if total > 0 && b.len() as u64 * 10 >= total * step {
            log(&format!("  {}% ({} из {})", step * 10, fmt_bytes(b.len() as u64), fmt_bytes(total)));
            step += 1;
        }
    }
}

fn parse_userinfo(h: &str) -> SubInfo {
    let mut i = SubInfo::default();
    for part in h.split(';') {
        let Some((k, v)) = part.trim().split_once('=') else { continue };
        let n: f64 = v.trim().parse().unwrap_or(0.0);
        match k.trim() {
            "upload" => i.upload = n as u64,
            "download" => i.download = n as u64,
            "total" => i.total = n as u64,
            "expire" => i.expire = n as i64,
            _ => {}
        }
    }
    i
}

/// Скачать подписку, понять формат, сохранить профиль. Меняет sub на месте.
fn fetch_sub(sub: &mut Sub, c: &Config) -> Result<ProfileChange, String> {
    let previous_kind = sub.kind.clone();
    let r = get_subscription(&sub.url, 30, c.vpn_port)?;
    if let Some(h) = r.header("subscription-userinfo") {
        sub.info = Some(parse_userinfo(h));
    }
    if let Some(h) = r.header("profile-update-interval").and_then(|v| v.trim().parse::<i64>().ok()) {
        sub.interval_h = h.max(1);
    }
    sub.name = sanitize_profile_name(&sub.name);
    if sub.name.is_empty() {
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
        sub.name = title.or(file).unwrap_or_else(|| sanitize_profile_name(&host_of(&mask_url(&sub.url))));
    }
    let body = String::from_utf8_lossy(&read_limited(r, 32 << 20)?).into_owned();
    let (kind, nodes) = classify_sub(&body)?;
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
    let parent = path.parent().ok_or_else(|| format!("{}: нет каталога профилей", path.display()))?;
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
    let dir = path.parent().ok_or_else(|| format!("{}: нет каталога профилей", path.display()))?;
    for _ in 0..8 {
        let n = PROFILE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = dir.join(format!(".upd-profile-{}-{n}.tmp", std::process::id()));
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
                return Err("временный профиль отличается от проверенного ответа подписки".into());
            }
            sync_profile_dir(path)
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
        return Ok(temp);
    }
    Err(format!("{}: не удалось создать временный профиль", path.display()))
}

fn install_profile(temp: &Path, target: &Path, old_extension: Option<PathBuf>) -> Result<ProfileChange, String> {
    let previous_target = match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_file() => {
            let parent = target.parent().ok_or_else(|| format!("{}: нет каталога профилей", target.display()))?;
            let mut backup = None;
            // Hard link keeps the old bytes available for rollback without another data write.
            for _ in 0..8 {
                let n = PROFILE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
                let candidate = parent.join(format!(".upd-profile-backup-{}-{n}.tmp", std::process::id()));
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
                    Err(e) => return Err(format!("{}: не удалось сохранить прежний профиль: {e}", target.display())),
                }
            }
            Some(backup.ok_or_else(|| format!("{}: не удалось создать резервную ссылку профиля", target.display()))?)
        }
        Ok(_) => return Err(format!("{}: ожидался обычный файл профиля", target.display())),
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
            Some(backup) => fs::rename(backup, target).map_err(|rollback| format!("{}: {rollback}; прежний профиль сохранён в {}", target.display(), backup.display())),
            None => remove_profile_file(target),
        };
        return match rollback {
            Ok(()) => Err(e),
            Err(rollback) => Err(format!("{e}; не удалось вернуть прежний профиль: {rollback}")),
        };
    }
    Ok(ProfileChange { target: target.to_path_buf(), previous_target, old_extension })
}

fn rollback_profile_change(change: &ProfileChange) -> Result<(), String> {
    match &change.previous_target {
        Some(backup) => {
            fs::rename(backup, &change.target).map_err(|e| format!("{}: {e}; прежний профиль сохранён в {}", change.target.display(), backup.display()))?;
            sync_profile_dir(&change.target)
        }
        None => remove_profile_file(&change.target),
    }
}

fn finish_profile_change(change: ProfileChange, log: Log) {
    if let Some(old) = &change.old_extension {
        if old != &change.target {
            if let Err(e) = remove_profile_file(old) {
                log(&format!("VPN: старый профиль оставлен для очистки: {e}"));
            }
        }
    }
    if let Some(backup) = &change.previous_target {
        if let Err(e) = remove_profile_file(backup) {
            log(&format!("VPN: резервный профиль оставлен для очистки: {e}"));
        }
    }
}

fn classify_sub(body: &str) -> Result<(String, usize), String> {
    if let Ok(Value::Mapping(m)) = serde_yaml::from_str::<Value>(body) {
        let proxies = m.get("proxies").and_then(Value::as_sequence).map(|s| s.len()).unwrap_or(0);
        if proxies > 0 || m.contains_key("proxy-providers") {
            return Ok(("clash".into(), proxies));
        }
    }
    let count = |t: &str| t.lines().filter(|l| l.contains("://")).count();
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
    Err("ответ не похож на подписку (нет ни конфига Clash, ни ссылок vless://, ss://…)".into())
}

pub fn add_sub(url: &str, name: &str, c: &Config, log: Log) -> Result<(), String> {
    let _lock = subscriptions_lock(true)?;
    subscription_url(url)?;
    let mut subs = load_subs()?;
    if subs.list.iter().any(|s| s.url == url) {
        return Err("такая подписка уже есть".into());
    }
    let id = format!("{:x}", now()) + &sha1_smol::Sha1::from(url).digest().to_string()[..6];
    let mut sub = Sub { id: id.clone(), name: sanitize_profile_name(name), url: url.to_string(), interval_h: c.vpn_sub_update_h, ..Default::default() };
    log(&format!("скачиваю подписку {}...", mask_url(url)));
    let change = fetch_sub(&mut sub, c)?;
    log(&format!("«{}»: серверов {}, формат {}", sub.name, sub.nodes, sub.kind));
    subs.list.push(sub);
    if subs.active.is_empty() || !subs.list.iter().any(|s| s.id == subs.active) {
        subs.active = id;
    }
    if let Err(e) = save_subs(&subs) {
        return match rollback_profile_change(&change) {
            Ok(()) => Err(e),
            Err(rollback) => Err(format!("{e}; профиль не удалось вернуть: {rollback}")),
        };
    }
    finish_profile_change(change, log);
    Ok(())
}

pub fn delete_sub(idx: usize) -> Result<String, String> {
    let _lock = subscriptions_lock(true)?;
    let mut subs = load_subs()?;
    if idx >= subs.list.len() {
        return Err("нет такой подписки".into());
    }
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

pub fn use_sub(idx: usize) -> Result<String, String> {
    let _lock = subscriptions_lock(true)?;
    let mut subs = load_subs()?;
    let s = subs.list.get(idx).ok_or("нет такой подписки")?.clone();
    subs.active = s.id;
    save_subs(&subs)?;
    Ok(s.name)
}

/// Обновить подписки: все (force) или те, у которых подошёл срок. true — активная изменилась.
pub fn update_subs(c: &Config, log: Log, force: bool) -> Result<bool, String> {
    let _lock = subscriptions_lock(true)?;
    update_subs_locked(c, log, force)
}

fn update_subs_locked(c: &Config, log: Log, force: bool) -> Result<bool, String> {
    let mut subs = load_subs()?;
    let mut active_changed = false;
    let mut profile_changes = Vec::new();
    for s in subs.list.iter_mut() {
        let interval = if s.interval_h > 0 { s.interval_h } else { c.vpn_sub_update_h };
        if !force && now() - s.updated < interval * 3600 {
            continue;
        }
        let previous = s.clone();
        match fetch_sub(s, c) {
            Ok(change) => {
                profile_changes.push(change);
                log(&format!("подписка «{}»: обновлена, серверов {}", s.name, s.nodes));
                active_changed |= s.id == subs.active;
            }
            Err(e) => {
                *s = previous;
                s.error = e.clone();
                log(&format!("подписка «{}»: {e}", s.name));
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
        return Err(format!("{e}; не удалось вернуть прежние профили: {}", rollback_errors.join("; ")));
    }
    for change in profile_changes {
        finish_profile_change(change, log);
    }
    Ok(active_changed)
}

// ======================= публичное состояние (без секретов) =======================

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SubPub {
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

pub fn secret() -> Result<String, String> {
    let p = format!("{}/secret", etc());
    match fs::read_to_string(&p) {
        Ok(s) => {
            let s = s.trim();
            if s.len() < 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("{p}: некорректный формат секрета"));
            }
            Ok(s.to_string())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut b = [0u8; 16];
            fs::File::open("/dev/urandom")
                .and_then(|mut f| f.read_exact(&mut b))
                .map_err(|e| format!("/dev/urandom: {e}"))?;
            let s: String = b.iter().map(|x| format!("{x:02x}")).collect();
            private_dir(&etc())?;
            write_private(&p, s.as_bytes())?;
            Ok(s)
        }
        Err(e) => Err(format!("{p}: {e}")),
    }
}

fn rules_path() -> String {
    format!("{}/rules.txt", etc())
}

const RULES_TEMPLATE: &str = "# upd VPN: свои правила — идут первыми, раньше правил подписки.\n\
# Формат mihomo: ТИП,значение,куда. Куда: DIRECT (напрямую), REJECT (блок) или имя группы/сервера.\n\
# Примеры:\n\
# DOMAIN-SUFFIX,mirror.yandex.ru,DIRECT\n\
# DOMAIN-KEYWORD,torrent,DIRECT\n\
# GEOSITE,youtube,⚡ Авто\n\
# IP-CIDR,10.8.0.0/16,DIRECT,no-resolve\n";

pub fn user_rules() -> Result<Vec<String>, String> {
    let p = rules_path();
    if !Path::new(&p).exists() {
        private_dir(&etc())?;
        atomic_write(Path::new(&p), RULES_TEMPLATE.as_bytes(), 0o600).map_err(|e| format!("{p}: {e}"))?;
    }
    let text = fs::read_to_string(&p).map_err(|e| format!("{p}: {e}"))?;
    Ok(text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect())
}

pub fn edit_rules() -> Result<(), String> {
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

/// Собирает итоговый конфиг: профиль подписки + настройки upd (порты, TUN, DNS, геофайлы, правила, авто-выбор).
pub fn build_config(c: &Config) -> Result<String, String> {
    let subs = load_subs()?;
    let sub = subs.list.iter().find(|s| s.id == subs.active).ok_or("нет подписки: добавь её (upd → VPN → Подписки → n)")?;
    let body = fs::read_to_string(profile_path(sub, &sub.kind)).map_err(|_| "профиль подписки не скачан — обнови подписку".to_string())?;

    let mut m: Mapping = if sub.kind == "clash" {
        match serde_yaml::from_str::<Value>(&body) {
            Ok(Value::Mapping(m)) => m,
            _ => return Err("профиль подписки повреждён — обнови подписку".into()),
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

    // --- то, что задаёт upd поверх подписки ---
    for key in ["port", "socks-port", "redir-port", "tproxy-port", "external-ui", "external-ui-url", "external-controller-tls", "external-controller-unix", "external-controller-pipe", "interface-name", "routing-mark"] {
        m.remove(key);
    }
    m.insert(k("mixed-port"), Value::from(c.vpn_port));
    m.insert(k("allow-lan"), Value::Bool(c.vpn_allow_lan));
    m.insert(k("bind-address"), k("*"));
    m.insert(k("external-controller"), k(CONTROLLER));
    m.insert(k("secret"), k(&secret()?));
    m.insert(k("mode"), k(c.vpn_mode_name()));
    m.insert(k("log-level"), k("warning"));
    m.insert(k("ipv6"), Value::Bool(c.vpn_ipv6));
    m.insert(k("unified-delay"), Value::Bool(true));
    m.insert(k("tcp-concurrent"), Value::Bool(true));
    m.insert(k("find-process-mode"), k("off")); // экономит CPU; правила по процессам не нужны
    m.insert(k("profile"), yaml_map(vec![("store-selected", Value::Bool(true)), ("store-fake-ip", Value::Bool(true))]));
    // геофайлы — тот же источник, что у FlClash
    m.insert(k("geodata-mode"), Value::Bool(false));
    m.insert(k("geo-auto-update"), Value::Bool(true));
    m.insert(k("geo-update-interval"), Value::from(24));
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
    groups.retain(|g| g.get("name").and_then(Value::as_str) != Some(AUTO_GROUP));
    if c.vpn_auto_select {
        groups.insert(
            0,
            yaml_map(vec![
                ("name", k(AUTO_GROUP)),
                ("type", k("url-test")),
                ("include-all", Value::Bool(true)),
                // служебные «серверы» с остатком трафика и сроком — не серверы
                ("exclude-filter", k("(?i)(трафик|traffic|осталось|remain|истека|expire|срок|сайт|website|官网|剩余|到期|流量)")),
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
                        list.retain(|x| x.as_str() != Some(AUTO_GROUP));
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
                        list.retain(|x| x.as_str() != Some(AUTO_GROUP));
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

pub fn api(method: &str, path: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value, String> {
    let a = ureq::AgentBuilder::new().timeout(Duration::from_secs(12)).build(); // без прокси из окружения
    let secret = secret()?;
    let req = a.request(method, &format!("http://{CONTROLLER}{path}")).set("Authorization", &format!("Bearer {secret}"));
    let res = match body {
        Some(b) => req.set("Content-Type", "application/json").send_string(&b.to_string()),
        None => req.call(),
    };
    match res {
        Ok(r) if r.status() == 204 => Ok(serde_json::Value::Null),
        Ok(r) => {
            let s = r.into_string().map_err(|e| e.to_string())?;
            if s.trim().is_empty() {
                Ok(serde_json::Value::Null)
            } else {
                serde_json::from_str(&s).map_err(|e| e.to_string())
            }
        }
        Err(ureq::Error::Status(code, r)) => {
            let msg = r.into_string().ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).and_then(|v| v["message"].as_str().map(String::from));
            Err(msg.unwrap_or_else(|| format!("HTTP {code}")))
        }
        Err(e) => Err(e.to_string()),
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

#[derive(Clone, Debug, Default)]
pub struct Group {
    pub name: String,
    pub kind: String,
    pub now: String,
    pub all: Vec<String>,
}

#[derive(Clone, Debug, Default)]
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
}

impl Snapshot {
    /// Цепочка от главной группы до реального сервера: Proxy → ⚡ Авто → 🇩🇪 DE-1
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
    let Ok(v) = api("GET", "/version", None) else { return s };
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
    if let Some(w) = flclash_running() {
        return Err(w);
    }
    reset_failed();
    run(true, &[], "systemctl", &["start", SERVICE]).map_err(|e| format!("{e}\n{}", journal_tail()))?;
    let _ = autostart(c.vpn_autostart);
    Ok(())
}

pub fn stop() -> Result<(), String> {
    run(true, &[], "systemctl", &["stop", SERVICE])
}

pub fn restart() -> Result<(), String> {
    if let Some(w) = flclash_running() {
        return Err(w);
    }
    reset_failed();
    run(true, &[], "systemctl", &["restart", SERVICE]).map_err(|e| format!("{e}\n{}", journal_tail()))
}

pub(crate) fn core_restart_failure(error: &str) -> String {
    format!("Ядро VPN обновлено на диске, но перезапуск не удался: {error}")
}

/// Последние строки журнала службы — чтобы причину сбоя было видно сразу, без journalctl.
pub fn journal_tail() -> String {
    let (s, _) = out("journalctl", &["-u", SERVICE, "-n", "8", "--no-pager", "-o", "cat"]);
    s.lines().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n")
}

pub fn autostart(on: bool) -> Result<(), String> {
    run(true, &[], "systemctl", &[if on { "enable" } else { "disable" }, SERVICE])
}

/// Два TUN одновременно не уживутся, порты тоже могут пересечься — FlClash должен быть выключен.
pub fn flclash_running() -> Option<String> {
    let rd = fs::read_dir("/proc").ok()?;
    for e in rd.flatten() {
        let comm = fs::read_to_string(e.path().join("comm")).unwrap_or_default();
        if comm.trim() == "FlClashCore" {
            return Some("запущен FlClash — закрой его (и выключи его автозапуск), иначе два VPN помешают друг другу".into());
        }
    }
    None
}

/// Применить изменения настроек: пересобрать конфиг и перезагрузить работающее ядро.
pub fn apply(c: &Config, log: Log) -> Result<(), String> {
    let changed = write_config(c)?;
    if !service_active() {
        log("конфиг собран; VPN не запущен");
        return Ok(());
    }
    if changed {
        // TUN включается/выключается надёжнее перезапуском, остальное — горячей перезагрузкой
        let tun_now = snapshot().tun;
        if tun_now != c.vpn_tun {
            restart()?;
            log("VPN перезапущен");
        } else {
            reload()?;
            log("конфиг перезагружен");
        }
    } else {
        log("конфиг не изменился");
    }
    sysproxy(c);
    Ok(())
}

/// Режим «только прокси»: включить системный прокси GNOME пользователю; в TUN — выключить.
pub fn sysproxy(c: &Config) {
    let Some(user) = invoking_user() else { return };
    if !have("gsettings") {
        return;
    }
    let uid = out("id", &["-u", &user]).0.trim().to_string();
    if uid.is_empty() {
        return;
    }
    let bus = format!("DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/{uid}/bus");
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
        let mut args: Vec<&str> = vec!["-u", &user, "--", "env", &bus, "gsettings", "set"];
        args.extend(a.iter().map(String::as_str));
        let _ = run(true, &[], "runuser", &args);
    }
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
    serde_json::from_str(&r.into_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
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
    log(&format!(
        "FlClash: {} · mihomo: установлен {}, последний {}",
        st.flclash_tag,
        if st.core_version.is_empty() { "—" } else { &st.core_version },
        st.core_latest
    ));
    Ok(st)
}

/// Проверка SHA-256 digest релиза mihomo до распаковки и запуска кандидата.
fn verify_core_gz_digest(raw_digest: &str, gz: &[u8]) -> Result<(), String> {
    let digest = raw_digest.strip_prefix("sha256:").ok_or("неверный формат SHA-256 digest; установка отменена")?;
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("неверный формат SHA-256 digest; установка отменена".into());
    }
    let digest = digest.to_ascii_lowercase();
    use sha2::Digest;
    let got: String = sha2::Sha256::digest(gz).iter().map(|b| format!("{b:02x}")).collect();
    if got != digest {
        return Err(format!("контрольная сумма не совпала (ожидалась {digest}, получена {got})"));
    }
    Ok(())
}

/// Скачать и поставить mihomo (сборка под процессор, проверка SHA-256). true — ядро сменилось.
pub fn core_install(c: &Config, log: Log, force: bool) -> Result<bool, String> {
    let rel = gh_latest("MetaCubeX/mihomo", c.vpn_port)?;
    let tag = rel["tag_name"].as_str().ok_or("нет tag_name в релизе mihomo")?.to_string();
    let cur = core_version();
    if !force && cur.as_deref() == Some(tag.as_str()) {
        log(&format!("ядро mihomo {tag} уже актуально"));
        return Ok(false);
    }
    let name = if cfg!(target_arch = "aarch64") { format!("mihomo-linux-arm64-{tag}.gz") } else { format!("mihomo-linux-amd64-{}-{tag}.gz", cpu_level()) };
    let asset = rel["assets"].as_array().and_then(|a| a.iter().find(|x| x["name"] == name.as_str())).ok_or(format!("в релизе нет {name}"))?;
    let raw_digest = asset["digest"].as_str().ok_or("в релизе нет SHA-256 digest; установка отменена")?;
    let url = asset["browser_download_url"].as_str().ok_or("нет ссылки на файл")?;
    log(&format!("скачиваю {name}..."));
    let gz = read_progress(get(url, 600, c.vpn_port)?, 200 << 20, log)?;
    verify_core_gz_digest(raw_digest, &gz)?;
    log("контрольная сумма SHA-256 совпала");
    let mut bin = vec![];
    flate2::read::GzDecoder::new(&gz[..]).read_to_end(&mut bin).map_err(|e| format!("распаковка: {e}"))?;
    private_dir(&format!("{}/bin", home()))?;
    let tmp = format!("{}.new", core_bin());
    atomic_write(Path::new(&tmp), &bin, 0o755).map_err(|e| e.to_string())?;
    let (ver, code) = out(&tmp, &["-v"]);
    if code != 0 {
        let _ = fs::remove_file(&tmp);
        return Err("новое ядро не запускается — оставляю прежнее".into());
    }
    fs::rename(&tmp, core_bin()).map_err(|e| e.to_string())?;
    log(&format!("ядро: {}", ver.lines().next().unwrap_or("").trim()));
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
    let key = proto_varint(data, pos).ok_or("неверная структура protobuf")?;
    let number = key >> 3;
    let wire = (key & 7) as u8;
    if number == 0 || number >= (1 << 29) {
        return Err("неверная структура protobuf".into());
    }
    let (bytes, integer) = match wire {
        0 => (&[][..], Some(proto_varint(data, pos).ok_or("неверная структура protobuf")?)),
        1 => {
            let end = pos.checked_add(8).filter(|end| *end <= data.len()).ok_or("обрезанная структура protobuf")?;
            let bytes = &data[*pos..end];
            *pos = end;
            (bytes, None)
        }
        2 => {
            let len: usize = proto_varint(data, pos).and_then(|n| n.try_into().ok()).ok_or("неверная длина protobuf")?;
            let end = pos.checked_add(len).filter(|end| *end <= data.len()).ok_or("обрезанная структура protobuf")?;
            let bytes = &data[*pos..end];
            *pos = end;
            (bytes, None)
        }
        5 => {
            let end = pos.checked_add(4).filter(|end| *end <= data.len()).ok_or("обрезанная структура protobuf")?;
            let bytes = &data[*pos..end];
            *pos = end;
            (bytes, None)
        }
        _ => return Err("неподдерживаемая структура protobuf".into()),
    };
    Ok(Some(ProtoField { number: number as u32, wire, bytes, integer }))
}

fn validate_geo_domain(data: &[u8]) -> Result<(), String> {
    let mut pos = 0;
    let mut value = false;
    while let Some(field) = next_proto_field(data, &mut pos)? {
        if field.number == 2 {
            if field.wire != 2 || field.bytes.is_empty() || std::str::from_utf8(field.bytes).is_err() {
                return Err("неверный домен в GeoSite".into());
            }
            value = true;
        }
    }
    if value { Ok(()) } else { Err("пустой домен в GeoSite".into()) }
}

fn validate_geo_cidr(data: &[u8]) -> Result<(), String> {
    let mut pos = 0;
    let mut ip = None;
    let mut prefix = None;
    while let Some(field) = next_proto_field(data, &mut pos)? {
        match field.number {
            1 => {
                if field.wire != 2 || !matches!(field.bytes.len(), 4 | 16) {
                    return Err("неверный адрес в GeoIP".into());
                }
                ip = Some(field.bytes.len());
            }
            2 => {
                if field.wire != 0 {
                    return Err("неверная маска в GeoIP".into());
                }
                prefix = field.integer;
            }
            _ => {}
        }
    }
    let Some(ip_len) = ip else { return Err("в GeoIP нет адреса".into()) };
    if prefix.map(|n| n > (ip_len * 8) as u64).unwrap_or(false) {
        return Err("неверная маска в GeoIP".into());
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
            return Err("неверная запись GeoIP/GeoSite".into());
        }
        let mut entry_pos = 0;
        let mut country = false;
        let mut values = 0;
        while let Some(entry) = next_proto_field(field.bytes, &mut entry_pos)? {
            match entry.number {
                1 => {
                    if entry.wire != 2 || entry.bytes.is_empty() || std::str::from_utf8(entry.bytes).is_err() {
                        return Err("неверный код GeoIP/GeoSite".into());
                    }
                    country = true;
                }
                2 => {
                    if entry.wire != 2 {
                        return Err("неверная запись GeoIP/GeoSite".into());
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
            return Err("неполная запись GeoIP/GeoSite".into());
        }
        entries += 1;
    }
    if entries == 0 {
        return Err("в GeoIP/GeoSite нет записей".into());
    }
    Ok(())
}

fn mmdb_size(data: &[u8], pos: &mut usize, code: u8) -> Result<usize, String> {
    let (extra, base) = match code {
        0..=28 => return Ok(code as usize),
        29 => (1, 29usize),
        30 => (2, 285usize),
        31 => (3, 65_821usize),
        _ => return Err("неверный размер поля MaxMind DB".into()),
    };
    let end = pos.checked_add(extra).filter(|end| *end <= data.len()).ok_or("обрезанные метаданные MaxMind DB")?;
    let mut value = 0usize;
    for byte in &data[*pos..end] {
        value = (value << 8) | *byte as usize;
    }
    *pos = end;
    base.checked_add(value).ok_or_else(|| "неверный размер поля MaxMind DB".into())
}

fn mmdb_skip_value(data: &[u8], pos: &mut usize, depth: usize) -> Result<u8, String> {
    if depth > 32 {
        return Err("слишком глубокие метаданные MaxMind DB".into());
    }
    let control = *data.get(*pos).ok_or("обрезанные метаданные MaxMind DB")?;
    *pos += 1;
    let mut kind = control >> 5;
    let size_code = control & 0x1f;
    if kind == 0 {
        kind = data.get(*pos).copied().and_then(|n| n.checked_add(7)).ok_or("обрезанные метаданные MaxMind DB")?;
        *pos += 1;
        if !(8..=15).contains(&kind) {
            return Err("неизвестный тип метаданных MaxMind DB".into());
        }
    }
    if kind == 1 {
        let width = ((size_code >> 3) + 1) as usize;
        let end = pos.checked_add(width).filter(|end| *end <= data.len()).ok_or("обрезанный указатель MaxMind DB")?;
        *pos = end;
        return Ok(kind);
    }
    let size = mmdb_size(data, pos, size_code)?;
    match kind {
        2 => {
            let end = pos.checked_add(size).filter(|end| *end <= data.len()).ok_or("обрезанная строка MaxMind DB")?;
            std::str::from_utf8(&data[*pos..end]).map_err(|_| "неверная строка метаданных MaxMind DB")?;
            *pos = end;
        }
        3 | 15 => {
            let expected = if kind == 3 { 8 } else { 4 };
            if size != expected {
                return Err("неверный размер числа MaxMind DB".into());
            }
            let end = pos.checked_add(size).filter(|end| *end <= data.len()).ok_or("обрезанное число MaxMind DB")?;
            *pos = end;
        }
        4 => {
            let end = pos.checked_add(size).filter(|end| *end <= data.len()).ok_or("обрезанные байты MaxMind DB")?;
            *pos = end;
        }
        5 | 6 | 8 | 9 | 10 => {
            let max = match kind { 5 => 2, 6 | 8 => 4, 9 => 8, _ => 16 };
            if size > max {
                return Err("неверный размер целого MaxMind DB".into());
            }
            let end = pos.checked_add(size).filter(|end| *end <= data.len()).ok_or("обрезанное целое MaxMind DB")?;
            *pos = end;
        }
        7 => {
            if size > data.len().saturating_sub(*pos) / 2 {
                return Err("обрезанная карта метаданных MaxMind DB".into());
            }
            for _ in 0..size {
                if mmdb_skip_value(data, pos, depth + 1)? != 2 {
                    return Err("ключ метаданных MaxMind DB не является строкой".into());
                }
                mmdb_skip_value(data, pos, depth + 1)?;
            }
        }
        11 => {
            if size > data.len().saturating_sub(*pos) {
                return Err("обрезанный список метаданных MaxMind DB".into());
            }
            for _ in 0..size {
                mmdb_skip_value(data, pos, depth + 1)?;
            }
        }
        14 if size <= 1 => {}
        12 | 13 | 14 => return Err("неподдерживаемый тип метаданных MaxMind DB".into()),
        _ => return Err("неизвестный тип метаданных MaxMind DB".into()),
    }
    Ok(kind)
}

fn validate_mmdb(data: &[u8]) -> Result<(), String> {
    const MARKER: &[u8] = b"\xab\xcd\xefMaxMind.com";
    let start = data.len().saturating_sub((128 << 10) + MARKER.len());
    let Some(relative) = data[start..].windows(MARKER.len()).rposition(|w| w == MARKER) else {
        return Err("нет сигнатуры MaxMind DB".into());
    };
    let metadata = &data[start + relative + MARKER.len()..];
    if metadata.is_empty() || metadata[0] >> 5 != 7 || metadata[0] & 0x1f == 0 {
        return Err("повреждённый раздел метаданных MaxMind DB".into());
    }
    let mut pos = 0;
    if mmdb_skip_value(metadata, &mut pos, 0)? != 7 || pos != metadata.len() {
        return Err("повреждённый раздел метаданных MaxMind DB".into());
    }
    Ok(())
}

fn validate_geo_file(remote: &str, data: &[u8]) -> Result<(), String> {
    if data.len() <= 1024 {
        return Err("файл слишком мал".into());
    }
    match remote {
        "geoip.metadb" | "ASN.mmdb" => validate_mmdb(data),
        "GeoSite.dat" => validate_geo_dat(data, true),
        "GeoIP.dat" => validate_geo_dat(data, false),
        _ => Err("неизвестный формат геофайла".into()),
    }
}

pub fn geo_update(c: &Config, log: Log, only_missing: bool) -> Result<(), String> {
    private_dir(&home())?;
    let mut errs = vec![];
    for (file, remote) in GEO {
        let path = format!("{}/{file}", home());
        if only_missing && Path::new(&path).exists() {
            continue;
        }
        log(&format!("геофайл {remote}..."));
        match get(&format!("{GEO_BASE}/{remote}"), 300, c.vpn_port)
            .and_then(|r| {
                if r.status() != 200 {
                    return Err(format!("HTTP {} вместо полного файла", r.status()));
                }
                read_limited(r, 100 << 20)
            })
            .and_then(|b| validate_geo_file(remote, &b).map(|()| b))
        {
            Ok(b) => {
                if let Err(e) = atomic_write(Path::new(&path), &b, 0o644) {
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
        log("ядра ещё нет — ставлю mihomo");
        core_install(c, log, true)?;
    }
    if let Err(e) = geo_update(c, log, true) {
        log(&format!("геофайлы не скачались ({e}) — mihomo попробует сам"));
    }
    Ok(())
}

/// Вызывается службой перед стартом ядра (ExecStartPre).
pub fn prepare(c: &Config, log: Log) -> Result<(), String> {
    if let Some(w) = flclash_running() {
        return Err(w);
    }
    fetch_missing(c, log)?;
    write_config(c)?;
    let (msg, code) = out(&core_bin(), &["-t", "-d", &home(), "-f", &config_path()]);
    if code != 0 {
        return Err(format!("mihomo не принял конфиг: {}", last_line(&msg).unwrap_or("")));
    }
    log("конфиг проверен");
    Ok(())
}

/// Фоновое обслуживание (из upd auto): подписки по сроку, ядро по сигналу FlClash.
pub fn maintain(c: &Config, log: Log) {
    {
        let _lock = match subscriptions_lock(true) {
            Ok(lock) => lock,
            Err(e) => {
                log(&format!("VPN: блокировка подписок не получена: {e}"));
                return;
            }
        };
        let subs = match load_subs() {
            Ok(subs) => subs,
            Err(e) => {
                log(&format!("VPN: подписки не прочитаны: {e}"));
                return;
            }
        };
        if subs.list.is_empty() {
            return;
        }
        match update_subs_locked(c, log, false) {
            Ok(true) => match apply(c, log) {
                Ok(()) => {}
                Err(e) => log(&format!("VPN: {e}")),
            },
            Ok(false) => {}
            Err(e) => log(&format!("VPN: подписки не обновлены: {e}")),
        }
    }
    let st = load_state();
    if now() - st.checked < c.vpn_core_check_h * 3600 && core_version().is_some() {
        return;
    }
    let Ok(st) = core_check(c, log) else { return };
    let signal = st.flclash_tag != st.flclash_applied;
    if signal || core_version().is_none() {
        log(&format!("новый релиз FlClash {} — обновляю ядро mihomo", st.flclash_tag));
        match core_install(c, log, false) {
            Ok(changed) => {
                let active = service_active();
                let restart_result = if changed && active { restart() } else { Ok(()) };
                let mut s2 = load_state();
                if let Err(e) = restart_result {
                    s2.event = core_restart_failure(&e);
                    s2.event_time = now();
                    log(&s2.event);
                } else {
                    s2.flclash_applied = st.flclash_tag.clone();
                    if changed {
                        s2.event = if active {
                            format!("Ядро VPN обновлено до {} (FlClash {})", s2.core_version, st.flclash_tag)
                        } else {
                            format!("Ядро VPN обновлено на диске до {} (FlClash {}); служба VPN не запущена", s2.core_version, st.flclash_tag)
                        };
                        s2.event_time = now();
                    }
                }
                let _ = save_json("vpn.json", &s2);
            }
            Err(e) => log(&format!("ядро не обновилось: {e}")),
        }
    }
}

#[cfg(test)]
mod contract_tests {
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
            let base = std::env::temp_dir().join(format!("upd-vpn-test-{}-{}", std::process::id(), now()));
            let etc = base.join("etc");
            let home = base.join("home");
            let state = base.join("state");
            fs::create_dir_all(&etc).unwrap();
            fs::create_dir_all(&home).unwrap();
            fs::create_dir_all(&state).unwrap();
            let mut g = Self::set_vars(_iso, [("UPD_VPN_ETC", etc.to_str().unwrap()), ("UPD_VPN_HOME", home.to_str().unwrap()), ("UPD_STATE_DIR", state.to_str().unwrap())]);
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
    fn sec06_rejects_short_existing_secret() {
        let _g = EnvGuard::vpn_dirs();
        let secret_path = format!("{}/secret", etc());
        write_private(&secret_path, b"short").unwrap();
        assert!(secret().is_err());
    }

    // --- DATA-02 ---
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

    // --- DATA-01 ---
    #[test]
    fn data01_subscriptions_lock_serializes() {
        let _g = EnvGuard::vpn_dirs();
        let l1 = subscriptions_lock(false).unwrap();
        assert!(subscriptions_lock(false).is_err());
        drop(l1);
        assert!(subscriptions_lock(false).is_ok());
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
        let name = delete_sub(0).unwrap();
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
        delete_sub(0).unwrap();
        let on_disk = fs::read_to_string(&subs_path).unwrap();
        assert!(on_disk.contains("\"list\":[]") || on_disk.contains("\"list\": []"));
    }
}
