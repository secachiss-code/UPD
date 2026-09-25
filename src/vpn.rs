//! VPN на ядре mihomo (Clash.Meta — то же ядро, что внутри FlClash).
//! Подписки, сборка конфига, управление через REST API, обновление ядра по релизам FlClash, геофайлы.

use crate::common::*;
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

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

pub fn load_subs() -> Subs {
    fs::read(format!("{}/subs.json", etc())).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_subs(s: &Subs) -> Result<(), String> {
    private_dir(&etc())?;
    write_private(&format!("{}/subs.json", etc()), &serde_json::to_vec_pretty(s).unwrap_or_default())?;
    publish_state(s);
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

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn pct_encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn agent(timeout: u64, via_proxy: Option<u16>) -> ureq::Agent {
    let mut b = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout(Duration::from_secs(timeout)).user_agent(UA);
    if let Some(p) = via_proxy {
        if let Ok(px) = ureq::Proxy::new(format!("http://127.0.0.1:{p}")) {
            b = b.proxy(px);
        }
    }
    b.build()
}

/// GET напрямую; если не вышло и VPN работает — через него.
fn get(url: &str, timeout: u64, port: u16) -> Result<ureq::Response, String> {
    match agent(timeout, None).get(url).call() {
        Ok(r) => Ok(r),
        Err(e) if running() => agent(timeout, Some(port)).get(url).call().map_err(|e2| format!("напрямую: {e}; через VPN: {e2}")),
        Err(e) => Err(e.to_string()),
    }
}

fn read_limited(r: ureq::Response, max: u64) -> Result<Vec<u8>, String> {
    let mut b = vec![];
    r.into_reader().take(max).read_to_end(&mut b).map_err(|e| e.to_string())?;
    Ok(b)
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
fn fetch_sub(sub: &mut Sub, c: &Config) -> Result<(), String> {
    let r = get(&sub.url, 30, c.vpn_port)?;
    if let Some(h) = r.header("subscription-userinfo") {
        sub.info = Some(parse_userinfo(h));
    }
    if let Some(h) = r.header("profile-update-interval").and_then(|v| v.trim().parse::<i64>().ok()) {
        sub.interval_h = h.max(1);
    }
    if sub.name.is_empty() {
        let title = r.header("profile-title").map(|t| match t.strip_prefix("base64:") {
            Some(b) => b64_decode(b).map(|v| String::from_utf8_lossy(&v).into_owned()).unwrap_or_default(),
            None => t.to_string(),
        });
        let file = r.header("content-disposition").and_then(|d| {
            d.split(';').find_map(|p| {
                let p = p.trim();
                p.strip_prefix("filename*=UTF-8''").map(pct_decode).or_else(|| p.strip_prefix("filename=").map(|x| x.trim_matches('"').to_string()))
            })
        });
        sub.name = title.or(file).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| host_of(&mask_url(&sub.url)).to_string());
    }
    let body = String::from_utf8_lossy(&read_limited(r, 32 << 20)?).into_owned();
    let (kind, nodes) = classify_sub(&body)?;
    private_dir(&format!("{}/profiles", home()))?;
    for ext in ["yaml", "txt"] {
        let _ = fs::remove_file(format!("{}/profiles/{}.{ext}", home(), sub.id));
    }
    write_private(&profile_path(sub, &kind), body.as_bytes())?;
    sub.kind = kind;
    sub.nodes = nodes;
    sub.updated = now();
    sub.error.clear();
    Ok(())
}

fn profile_path(sub: &Sub, kind: &str) -> String {
    format!("{}/profiles/{}.{}", home(), sub.id, if kind == "uri" { "txt" } else { "yaml" })
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
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("нужен адрес подписки http(s)://…".into());
    }
    let mut subs = load_subs();
    if subs.list.iter().any(|s| s.url == url) {
        return Err("такая подписка уже есть".into());
    }
    let id = format!("{:x}", now()) + &sha1_smol::Sha1::from(url).digest().to_string()[..6];
    let mut sub = Sub { id: id.clone(), name: name.trim().to_string(), url: url.to_string(), interval_h: c.vpn_sub_update_h, ..Default::default() };
    log(&format!("скачиваю подписку {}...", mask_url(url)));
    fetch_sub(&mut sub, c)?;
    log(&format!("«{}»: серверов {}, формат {}", sub.name, sub.nodes, sub.kind));
    subs.list.push(sub);
    if subs.active.is_empty() || !subs.list.iter().any(|s| s.id == subs.active) {
        subs.active = id;
    }
    save_subs(&subs)
}

pub fn delete_sub(idx: usize) -> Result<String, String> {
    let mut subs = load_subs();
    if idx >= subs.list.len() {
        return Err("нет такой подписки".into());
    }
    let s = subs.list.remove(idx);
    for ext in ["yaml", "txt"] {
        let _ = fs::remove_file(format!("{}/profiles/{}.{ext}", home(), s.id));
    }
    if subs.active == s.id {
        subs.active = subs.list.first().map(|x| x.id.clone()).unwrap_or_default();
    }
    save_subs(&subs)?;
    Ok(s.name)
}

pub fn use_sub(idx: usize) -> Result<String, String> {
    let mut subs = load_subs();
    let s = subs.list.get(idx).ok_or("нет такой подписки")?.clone();
    subs.active = s.id;
    save_subs(&subs)?;
    Ok(s.name)
}

/// Обновить подписки: все (force) или те, у которых подошёл срок. true — активная изменилась.
pub fn update_subs(c: &Config, log: Log, force: bool) -> bool {
    let mut subs = load_subs();
    let mut active_changed = false;
    for s in subs.list.iter_mut() {
        let interval = if s.interval_h > 0 { s.interval_h } else { c.vpn_sub_update_h };
        if !force && now() - s.updated < interval * 3600 {
            continue;
        }
        match fetch_sub(s, c) {
            Ok(()) => {
                log(&format!("подписка «{}»: обновлена, серверов {}", s.name, s.nodes));
                active_changed |= s.id == subs.active;
            }
            Err(e) => {
                s.error = e.clone();
                log(&format!("подписка «{}»: {e}", s.name));
            }
        }
    }
    let _ = save_subs(&subs);
    active_changed
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
    load_json("vpn.json")
}

fn publish_state(s: &Subs) {
    let mut st = load_state();
    st.subs = s
        .list
        .iter()
        .map(|x| SubPub {
            name: x.name.clone(),
            host: host_of(&mask_url(&x.url)).to_string(),
            updated: x.updated,
            nodes: x.nodes,
            info: x.info.clone(),
            error: x.error.clone(),
            active: x.id == s.active,
        })
        .collect();
    let _ = save_json("vpn.json", &st);
}

// ======================= конфиг mihomo =======================

pub fn secret() -> String {
    let p = format!("{}/secret", etc());
    if let Ok(s) = fs::read_to_string(&p) {
        if s.trim().len() >= 16 {
            return s.trim().to_string();
        }
    }
    let mut b = [0u8; 16];
    let _ = fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b));
    let s: String = b.iter().map(|x| format!("{x:02x}")).collect();
    let _ = private_dir(&etc()).and_then(|_| write_private(&p, s.as_bytes()));
    s
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

