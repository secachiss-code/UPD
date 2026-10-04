//! Strict, bounded first-pass parser for a conservative mihomo-native subset.
//!
//! Parsing is pure: this module never reads files, fetches providers, or publishes a Store
//! update. The implementation deliberately rejects native features whose complete schema
//! is not validated here.

use super::yaml_guard;
use crate::profiles::NodeProtocol;
use crate::sources::artifact::{GlobalDefaults, MAX_RAW_SOURCE_BYTES, NodeDefinitionInput};
use crate::sources::capabilities::{
    CapabilityError, ImportFormat, NativeFieldDisposition, PINNED_CORE_VERSION, Transport,
    classify_native_config_field, classify_node_option, validate as validate_capability,
};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Number, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::net::IpAddr;

pub const MAX_NATIVE_NODES: usize = 4096;
pub const MAX_NATIVE_DEPTH: usize = 64;
pub const MAX_NATIVE_ENTRIES: usize = 1_000_000;
const MAX_NAME_BYTES: usize = 128;
const MAX_SECRET_BYTES: usize = 4096;
const MAX_HOST_BYTES: usize = 253;

/// Safe parser failures. Variants carry only fixed enums and bounded node indexes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParserError {
    UnsupportedFormat,
    UnsupportedCoreVersion,
    BodyTooLarge,
    InvalidUtf8,
    InvalidSyntax,
    DuplicateKey,
    MultipleDocuments,
    YamlFeaturesRejected,
    TooDeep,
    TooManyEntries,
    InvalidTopLevel,
    MissingProxies,
    EmptyProxies,
    TooManyNodes,
    RestrictedNativeField,
    UnsupportedNativeSection,
    UnsupportedNativeField,
    InvalidDefault,
    UnsupportedProtocol { node_index: usize },
    UnsupportedTransport { node_index: usize },
    UnsupportedNodeFeature { node_index: usize },
    RestrictedNodeOption { node_index: usize },
    UnsupportedNodeField { node_index: usize },
    DuplicateNodeName { node_index: usize },
    InvalidNode { node_index: usize },
}

impl fmt::Display for ParserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedFormat => "unsupported native source format",
            Self::UnsupportedCoreVersion => "unsupported mihomo core version",
            Self::BodyTooLarge => "native source exceeds the byte limit",
            Self::InvalidUtf8 => "native source is not valid UTF-8",
            Self::InvalidSyntax => "native source syntax is invalid",
            Self::DuplicateKey => "native source contains a duplicate key",
            Self::MultipleDocuments => "native YAML must contain exactly one document",
            Self::YamlFeaturesRejected => "native YAML uses a rejected feature",
            Self::TooDeep => "native source nesting limit exceeded",
            Self::TooManyEntries => "native source entry limit exceeded",
            Self::InvalidTopLevel => "native source root must be an object",
            Self::MissingProxies => "native source has no proxy definitions",
            Self::EmptyProxies => "native source proxy list is empty",
            Self::TooManyNodes => "native source node limit exceeded",
            Self::RestrictedNativeField => "native source contains a host-control field",
            Self::UnsupportedNativeSection => {
                "native source section is outside the supported subset"
            }
            Self::UnsupportedNativeField => "native source field is unsupported",
            Self::InvalidDefault => "native source default is invalid",
            Self::UnsupportedProtocol { .. } => "native proxy protocol is unsupported",
            Self::UnsupportedTransport { .. } => "native proxy transport is unsupported",
            Self::UnsupportedNodeFeature { .. } => {
                "native proxy feature is outside the supported subset"
            }
            Self::RestrictedNodeOption { .. } => "native proxy option controls host networking",
            Self::UnsupportedNodeField { .. } => "native proxy field is unsupported",
            Self::DuplicateNodeName { .. } => "native source has duplicate proxy names",
            Self::InvalidNode { .. } => "native proxy definition is invalid",
        })
    }
}

impl std::error::Error for ParserError {}

/// Parsed, validated data ready for a later source-artifact pipeline.
///
/// Fields stay private so callers cannot manufacture parser-approved values. The definitions
/// and defaults remain secret-bearing data and the debug view intentionally omits them.
pub struct ParsedSource {
    format: ImportFormat,
    definitions: Vec<NodeDefinitionInput>,
    defaults: GlobalDefaults,
    body_digest_sha256: String,
}

