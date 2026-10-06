//! Read-only `cm source` commands (`V.02`).
//!
//! `import --dry-run` fetches through [`negotiate_source`] and [`FetchTransport`] and prints
//! only format, protocol counts, D1/D2/D3 omissions, the actual User-Agent and TLS tallies.
//! It does not open the profile Store and does not write legacy subscription files.
//! The subscription URL is read from stdin, from a `0600` file, or from `subs.json` as root.

use super::capabilities::PINNED_CORE_VERSION;
use super::manual::{ManualError, read_secret, read_secret_file};
use super::negotiation::{ConfiguredEndpoint, NegotiationPolicy, UserAgent};
use super::parser::SourcePreviewCounts;
use super::pipeline::{FetchSourceError, negotiate_source};
use super::transport::FetchTransport;
use crate::profiles::{
    GraphSnapshot, Id, ImportOmissions, NodeProtocol, SkippedLineClass, SourceFormat, Store,
    TlsVerification,
};
use crate::vpn::{self, Sub, Subs};
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

fn help_text() -> String {
    t!("cm source — источники подписок (только чтение)\n\n  cm source list\n  cm source show ID\n  cm source import --dry-run --url-stdin\n  cm source import --dry-run --url-file ФАЙЛ\n  cm source import --dry-run --from-legacy N\n  cm source doctor [ID]\n\nАдрес подписки не передаётся аргументом: одна строка в stdin, файл 0600 текущего пользователя или --from-legacy N (subs.json, только от root; N — номер или id:ИД).\n--dry-run ничего не записывает. doctor не меняет работающий VPN.\n").to_string()
}

const SUBSCRIPTION_AGENTS: [&str; 5] = [
    "mihomo/1.19.32",
    "ClashMeta/1.19.32",
    "Clash-Verge/2.4.2",
    "FlClash/0.8.92",
    "v2rayNG/1.8.10",
];

#[derive(Debug)]
enum SourceCliError {
    Usage,
    Unknown,
    ApplyUnsupported,
    UrlInArgv,
    MissingSource,
    ExtraArgument,
    InsecureUrlFile,
    UnreadableUrl,
    InvalidUrl,
    HttpNotLoopback,
    NotRoot,
    LegacyMissing,
    Fetch(String),
    Store(String),
    LegacyUnreadable,
    BadId,
    NotFound,
}

impl SourceCliError {
    fn code(&self) -> i32 {
        match self {
            Self::Usage
            | Self::Unknown
            | Self::ApplyUnsupported
            | Self::UrlInArgv
            | Self::MissingSource
            | Self::ExtraArgument => 2,
            _ => 1,
        }
    }
}

impl fmt::Display for SourceCliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage => f.write_str(t!("неверные аргументы cm source")),
            Self::Unknown => f.write_str(t!("неизвестная команда source")),
            Self::ApplyUnsupported => f.write_str(t!(
                "импорт без --dry-run не поддержан: запись в Store в этой команде отключена"
            )),
            Self::UrlInArgv => f.write_str(t!(
                "адрес подписки принимается через --url-stdin, --url-file или --from-legacy, не через аргументы"
            )),
            Self::MissingSource => f.write_str(t!(
                "нужен ровно один источник: --url-stdin, --url-file ФАЙЛ или --from-legacy N"
            )),
            Self::ExtraArgument => f.write_str(t!("лишний аргумент")),
            Self::InsecureUrlFile => f.write_str(t!(
                "файл адреса должен принадлежать текущему пользователю и быть не шире 0600"
            )),
            Self::UnreadableUrl => f.write_str(t!("адрес подписки не удалось прочитать")),
            Self::InvalidUrl => f.write_str(t!(
                "адрес подписки пуст, слишком длинный или содержит управляющие символы"
            )),
            Self::HttpNotLoopback => {
                f.write_str(t!("http разрешён только для 127.0.0.1, ::1 и localhost"))
            }
            Self::NotRoot => f.write_str(t!(
                "--from-legacy требует root: subs.json читается только от root"
            )),
            Self::LegacyMissing => f.write_str(t!("нет такой подписки")),
            Self::Fetch(message) => {
                write!(f, "{}", t!("не удалось прочитать источник: {0}", message))
            }
            Self::Store(message) => write!(f, "{}", t!("store: {0}", message)),
            Self::LegacyUnreadable => f.write_str(t!("subs.json не удалось прочитать")),
            Self::BadId => f.write_str(t!("неверный идентификатор источника")),
            Self::NotFound => f.write_str(t!("источник не найден")),
        }
    }
}

