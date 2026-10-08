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
    /// Store source selected by `cm vpn use source:ID`. Empty means the legacy subscription.
    #[serde(default)]
    pub store_source: String,
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
        if b[i] == b'%' && i + 2 < b.len()
            && let (Some(hi), Some(lo)) = (hex_digit(b[i + 1]), hex_digit(b[i + 2])) {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
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