impl ParsedSource {
    pub fn format(&self) -> ImportFormat {
        self.format
    }

    pub fn node_count(&self) -> usize {
        self.definitions.len()
    }

    pub fn defaults(&self) -> &GlobalDefaults {
        &self.defaults
    }

    /// Digest tying these definitions to the exact bytes parsed by this call.
    pub fn source_body_sha256(&self) -> &str {
        &self.body_digest_sha256
    }

    pub fn into_parts(self) -> (ImportFormat, Vec<NodeDefinitionInput>, GlobalDefaults) {
        (self.format, self.definitions, self.defaults)
    }
}

impl fmt::Debug for ParsedSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParsedSource")
            .field("format", &self.format)
            .field("node_count", &self.definitions.len())
            .field("defaults", &"[REDACTED]")
            .finish()
    }
}

/// Parse one bounded mihomo YAML or JSON source for the pinned core.
///
/// This initial parser implements TCP-only plain branches for VLESS, VMess, Shadowsocks,
/// HTTP, and SOCKS5. TLS, REALITY, non-TCP transports, Trojan, Hysteria2, TUIC, WireGuard,
/// nested transport options, and providers are rejected explicitly until their own complete
/// schemas and controller policy are reviewed.
pub fn parse_native(
    core_version: &str,
    format: ImportFormat,
    bytes: &[u8],
) -> Result<ParsedSource, ParserError> {
    if !matches!(format, ImportFormat::MihomoYaml | ImportFormat::MihomoJson) {
        return Err(ParserError::UnsupportedFormat);
    }
    if core_version.strip_prefix('v').unwrap_or(core_version) != PINNED_CORE_VERSION {
        return Err(ParserError::UnsupportedCoreVersion);
    }
    if bytes.len() > MAX_RAW_SOURCE_BYTES {
        return Err(ParserError::BodyTooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ParserError::InvalidUtf8)?;

    let parsed = match format {
        ImportFormat::MihomoJson => parse_json(text.as_bytes())?,
        ImportFormat::MihomoYaml => {
            yaml_guard::validate(bytes).map_err(map_yaml_guard_error)?;
            parse_yaml(text)?
        }
        _ => return Err(ParserError::UnsupportedFormat),
    };
    let mut source = build_source(core_version, format, parsed)?;
    source.body_digest_sha256 = sha256_hex(bytes);
    Ok(source)
}

#[derive(Default)]
struct ParseBudget {
    entries: usize,
    failure: Option<ParserError>,
}

impl ParseBudget {
    fn entry<E: de::Error>(&mut self) -> Result<(), E> {
        match self.entries.checked_add(1) {
            Some(value) if value <= MAX_NATIVE_ENTRIES => {
                self.entries = value;
                Ok(())
            }
            _ => {
                self.failure = Some(ParserError::TooManyEntries);
                Err(E::custom("bounded entry limit"))
            }
        }
    }

    fn fail<E: de::Error>(&mut self, error: ParserError) -> E {
        self.failure = Some(error);
        E::custom("bounded native parse rejected")
    }
}

enum StrictValue {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Sequence(Vec<StrictValue>),
    Mapping(BTreeMap<String, StrictValue>),
}

impl StrictValue {
    fn into_json(self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Bool(value) => Value::Bool(value),
            Self::Number(value) => Value::Number(value),
            Self::String(value) => Value::String(value),
            Self::Sequence(values) => {
                Value::Array(values.into_iter().map(Self::into_json).collect())
            }
            Self::Mapping(values) => Value::Object(
                values
                    .into_iter()
                    .map(|(key, value)| (key, value.into_json()))
                    .collect(),
            ),
        }
    }
}

struct StrictValueSeed<'a> {
    budget: &'a mut ParseBudget,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for StrictValueSeed<'_> {
    type Value = StrictValue;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if self.depth > MAX_NATIVE_DEPTH {
            return Err(self.budget.fail(ParserError::TooDeep));
        }
        deserializer.deserialize_any(StrictValueVisitor {
            budget: self.budget,
            depth: self.depth,
        })
    }
}

