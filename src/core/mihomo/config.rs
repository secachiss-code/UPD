//! Profile config for one mihomo worker.
//!
//! Listeners come only from [`crate::core::worker::generate`]. This module adds
//! `type` afterwards because mihomo 1.19.32 rejects a listener that has no type,
//! while `worker::generate` itself must keep returning a listener without one.
//!
//! Legacy `default-nameserver` includes `"system"`. A worker has no host resolver
//! claim, so that token is omitted. `1.1.1.1` and `77.88.8.8` stay.

use crate::profiles::{DnsPolicy, FakeIpPolicy, Id, Ipv6Policy, NodeSelection, Store};
use crate::sources::{ArtifactError, read_source_artifact};
use serde_json::{Map, Value};

const ALLOWED_TYPES: &[&str] = &[
    "ss",
    "vless",
    "vmess",
    "trojan",
    "hysteria2",
    "tuic",
    "http",
    "socks5",
    "wireguard",
];

/// Node keys that send traffic around CM routing or through another outbound.
/// The I03 parser never emits them; raw callers are refused here as well.
const HOST_ESCAPE_KEYS: &[&str] = &["interface-name", "routing-mark", "dialer-proxy"];

/// Fixed failure classes. Display and Debug are phrases with no node payload.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum ConfigError {
    Empty,
    InvalidNode,
    Forbidden,
    InvalidRule,
}

impl ConfigError {
    fn phrase(self) -> &'static str {
        match self {
            Self::Empty => "proxy list is empty",
            Self::InvalidNode => "proxy node is invalid",
            Self::Forbidden => "worker config is forbidden",
            Self::InvalidRule => "user rule is invalid",
        }
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.phrase())
    }
}

impl std::fmt::Debug for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.phrase())
    }
}

impl std::error::Error for ConfigError {}

/// Build the profile document without listeners.
pub fn profile_document(
    proxies: &[Value],
    dns: &DnsPolicy,
    user_rules: &[&str],
    auto_group: bool,
) -> Result<Value, ConfigError> {
    if proxies.is_empty() {
        return Err(ConfigError::Empty);
    }
    let mut names = Vec::with_capacity(proxies.len());
    for proxy in proxies {
        check_proxy(proxy)?;
        let name = proxy
            .get("name")
            .and_then(Value::as_str)
            .ok_or(ConfigError::InvalidNode)?;
        names.push(name);
    }
    let mut rules = Vec::with_capacity(user_rules.len().saturating_add(1));
    for rule in user_rules {
        check_rule(rule)?;
        rules.push(Value::String((*rule).to_owned()));
    }
    if !rules
        .iter()
        .any(|rule| rule.as_str().is_some_and(|text| text.starts_with("MATCH,")))
    {
        let target = if auto_group {
            crate::vpn::AUTO_GROUP
        } else {
            "DIRECT"
        };
        rules.push(Value::String(format!("MATCH,{target}")));
    }
    let ipv6 = dns.ipv6 == Ipv6Policy::Pass;
    Ok(serde_json::json!({
        "mode": "rule",
        "log-level": "warning",
        "ipv6": ipv6,
        "proxies": proxies,
        "proxy-groups": [proxy_group(&names, auto_group)],
        "rules": rules,
        "dns": dns_object(dns, ipv6),
    }))
}

/// Checked proxies, DNS policy, and user rules, then the leased loopback listener.
pub fn generate_config(
    proxies: &[Value],
    dns: &DnsPolicy,
    leased_port: u16,
    user_rules: &[&str],
    auto_group: bool,
) -> Result<Vec<u8>, ConfigError> {
    let document = profile_document(proxies, dns, user_rules, auto_group)?;
    attach_worker_listeners(&document, leased_port)
}