enum ImportSource {
    Stdin,
    File(PathBuf),
    Legacy(String),
}

/// Profile Store used by `list`, `show` and `doctor`. Tests set `CM_PROFILE_STORE`.
pub fn store_root() -> PathBuf {
    match std::env::var("CM_PROFILE_STORE") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => PathBuf::from(crate::common::state_dir()).join("profiles"),
    }
}

/// Run `cm source` arguments (without the `source` word). Prints to stdout or stderr.
pub fn dispatch(args: &[String]) -> i32 {
    match execute(args) {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(error) => {
            let code = error.code();
            eprintln!("cm: {error}");
            code
        }
    }
}

fn execute(args: &[String]) -> Result<String, SourceCliError> {
    if args.is_empty()
        || args
            .iter()
            .any(|arg| matches!(arg.as_str(), "help" | "--help" | "-h"))
    {
        return Ok(help_text());
    }
    if args.iter().any(|arg| arg.contains("://")) {
        return Err(SourceCliError::UrlInArgv);
    }
    match args[0].as_str() {
        "list" => {
            if args.len() != 1 {
                return Err(SourceCliError::ExtraArgument);
            }
            list_sources()
        }
        "show" => {
            let id = args.get(1).ok_or(SourceCliError::Usage)?;
            if args.len() != 2 {
                return Err(SourceCliError::ExtraArgument);
            }
            show_source(id)
        }
        "import" => import_dry_run(&args[1..]),
        "doctor" => {
            if args.len() > 2 {
                return Err(SourceCliError::ExtraArgument);
            }
            doctor(args.get(1).map(String::as_str))
        }
        _ => Err(SourceCliError::Unknown),
    }
}

fn list_sources() -> Result<String, SourceCliError> {
    let Some(snapshot) = open_snapshot()? else {
        return Ok(format!("{}\n", t!("источников нет")));
    };
    if snapshot.sources.is_empty() {
        return Ok(format!("{}\n", t!("источников нет")));
    }
    let mut lines = Vec::new();
    for (id, source) in &snapshot.sources {
        let format = source
            .provenance
            .as_ref()
            .map(|item| source_format_name(item.format))
            .unwrap_or_else(|| t!("неизвестен"));
        lines.push(t!(
            "id {0} · {1} · поколение {2} · узлов {3} · {4}",
            id,
            kind_name(source.kind),
            source.generation,
            source.current_node_ids.len(),
            format
        ));
    }
    lines.push(t!("VPN не изменялся").to_string());
    Ok(join_lines(&lines))
}

fn show_source(raw_id: &str) -> Result<String, SourceCliError> {
    let id = Id::new(raw_id).map_err(|_| SourceCliError::BadId)?;
    let snapshot = open_snapshot()?.ok_or(SourceCliError::NotFound)?;
    let source = snapshot.sources.get(&id).ok_or(SourceCliError::NotFound)?;
    let mut lines = vec![t!(
        "id {0} · {1} · поколение {2} · узлов {3} · {4}",
        source.id,
        kind_name(source.kind),
        source.generation,
        source.current_node_ids.len(),
        source
            .provenance
            .as_ref()
            .map(|item| source_format_name(item.format).to_string())
            .unwrap_or_else(|| t!("неизвестен").to_string())
    )];
    let tallies = tally_nodes(&snapshot, &source.current_node_ids);
    push_tallies(&mut lines, &tallies.protocols, &tallies.tls);
    match &source.provenance {
        Some(provenance) => {
            push_omissions(&mut lines, &provenance.omissions);
            let agent = if provenance.actual_user_agent_sha256.is_some() {
                t!("user-agent: только хеш")
            } else {
                t!("user-agent: нет")
            };
            lines.push(agent.to_string());
        }
        None => lines.push(t!("пропусков нет").to_string()),
    }
    lines.push(t!("VPN не изменялся").to_string());
    Ok(join_lines(&lines))
}

fn doctor(id: Option<&str>) -> Result<String, SourceCliError> {
    let body = match id {
        Some(id) => show_source(id)?,
        None => list_sources()?,
    };
    if body.contains(&t!("VPN не изменялся").to_string()) {
        return Ok(body);
    }
    Ok(format!("{body}{}\n", t!("VPN не изменялся")))
}