struct StrictValueVisitor<'a> {
    budget: &'a mut ParseBudget,
    depth: usize,
}

impl<'de> Visitor<'de> for StrictValueVisitor<'_> {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded JSON-compatible YAML/JSON value")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(StrictValue::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(StrictValue::Null)
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictValue::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictValue::Number(Number::from(value)))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictValue::Number(Number::from(value)))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
        Number::from_f64(value)
            .map(StrictValue::Number)
            .ok_or_else(|| E::custom("invalid native number"))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(StrictValue::String(value.to_owned()))
    }

    fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
        Ok(StrictValue::String(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictValue::String(value))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        StrictValueSeed {
            budget: self.budget,
            depth: self.depth + 1,
        }
        .deserialize(deserializer)
    }

    fn visit_newtype_struct<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        StrictValueSeed {
            budget: self.budget,
            depth: self.depth + 1,
        }
        .deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(StrictValueSeed {
            budget: self.budget,
            depth: self.depth + 1,
        })? {
            self.budget.entry::<A::Error>()?;
            values.push(value);
        }
        Ok(StrictValue::Sequence(values))
    }

    fn visit_map<A>(self, mut mapping: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = BTreeMap::new();
        while let Some(key) = mapping.next_key::<String>()? {
            self.budget.entry::<A::Error>()?;
            if values.contains_key(&key) {
                return Err(self.budget.fail(ParserError::DuplicateKey));
            }
            let value = mapping.next_value_seed(StrictValueSeed {
                budget: self.budget,
                depth: self.depth + 1,
            })?;
            values.insert(key, value);
        }
        Ok(StrictValue::Mapping(values))
    }
}

fn parse_json(bytes: &[u8]) -> Result<Value, ParserError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let mut budget = ParseBudget::default();
    let parsed = StrictValueSeed {
        budget: &mut budget,
        depth: 1,
    }
    .deserialize(&mut deserializer);
    match parsed {
        Ok(value) => {
            if deserializer.end().is_err() {
                return Err(ParserError::InvalidSyntax);
            }
            Ok(value.into_json())
        }
        Err(_) => Err(budget.failure.unwrap_or(ParserError::InvalidSyntax)),
    }
}

fn parse_yaml(text: &str) -> Result<Value, ParserError> {
    let mut documents = serde_yaml::Deserializer::from_str(text);
    let Some(document) = documents.next() else {
        return Err(ParserError::InvalidSyntax);
    };
    let mut budget = ParseBudget::default();
    let parsed = StrictValueSeed {
        budget: &mut budget,
        depth: 1,
    }
    .deserialize(document);
    let value = match parsed {
        Ok(value) => value,
        Err(_) => return Err(budget.failure.unwrap_or(ParserError::InvalidSyntax)),
    };
    if documents.next().is_some() {
        return Err(ParserError::MultipleDocuments);
    }
    Ok(value.into_json())
}