/// Pass `document` through `worker::generate` and mark the single listener `mixed`.
///
/// A nested `auto-route` or a top-level host listener key is [`ConfigError::Forbidden`].
/// Those keys are not stripped.
pub fn attach_worker_listeners(document: &Value, leased_port: u16) -> Result<Vec<u8>, ConfigError> {
    let generated =
        crate::core::worker::generate(document, leased_port).map_err(|_| ConfigError::Forbidden)?;
    let generated: Value = serde_json::from_str(&generated).map_err(|_| ConfigError::Forbidden)?;
    let mut listeners = match generated.get("listeners").and_then(Value::as_array) {
        Some(items) if items.len() == 1 => items.clone(),
        _ => return Err(ConfigError::Forbidden),
    };
    let listener = listeners[0].as_object_mut().ok_or(ConfigError::Forbidden)?;
    if listener.get("listen").and_then(Value::as_str) != Some("127.0.0.1") {
        return Err(ConfigError::Forbidden);
    }
    if listener.get("port").and_then(Value::as_u64) != Some(u64::from(leased_port)) {
        return Err(ConfigError::Forbidden);
    }
    listener.insert("name".to_owned(), Value::String("cm".to_owned()));
    listener.insert("type".to_owned(), Value::String("mixed".to_owned()));
    let mut out = document.clone();
    let object = out.as_object_mut().ok_or(ConfigError::Forbidden)?;
    object.insert("listeners".to_owned(), Value::Array(listeners));
    serde_json::to_vec(&out).map_err(|_| ConfigError::Forbidden)
}

/// Read one source from `store` and generate a worker config.
///
/// `NodeSelection::Pinned` uses that node and rejects a different source id.
/// `NodeSelection::Policy` uses every `current_node_ids` entry. Store failures
/// become [`ConfigError::Empty`] or [`ConfigError::InvalidNode`] with no payload.
pub fn generate_from_store(
    store: &Store,
    source_id: &Id,
    selection: NodeSelection,
    dns: &DnsPolicy,
    leased_port: u16,
    user_rules: &[&str],
    auto_group: bool,
) -> Result<Vec<u8>, ConfigError> {
    let artifact = read_source_artifact(store, source_id).map_err(artifact_error)?;
    let snapshot = store
        .read_snapshot()
        .map_err(|_| ConfigError::InvalidNode)?;
    let source = snapshot.sources.get(source_id).ok_or(ConfigError::Empty)?;
    let ids = match selection {
        NodeSelection::Pinned { node } => {
            if &node.source_id != source_id {
                return Err(ConfigError::InvalidNode);
            }
            vec![node.node_id]
        }
        NodeSelection::Policy { .. } => source.current_node_ids.clone(),
    };
    let mut proxies = Vec::with_capacity(ids.len());
    for id in &ids {
        let definition = artifact.definition(id).ok_or(ConfigError::InvalidNode)?;
        proxies.push(definition.full_definition().clone());
    }
    generate_config(&proxies, dns, leased_port, user_rules, auto_group)
}

fn artifact_error(error: ArtifactError) -> ConfigError {
    match error {
        ArtifactError::SourceNotFound | ArtifactError::SourceArtifactMissing => ConfigError::Empty,
        _ => ConfigError::InvalidNode,
    }
}

fn proxy_group(names: &[&str], auto_group: bool) -> Value {
    if auto_group {
        serde_json::json!({
            "name": crate::vpn::AUTO_GROUP,
            "type": "url-test",
            "proxies": names,
            "url": crate::vpn::TEST_URL,
            "interval": 300,
            "tolerance": 50,
            "timeout": 3000,
            "lazy": false,
        })
    } else {
        serde_json::json!({
            "name": crate::vpn::AUTO_GROUP,
            "type": "select",
            "proxies": names,
        })
    }
}

fn dns_object(dns: &DnsPolicy, ipv6: bool) -> Value {
    let fake_ip = dns.fake_ip == FakeIpPolicy::Allowed;
    let mut object = serde_json::json!({
        "enable": true,
        "ipv6": ipv6,
        "enhanced-mode": if fake_ip { "fake-ip" } else { "redir-host" },
        "default-nameserver": ["1.1.1.1", "77.88.8.8"],
        "proxy-server-nameserver": ["1.1.1.1", "77.88.8.8"],
        "nameserver": ["https://1.1.1.1/dns-query", "https://dns.google/dns-query"],
        "respect-rules": true,
    });
    if fake_ip {
        object["fake-ip-range"] = Value::String("198.18.0.1/16".to_owned());
        object["fake-ip-filter"] =
            serde_json::json!(["*.lan", "*.local", "+.home.arpa", "localhost.*"]);
    }
    object
}