fn import_dry_run(args: &[String]) -> Result<String, SourceCliError> {
    if args.iter().any(|arg| arg.contains("://")) {
        return Err(SourceCliError::UrlInArgv);
    }
    let source = parse_import(args)?;
    let (label, url, preferred) = match source {
        ImportSource::Stdin => (None, read_url(io::stdin())?, None),
        ImportSource::File(path) => (None, read_url_file(&path)?, None),
        ImportSource::Legacy(spec) => {
            let sub = load_legacy(&spec)?;
            let label = Some(safe_label(&sub.name));
            let preferred = (!sub.user_agent.is_empty()).then(|| sub.user_agent.clone());
            if sub.url.is_empty() {
                return Err(SourceCliError::InvalidUrl);
            }
            (label, sub.url, preferred)
        }
    };
    let mut lines = render_preview(label.as_deref(), &preview_url(&url, preferred.as_deref())?);
    lines.push(t!("запись: нет (Store и legacy не изменялись)").to_string());
    lines.push(t!("VPN не изменялся").to_string());
    Ok(join_lines(&lines))
}

fn parse_import(args: &[String]) -> Result<ImportSource, SourceCliError> {
    let mut dry_run = false;
    let mut stdin = false;
    let mut file = None;
    let mut legacy = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dry-run" => dry_run = true,
            "--url-stdin" => stdin = true,
            "--url-file" => {
                index += 1;
                let path = args.get(index).ok_or(SourceCliError::MissingSource)?;
                if path.contains("://") {
                    return Err(SourceCliError::UrlInArgv);
                }
                if path.starts_with('-') {
                    return Err(SourceCliError::Usage);
                }
                file = Some(PathBuf::from(path));
            }
            "--from-legacy" => {
                index += 1;
                let spec = args.get(index).ok_or(SourceCliError::MissingSource)?;
                if spec.contains("://") {
                    return Err(SourceCliError::UrlInArgv);
                }
                legacy = Some(spec.clone());
            }
            _ => return Err(SourceCliError::ExtraArgument),
        }
        index += 1;
    }
    if !dry_run {
        return Err(SourceCliError::ApplyUnsupported);
    }
    match (stdin, file, legacy) {
        (true, None, None) => Ok(ImportSource::Stdin),
        (false, Some(path), None) => Ok(ImportSource::File(path)),
        (false, None, Some(spec)) => Ok(ImportSource::Legacy(spec)),
        _ => Err(SourceCliError::MissingSource),
    }
}

fn load_legacy(spec: &str) -> Result<Sub, SourceCliError> {
    if !crate::common::is_root() {
        return Err(SourceCliError::NotRoot);
    }
    let subs = vpn::load_subs().map_err(|_| SourceCliError::LegacyUnreadable)?;
    Ok(resolve_legacy(spec, &subs)?.clone())
}

fn resolve_legacy<'a>(spec: &str, subs: &'a Subs) -> Result<&'a Sub, SourceCliError> {
    if let Some(id) = spec.strip_prefix("id:") {
        if id.is_empty() {
            return Err(SourceCliError::LegacyMissing);
        }
        return subs
            .list
            .iter()
            .find(|sub| sub.id == id)
            .ok_or(SourceCliError::LegacyMissing);
    }
    let number: usize = spec.parse().map_err(|_| SourceCliError::LegacyMissing)?;
    if number == 0 {
        return Err(SourceCliError::LegacyMissing);
    }
    subs.list
        .get(number - 1)
        .ok_or(SourceCliError::LegacyMissing)
}

fn read_url(reader: impl Read) -> Result<String, SourceCliError> {
    match read_secret(reader) {
        Ok(url) => Ok(url),
        Err(ManualError::InvalidSecret) => Err(SourceCliError::InvalidUrl),
        Err(_) => Err(SourceCliError::UnreadableUrl),
    }
}

fn read_url_file(path: &Path) -> Result<String, SourceCliError> {
    match read_secret_file(path) {
        Ok(url) => Ok(url),
        Err(ManualError::InsecureCredentialFile) => Err(SourceCliError::InsecureUrlFile),
        Err(ManualError::InvalidSecret) => Err(SourceCliError::InvalidUrl),
        Err(_) => Err(SourceCliError::UnreadableUrl),
    }
}

struct Preview {
    counts: SourcePreviewCounts,
    user_agent: String,
}

fn preview_url(url: &str, preferred: Option<&str>) -> Result<Preview, SourceCliError> {
    let endpoint = endpoint_from_url(url)?;
    let agents = subscription_agents(preferred)?;
    let transport = FetchTransport::new();
    let accepted = negotiate_source(
        &endpoint,
        PINNED_CORE_VERSION,
        &agents,
        &[],
        None,
        &NegotiationPolicy::default(),
        |spec| transport.fetch(spec),
    )
    .map_err(fetch_error)?;
    let user_agent = accepted.actual_user_agent().expose_value().to_owned();
    if user_agent.is_empty() || user_agent.contains("://") {
        return Err(SourceCliError::Fetch(
            "source response body rejected".to_owned(),
        ));
    }
    Ok(Preview {
        counts: accepted.parsed().counts(),
        user_agent,
    })
}