fn build_source(
    core_version: &str,
    format: ImportFormat,
    root: Value,
) -> Result<ParsedSource, ParserError> {
    let Value::Object(mut object) = root else {
        return Err(ParserError::InvalidTopLevel);
    };
    let proxy_value = object
        .remove("proxies")
        .ok_or(ParserError::MissingProxies)?;
    let Value::Array(proxies) = proxy_value else {
        return Err(ParserError::InvalidTopLevel);
    };
    if proxies.is_empty() {
        return Err(ParserError::EmptyProxies);
    }
    if proxies.len() > MAX_NATIVE_NODES {
        return Err(ParserError::TooManyNodes);
    }

    let mut default_fields = Vec::new();
    for (field, value) in object {
        match classify_native_config_field(&field) {
            Ok(NativeFieldDisposition::ConstrainedDefault) => {
                if field == "global-client-fingerprint" {
                    // Pinned v1.19.32 explicitly marks this global setting as removed.
                    return Err(ParserError::UnsupportedNativeField);
                }
                validate_default_field(&field, &value)?;
                default_fields.push((field, value));
            }
            Ok(NativeFieldDisposition::RestrictedNative) => {
                return Err(ParserError::RestrictedNativeField);
            }
            Ok(
                NativeFieldDisposition::ProxyGroups
                | NativeFieldDisposition::Rules
                | NativeFieldDisposition::SubRules
                | NativeFieldDisposition::ProviderDeclarations,
            ) => {
                return Err(ParserError::UnsupportedNativeSection);
            }
            Ok(NativeFieldDisposition::ProxyDefinitions) => {
                return Err(ParserError::InvalidTopLevel);
            }
            Err(CapabilityError::UnsupportedField) => {
                return Err(ParserError::UnsupportedNativeField);
            }
            Err(_) => return Err(ParserError::UnsupportedNativeField),
        }
    }
    let defaults = GlobalDefaults::new(default_fields).map_err(|_| ParserError::InvalidDefault)?;

    let mut definitions = Vec::with_capacity(proxies.len());
    let mut names = BTreeSet::new();
    for (node_index, proxy) in proxies.into_iter().enumerate() {
        let object = proxy
            .as_object()
            .ok_or(ParserError::InvalidNode { node_index })?;
        let (protocol, transport) = validate_proxy(core_version, node_index, object)?;
        let name = required_string(object, "name", node_index, MAX_NAME_BYTES)?.to_owned();
        if !names.insert(name) {
            return Err(ParserError::DuplicateNodeName { node_index });
        }
        let parsed =
            NodeDefinitionInput::from_trusted_classifier(core_version, protocol, transport, proxy)
                .map_err(|_| ParserError::InvalidNode { node_index })?;
        definitions.push(parsed);
    }

    Ok(ParsedSource {
        format,
        definitions,
        defaults,
        body_digest_sha256: String::new(),
    })
}

fn validate_default_field(field: &str, value: &Value) -> Result<(), ParserError> {
    let valid = match field {
        "mode" => matches!(value.as_str(), Some("rule" | "global" | "direct")),
        "log-level" => matches!(
            value.as_str(),
            Some("silent" | "error" | "warning" | "info" | "debug")
        ),
        "unified-delay" | "tcp-concurrent" => value.is_boolean(),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(ParserError::InvalidDefault)
    }
}

fn validate_proxy(
    core_version: &str,
    node_index: usize,
    object: &serde_json::Map<String, Value>,
) -> Result<(NodeProtocol, Transport), ParserError> {
    let type_name = required_string(object, "type", node_index, 32)?;
    let (protocol, transport) = match type_name {
        "vless" => {
            let transport = tcp_network(object, node_index)?;
            (NodeProtocol::Vless, transport)
        }
        "vmess" => {
            let transport = tcp_network(object, node_index)?;
            (NodeProtocol::Vmess, transport)
        }
        "ss" => (NodeProtocol::Shadowsocks, Transport::Tcp),
        "http" => (NodeProtocol::Http, Transport::Tcp),
        "socks5" => (NodeProtocol::Socks5, Transport::Tcp),
        "trojan" | "hysteria2" | "hy2" | "tuic" | "wireguard" => {
            return Err(ParserError::UnsupportedProtocol { node_index });
        }
        _ => return Err(ParserError::UnsupportedProtocol { node_index }),
    };
    validate_capability(core_version, protocol, transport)
        .map_err(|error| map_capability(error, node_index))?;

    required_string(object, "name", node_index, MAX_NAME_BYTES)?;
    required_host(object, "server", node_index)?;
    required_port(object, node_index)?;

    for field in object.keys() {
        match classify_node_option(field) {
            Ok(crate::sources::capabilities::NodeOptionDisposition::RestrictedNative) => {
                return Err(ParserError::RestrictedNodeOption { node_index });
            }
            Err(CapabilityError::UnsupportedFeature) => {
                return Err(ParserError::UnsupportedNodeFeature { node_index });
            }
            _ => {}
        }
    }

    let fields = supported_fields(protocol);
    for field in object.keys() {
        if !fields.contains(&field.as_str()) {
            return Err(ParserError::UnsupportedNodeField { node_index });
        }
    }

    if object.contains_key("udp") {
        bool_value(object, "udp", node_index)?;
    }
    if object.contains_key("tls") && bool_value(object, "tls", node_index)? {
        return Err(ParserError::UnsupportedNodeFeature { node_index });
    }

    match protocol {
        NodeProtocol::Vless => {
            validate_uuid(object, node_index)?;
            if object.contains_key("flow") {
                return Err(ParserError::UnsupportedNodeFeature { node_index });
            }
        }
        NodeProtocol::Vmess => {
            validate_uuid(object, node_index)?;
            let cipher = required_string(object, "cipher", node_index, 64)?;
            if !matches!(
                cipher,
                "auto" | "none" | "zero" | "aes-128-gcm" | "chacha20-poly1305"
            ) {
                return Err(ParserError::InvalidNode { node_index });
            }
            if let Some(alter_id) = object.get("alterId") {
                if alter_id.as_u64() != Some(0) {
                    return Err(ParserError::UnsupportedNodeFeature { node_index });
                }
            }
        }
        NodeProtocol::Shadowsocks => {
            let password = required_string(object, "password", node_index, MAX_SECRET_BYTES)?;
            if password.is_empty() || has_control(password) {
                return Err(ParserError::InvalidNode { node_index });
            }
            let cipher = required_string(object, "cipher", node_index, 64)?;
            if [
                "2022-blake3-aes-128-gcm",
                "2022-blake3-aes-256-gcm",
                "2022-blake3-chacha20-poly1305",
            ]
            .iter()
            .any(|known| cipher.eq_ignore_ascii_case(known))
            {
                return Err(ParserError::UnsupportedNodeFeature { node_index });
            }
            if !is_supported_ss_cipher(cipher) {
                return Err(ParserError::InvalidNode { node_index });
            }
        }
        NodeProtocol::Http | NodeProtocol::Socks5 => {
            let username = optional_string(object, "username", node_index, MAX_SECRET_BYTES)?;
            let password = optional_string(object, "password", node_index, MAX_SECRET_BYTES)?;
            if username.is_some() != password.is_some()
                || username.is_some_and(|value| value.is_empty() || has_control(value))
                || password.is_some_and(|value| value.is_empty() || has_control(value))
            {
                return Err(ParserError::InvalidNode { node_index });
            }
            if protocol == NodeProtocol::Http && object.contains_key("headers") {
                validate_headers(object.get("headers"), node_index)?;
            }
        }
        _ => return Err(ParserError::UnsupportedProtocol { node_index }),
    }
    Ok((protocol, transport))
}