fn check_rule(rule: &str) -> Result<(), ConfigError> {
    if rule.is_empty() || rule.chars().any(|c| c == '\n' || c.is_ascii_control()) {
        return Err(ConfigError::InvalidRule);
    }
    Ok(())
}

fn check_proxy(value: &Value) -> Result<(), ConfigError> {
    let object = value.as_object().ok_or(ConfigError::InvalidNode)?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or(ConfigError::InvalidNode)?;
    if !ALLOWED_TYPES.contains(&kind) {
        return Err(ConfigError::InvalidNode);
    }
    if HOST_ESCAPE_KEYS.iter().any(|key| object.contains_key(*key)) {
        return Err(ConfigError::Forbidden);
    }
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or(ConfigError::InvalidNode)?;
    if name.is_empty() || name.chars().any(|c| c.is_ascii_control()) {
        return Err(ConfigError::InvalidNode);
    }
    let server = object
        .get("server")
        .and_then(Value::as_str)
        .ok_or(ConfigError::InvalidNode)?;
    if server.is_empty() || server.chars().any(|c| c.is_ascii_control()) {
        return Err(ConfigError::InvalidNode);
    }
    let port = object
        .get("port")
        .and_then(Value::as_number)
        .ok_or(ConfigError::InvalidNode)?;
    if !port.is_i64() && !port.is_u64() {
        return Err(ConfigError::InvalidNode);
    }
    let port = port.as_u64().ok_or(ConfigError::InvalidNode)?;
    if !(1..=65535).contains(&port) {
        return Err(ConfigError::InvalidNode);
    }
    match kind {
        "vless" | "vmess" | "tuic" => {
            let uuid = object
                .get("uuid")
                .and_then(Value::as_str)
                .ok_or(ConfigError::InvalidNode)?;
            if !is_uuid(uuid) {
                return Err(ConfigError::InvalidNode);
            }
        }
        "ss" => {
            require_nonempty(object, "cipher")?;
            require_nonempty(object, "password")?;
        }
        "trojan" | "hysteria2" => require_nonempty(object, "password")?,
        "http" | "socks5" => {
            optional_string(object, "username")?;
            optional_string(object, "password")?;
        }
        "wireguard" => check_wireguard(object)?,
        _ => return Err(ConfigError::InvalidNode),
    }
    Ok(())
}

fn check_wireguard(object: &Map<String, Value>) -> Result<(), ConfigError> {
    require_nonempty(object, "private-key")?;
    require_nonempty(object, "ip")?;
    let peers_ok = match object.get("peers") {
        None => false,
        Some(Value::Array(items)) if !items.is_empty() => true,
        Some(Value::Array(_)) => false,
        Some(_) => return Err(ConfigError::InvalidNode),
    };
    let public_ok = match object.get("public-key") {
        None => false,
        Some(Value::String(text)) if !text.is_empty() => true,
        Some(Value::String(_)) => false,
        Some(_) => return Err(ConfigError::InvalidNode),
    };
    if !public_ok && !peers_ok {
        return Err(ConfigError::InvalidNode);
    }
    let allowed_ok = match object.get("allowed-ips") {
        None => false,
        Some(Value::Array(items)) if !items.is_empty() => true,
        Some(Value::Array(_)) => false,
        Some(_) => return Err(ConfigError::InvalidNode),
    };
    if !allowed_ok && !peers_ok {
        return Err(ConfigError::InvalidNode);
    }
    Ok(())
}

fn require_nonempty(object: &Map<String, Value>, key: &str) -> Result<(), ConfigError> {
    match object.get(key) {
        Some(Value::String(text)) if !text.is_empty() => Ok(()),
        _ => Err(ConfigError::InvalidNode),
    }
}

fn optional_string(object: &Map<String, Value>, key: &str) -> Result<(), ConfigError> {
    match object.get(key) {
        None | Some(Value::String(_)) => Ok(()),
        Some(_) => Err(ConfigError::InvalidNode),
    }
}

fn is_uuid(text: &str) -> bool {
    let mut parts = text.split('-');
    for length in [8, 4, 4, 4, 12] {
        let Some(part) = parts.next() else {
            return false;
        };
        if part.len() != length || !part.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return false;
        }
    }
    parts.next().is_none()
}