fn fetch_error(error: FetchSourceError) -> SourceCliError {
    SourceCliError::Fetch(error.to_string())
}

fn endpoint_from_url(url: &str) -> Result<ConfiguredEndpoint, SourceCliError> {
    let parsed = url::Url::parse(url).map_err(|_| SourceCliError::InvalidUrl)?;
    let http = parsed.scheme() == "http";
    if http && !host_is_loopback(parsed.host_str()) {
        return Err(SourceCliError::HttpNotLoopback);
    }
    if !http && parsed.scheme() != "https" {
        return Err(SourceCliError::InvalidUrl);
    }
    ConfiguredEndpoint::new(url, http).map_err(|_| SourceCliError::InvalidUrl)
}

fn host_is_loopback(host: Option<&str>) -> bool {
    match host {
        Some("localhost") => true,
        Some(host) => host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback()),
        None => false,
    }
}

fn subscription_agents(preferred: Option<&str>) -> Result<Vec<UserAgent>, SourceCliError> {
    let mut values = Vec::new();
    if let Some(preferred) = preferred.filter(|value| !value.is_empty()) {
        values.push(preferred.to_owned());
    }
    for agent in SUBSCRIPTION_AGENTS {
        if !values.iter().any(|value| value == agent) {
            values.push(agent.to_owned());
        }
    }
    values
        .into_iter()
        .map(|value| UserAgent::new(value).map_err(|_| SourceCliError::InvalidUrl))
        .collect()
}

fn render_preview(label: Option<&str>, preview: &Preview) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(label) = label {
        lines.push(t!("подписка: {0}", label));
    }
    lines.push(t!("формат: {0}", format_name(preview.counts.format)));
    lines.push(t!(
        "узлов: {0}",
        preview
            .counts
            .protocols
            .iter()
            .map(|(_, count)| *count)
            .sum::<u32>()
    ));
    lines.push(t!("user-agent: {0}", preview.user_agent));
    push_tallies(&mut lines, &preview.counts.protocols, &preview.counts.tls);
    push_omissions(&mut lines, &preview.counts.omissions);
    lines
}

struct NodeTallies {
    protocols: Vec<(NodeProtocol, u32)>,
    tls: Vec<(TlsVerification, u32)>,
}

fn tally_nodes(snapshot: &GraphSnapshot, node_ids: &[Id]) -> NodeTallies {
    let mut protocols = Vec::from([
        (NodeProtocol::Vless, 0),
        (NodeProtocol::Vmess, 0),
        (NodeProtocol::Shadowsocks, 0),
        (NodeProtocol::Trojan, 0),
        (NodeProtocol::Socks5, 0),
        (NodeProtocol::Http, 0),
        (NodeProtocol::Hysteria2, 0),
        (NodeProtocol::Tuic, 0),
        (NodeProtocol::WireGuard, 0),
        (NodeProtocol::Other, 0),
    ]);
    let mut tls = Vec::from([
        (TlsVerification::NotApplicable, 0),
        (TlsVerification::Verified, 0),
        (TlsVerification::Pinned, 0),
        (TlsVerification::Disabled, 0),
    ]);
    for node_id in node_ids {
        let Some(node) = snapshot.nodes.get(node_id) else {
            continue;
        };
        add_slot(&mut protocols, node.protocol);
        add_slot(&mut tls, node.tls_verification);
    }
    NodeTallies { protocols, tls }
}

fn add_slot<T: Copy + Eq>(slots: &mut [(T, u32)], key: T) {
    if let Some(slot) = slots.iter_mut().find(|(item, _)| *item == key) {
        slot.1 = slot.1.saturating_add(1);
    }
}

fn push_tallies(
    lines: &mut Vec<String>,
    protocols: &[(NodeProtocol, u32)],
    tls: &[(TlsVerification, u32)],
) {
    for (protocol, count) in protocols {
        if *count == 0 {
            continue;
        }
        lines.push(t!("протокол {0}: {1}", protocol_name(*protocol), count));
    }
    for (status, count) in tls {
        if *count == 0 {
            continue;
        }
        lines.push(t!("tls {0}: {1}", tls_name(*status), count));
    }
}