fn supported_fields(protocol: NodeProtocol) -> &'static [&'static str] {
    match protocol {
        NodeProtocol::Vless => &[
            "name", "type", "server", "port", "uuid", "network", "udp", "tls",
        ],
        NodeProtocol::Vmess => &[
            "name", "type", "server", "port", "uuid", "cipher", "alterId", "network", "udp", "tls",
        ],
        NodeProtocol::Shadowsocks => &[
            "name", "type", "server", "port", "password", "cipher", "udp",
        ],
        NodeProtocol::Http => &[
            "name", "type", "server", "port", "username", "password", "headers", "tls",
        ],
        NodeProtocol::Socks5 => &[
            "name", "type", "server", "port", "username", "password", "udp", "tls",
        ],
        _ => &[],
    }
}

fn tcp_network(
    object: &serde_json::Map<String, Value>,
    node_index: usize,
) -> Result<Transport, ParserError> {
    match object.get("network") {
        None => Ok(Transport::Tcp),
        Some(Value::String(network)) if network == "tcp" => Ok(Transport::Tcp),
        Some(Value::String(_)) => Err(ParserError::UnsupportedTransport { node_index }),
        Some(_) => Err(ParserError::InvalidNode { node_index }),
    }
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
    node_index: usize,
    max_bytes: usize,
) -> Result<&'a str, ParserError> {
    let value = object
        .get(field)
        .and_then(Value::as_str)
        .ok_or(ParserError::InvalidNode { node_index })?;
    if value.is_empty() || value.len() > max_bytes || has_control(value) {
        return Err(ParserError::InvalidNode { node_index });
    }
    Ok(value)
}

fn optional_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
    node_index: usize,
    max_bytes: usize,
) -> Result<Option<&'a str>, ParserError> {
    match object.get(field) {
        None => Ok(None),
        Some(Value::String(value)) if value.len() <= max_bytes && !has_control(value) => {
            Ok(Some(value))
        }
        _ => Err(ParserError::InvalidNode { node_index }),
    }
}