pub fn user_rules() -> Vec<String> {
    let p = rules_path();
    if !Path::new(&p).exists() {
        let _ = private_dir(&etc()).and_then(|_| atomic_write(Path::new(&p), RULES_TEMPLATE.as_bytes(), 0o600).map_err(|e| e.to_string()));
    }
    fs::read_to_string(p).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect()
}

pub fn edit_rules() -> Result<(), String> {
    user_rules();
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
    let subs = load_subs();
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
    m.insert(k("secret"), k(&secret()));
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
    let mut rules: Vec<Value> = user_rules().into_iter().map(Value::String).collect();
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
    let req = a.request(method, &format!("http://{CONTROLLER}{path}")).set("Authorization", &format!("Bearer {}", secret()));
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

pub fn start(c: &Config) -> Result<(), String> {
    if let Some(w) = flclash_running() {
        return Err(w);
    }
    run(true, &[], "systemctl", &["start", SERVICE])?;
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
    run(true, &[], "systemctl", &["restart", SERVICE])
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
    let url = asset["browser_download_url"].as_str().ok_or("нет ссылки на файл")?;
    let digest = asset["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(str::to_lowercase);
    log(&format!("скачиваю {name}..."));
    let gz = read_limited(get(url, 600, c.vpn_port)?, 200 << 20)?;
    match &digest {
        Some(d) => {
            use sha2::Digest;
            let got: String = sha2::Sha256::digest(&gz).iter().map(|b| format!("{b:02x}")).collect();
            if &got != d {
                return Err(format!("контрольная сумма не совпала (ожидалась {d}, получена {got})"));
            }
            log("контрольная сумма SHA-256 совпала");
        }
        None => log("⚠ GitHub не отдал контрольную сумму — ставлю без проверки"),
    }
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

pub fn geo_update(c: &Config, log: Log, only_missing: bool) -> Result<(), String> {
    private_dir(&home())?;
    let mut errs = vec![];
    for (file, remote) in GEO {
        let path = format!("{}/{file}", home());
        if only_missing && Path::new(&path).exists() {
            continue;
        }
        log(&format!("геофайл {remote}..."));
        match get(&format!("{GEO_BASE}/{remote}"), 300, c.vpn_port).and_then(|r| read_limited(r, 100 << 20)) {
            Ok(b) if b.len() > 1024 => {
                atomic_write(Path::new(&path), &b, 0o644).map_err(|e| e.to_string())?;
            }
            Ok(_) => errs.push(format!("{remote}: пустой ответ")),
            Err(e) => errs.push(format!("{remote}: {e}")),
        }
    }
    let mut st = load_state();
    st.geo_updated = now();
    let _ = save_json("vpn.json", &st);
    if service_active() && !only_missing {
        let _ = reload();
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs.join("; "))
    }
}

// ======================= подготовка к запуску и обслуживание =======================

/// Вызывается службой перед стартом ядра (ExecStartPre).
pub fn prepare(c: &Config, log: Log) -> Result<(), String> {
    if let Some(w) = flclash_running() {
        return Err(w);
    }
    if core_version().is_none() {
        log("ядра ещё нет — ставлю mihomo");
        core_install(c, log, true)?;
    }
    if let Err(e) = geo_update(c, log, true) {
        log(&format!("геофайлы не скачались ({e}) — mihomo попробует сам"));
    }
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
    if load_subs().list.is_empty() {
        return;
    }
    if update_subs(c, log, false) {
        match apply(c, log) {
            Ok(()) => {}
            Err(e) => log(&format!("VPN: {e}")),
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
                let mut s2 = load_state();
                s2.flclash_applied = st.flclash_tag.clone();
                if changed {
                    s2.event = format!("Ядро VPN обновлено до {} (FlClash {})", s2.core_version, st.flclash_tag);
                    s2.event_time = now();
                }
                let _ = save_json("vpn.json", &s2);
                if changed && service_active() {
                    let _ = restart();
                }
            }
            Err(e) => log(&format!("ядро не обновилось: {e}")),
        }
    }
}