fn push_omissions(lines: &mut Vec<String>, omissions: &ImportOmissions) {
    if omissions.is_empty() {
        lines.push(t!("пропусков нет").to_string());
        return;
    }
    for section in &omissions.section_names {
        lines.push(t!("пропуск D1: {0}", section));
    }
    if omissions.tls_verification_disabled_count > 0 {
        lines.push(t!(
            "пропуск D2: tls_verification disabled {0}",
            omissions.tls_verification_disabled_count
        ));
    }
    for report in &omissions.skipped_lines {
        let numbers = report
            .line_numbers
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join(",");
        lines.push(t!("пропуск D3: {0} {1}", skip_name(report.class), numbers));
    }
}

fn open_snapshot() -> Result<Option<GraphSnapshot>, SourceCliError> {
    let root = store_root();
    if !root.exists() {
        return Ok(None);
    }
    match Store::open(&root) {
        Ok(store) => store
            .read_snapshot()
            .map(Some)
            .map_err(|error| SourceCliError::Store(error.to_string())),
        Err(error) => Err(SourceCliError::Store(error.to_string())),
    }
}

fn safe_label(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.contains("://") || trimmed.chars().any(|c| c.is_control()) {
        t!("имя скрыто").to_string()
    } else {
        trimmed.to_string()
    }
}

fn join_lines(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn kind_name(kind: crate::profiles::SourceKind) -> &'static str {
    match kind {
        crate::profiles::SourceKind::Subscription => "subscription",
        crate::profiles::SourceKind::ManualServer => "manual_server",
    }
}

fn source_format_name(format: SourceFormat) -> &'static str {
    match format {
        SourceFormat::UriList => "uri_list",
        SourceFormat::Base64UriList => "base64_uri_list",
        SourceFormat::MihomoYaml => "mihomo_yaml",
        SourceFormat::MihomoJson => "mihomo_json",
        SourceFormat::LocalDefinition => "local_definition",
    }
}

fn format_name(format: super::capabilities::ImportFormat) -> &'static str {
    match format {
        super::capabilities::ImportFormat::UriList => "uri_list",
        super::capabilities::ImportFormat::Base64UriList => "base64_uri_list",
        super::capabilities::ImportFormat::MihomoYaml => "mihomo_yaml",
        super::capabilities::ImportFormat::MihomoJson => "mihomo_json",
    }
}

fn protocol_name(protocol: NodeProtocol) -> &'static str {
    match protocol {
        NodeProtocol::Vless => "vless",
        NodeProtocol::Vmess => "vmess",
        NodeProtocol::Shadowsocks => "shadowsocks",
        NodeProtocol::Trojan => "trojan",
        NodeProtocol::Socks5 => "socks5",
        NodeProtocol::Http => "http",
        NodeProtocol::Hysteria2 => "hysteria2",
        NodeProtocol::Tuic => "tuic",
        NodeProtocol::WireGuard => "wireguard",
        NodeProtocol::Other => "other",
    }
}

fn tls_name(status: TlsVerification) -> &'static str {
    match status {
        TlsVerification::NotApplicable => "not_applicable",
        TlsVerification::Verified => "verified",
        TlsVerification::Pinned => "pinned",
        TlsVerification::Disabled => "disabled",
    }
}

fn skip_name(class: SkippedLineClass) -> &'static str {
    match class {
        SkippedLineClass::UnsupportedScheme => "unsupported_scheme",
        SkippedLineClass::UnsupportedFeature => "unsupported_feature",
        SkippedLineClass::Malformed => "malformed",
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_legacy, safe_label};
    use crate::vpn::{Sub, Subs};

    fn sub(id: &str, name: &str, url: &str) -> Sub {
        Sub {
            id: id.to_owned(),
            name: name.to_owned(),
            url: url.to_owned(),
            ..Sub::default()
        }
    }

    #[test]
    fn legacy_selector_uses_index_or_id_and_hides_urls() {
        let secret = "https://secret.example/sub";
        let subs = Subs {
            active: "a".into(),
            list: vec![
                sub("a", "one", secret),
                sub("b", secret, "https://secret.example/other"),
            ],
        };
        assert_eq!(resolve_legacy("1", &subs).unwrap().id, "a");
        assert_eq!(resolve_legacy("id:b", &subs).unwrap().id, "b");
        assert_eq!(
            resolve_legacy("2", &subs).unwrap().url,
            "https://secret.example/other"
        );
        for spec in ["0", "9", "id:", "id:missing"] {
            let text = resolve_legacy(spec, &subs).unwrap_err().to_string();
            assert!(!text.contains("secret.example"), "{spec}: {text}");
        }
        let hidden = safe_label(secret);
        assert!(!hidden.contains("secret.example"));
        assert_eq!(safe_label("one"), "one");
    }
}