fn required_host(
    object: &serde_json::Map<String, Value>,
    field: &str,
    node_index: usize,
) -> Result<(), ParserError> {
    let host = required_string(object, field, node_index, MAX_HOST_BYTES)?;
    if !is_host(host) {
        return Err(ParserError::InvalidNode { node_index });
    }
    Ok(())
}

fn is_host(value: &str) -> bool {
    if value.len() > MAX_HOST_BYTES
        || value.contains("//")
        || value.bytes().any(|byte| {
            byte.is_ascii_control()
                || byte.is_ascii_whitespace()
                || matches!(byte, b'/' | b'?' | b'#' | b'@' | b'%')
        })
    {
        return false;
    }
    if value.parse::<IpAddr>().is_ok() {
        return true;
    }
    let host = value.strip_suffix('.').unwrap_or(value);
    !host.is_empty()
        && host.is_ascii()
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn required_port(
    object: &serde_json::Map<String, Value>,
    node_index: usize,
) -> Result<(), ParserError> {
    if object
        .get("port")
        .and_then(Value::as_u64)
        .is_some_and(|port| (1..=u16::MAX as u64).contains(&port))
    {
        Ok(())
    } else {
        Err(ParserError::InvalidNode { node_index })
    }
}

fn bool_value(
    object: &serde_json::Map<String, Value>,
    field: &str,
    node_index: usize,
) -> Result<bool, ParserError> {
    object
        .get(field)
        .and_then(Value::as_bool)
        .ok_or(ParserError::InvalidNode { node_index })
}

fn validate_uuid(
    object: &serde_json::Map<String, Value>,
    node_index: usize,
) -> Result<(), ParserError> {
    let uuid = required_string(object, "uuid", node_index, 36)?;
    let bytes = uuid.as_bytes();
    if bytes.len() != 36
        || ![8, 13, 18, 23]
            .into_iter()
            .all(|index| bytes[index] == b'-')
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit())
    {
        return Err(ParserError::InvalidNode { node_index });
    }
    Ok(())
}

fn validate_headers(value: Option<&Value>, node_index: usize) -> Result<(), ParserError> {
    let Some(Value::Object(headers)) = value else {
        return Err(ParserError::InvalidNode { node_index });
    };
    if headers.len() > 128 {
        return Err(ParserError::InvalidNode { node_index });
    }
    let mut normalized_names = BTreeSet::new();
    for (name, value) in headers {
        if name.is_empty()
            || name.len() > 256
            || !name.bytes().all(is_http_token_byte)
            || !normalized_names.insert(name.to_ascii_lowercase())
            || value.as_str().is_none_or(|text| {
                text.len() > 8192
                    || text
                        .bytes()
                        .any(|byte| byte != b'\t' && (byte < 0x20 || byte == 0x7f))
            })
        {
            return Err(ParserError::InvalidNode { node_index });
        }
    }
    Ok(())
}

fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

fn is_supported_ss_cipher(cipher: &str) -> bool {
    [
        "aes-128-gcm",
        "aes-192-gcm",
        "aes-256-gcm",
        "chacha20-ietf-poly1305",
        "xchacha20-ietf-poly1305",
    ]
    .contains(&cipher)
}

fn has_control(value: &str) -> bool {
    value.chars().any(char::is_control)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn map_capability(error: CapabilityError, node_index: usize) -> ParserError {
    match error {
        CapabilityError::UnsupportedCoreVersion => ParserError::UnsupportedCoreVersion,
        CapabilityError::UnsupportedTransport => ParserError::UnsupportedTransport { node_index },
        CapabilityError::UnsupportedProtocol => ParserError::UnsupportedProtocol { node_index },
        _ => ParserError::UnsupportedNodeFeature { node_index },
    }
}

fn map_yaml_guard_error(error: yaml_guard::GuardError) -> ParserError {
    match error {
        yaml_guard::GuardError::Malformed => ParserError::InvalidSyntax,
        yaml_guard::GuardError::UnsupportedYaml => ParserError::YamlFeaturesRejected,
        yaml_guard::GuardError::TooDeep => ParserError::TooDeep,
        yaml_guard::GuardError::TooManyValues => ParserError::TooManyEntries,
    }
}
