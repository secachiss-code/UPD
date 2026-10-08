//! Strict, bounded first-pass parser for a conservative mihomo-native subset.
//!
//! Parsing is pure: this module never reads files, fetches providers, or publishes a Store
//! update. The implementation deliberately rejects native features whose complete schema
//! is not validated here.

mod common;
mod matrix;
mod opts;
mod proto;
mod secrets;
mod tls;
mod transport;

use super::yaml_guard;
use crate::profiles::{ImportOmissions, NodeProtocol, TlsVerification};
use crate::sources::artifact::{GlobalDefaults, MAX_RAW_SOURCE_BYTES, NodeDefinitionInput};
use crate::sources::capabilities::{
    CapabilityError, ImportFormat, NativeFieldDisposition, PINNED_CORE_VERSION,
    classify_native_config_field, classify_node_option, validate as validate_capability,
};
use common::{
    MAX_NAME_BYTES, build_fixture_opts, fixture_opts_definition_digest, map_capability, parse_json,
    required_host, required_port, required_string,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;

pub const MAX_NATIVE_NODES: usize = 4096;
pub const MAX_NATIVE_DEPTH: usize = 64;
pub const MAX_NATIVE_ENTRIES: usize = 1_000_000;

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
    UnsupportedProtocol {
        node_index: usize,
    },
    UnsupportedTransport {
        node_index: usize,
    },
    UnsupportedNodeFeature {
        node_index: usize,
    },
    RestrictedNodeOption {
        node_index: usize,
    },
    UnsupportedNodeField {
        node_index: usize,
    },
    DuplicateNodeName {
        node_index: usize,
    },
    InvalidNode {
        node_index: usize,
    },
    UriTooLong,
    PublisherRelay,
    PublisherGhostTarget,
    PublisherCycle,
    PublisherProvider,
    /// HTML page, provider stub or another body that is not a subscription (retryable).
    UnusableBody,
    /// More nested encoding layers than a subscription may use (terminal).
    UnsupportedEncoding,
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
            Self::UriTooLong => "share uri exceeds the length limit",
            Self::PublisherRelay => "publisher group type relay is rejected",
            Self::PublisherGhostTarget => "publisher rule target does not exist",
            Self::PublisherCycle => "publisher sub-rules contain a cycle",
            Self::PublisherProvider => "publisher rule provider is not inline",
            Self::UnusableBody => "response body is not a subscription",
            Self::UnsupportedEncoding => "subscription encoding is nested too deeply",
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
    /// What the parser accepted but did not apply (D1/D3). Publication takes it from here,
    /// never from the caller, so nothing the parser skipped can be published unannounced.
    omissions: ImportOmissions,
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

    pub fn omissions(&self) -> &ImportOmissions {
        &self.omissions
    }

    pub fn into_parts(self) -> (ImportFormat, Vec<NodeDefinitionInput>, GlobalDefaults) {
        (self.format, self.definitions, self.defaults)
    }

    /// Protocol and TLS counts plus omissions. Definitions and secrets stay private.
    pub fn counts(&self) -> SourcePreviewCounts {
        let mut protocols = protocol_slots();
        let mut tls = tls_slots();
        for definition in &self.definitions {
            add_count(&mut protocols, definition.protocol());
            add_count(&mut tls, definition.tls_verification());
        }
        SourcePreviewCounts {
            format: self.format,
            protocols,
            tls,
            omissions: self.omissions.clone(),
        }
    }
}

fn protocol_slots() -> Vec<(NodeProtocol, u32)> {
    [
        NodeProtocol::Vless,
        NodeProtocol::Vmess,
        NodeProtocol::Shadowsocks,
        NodeProtocol::Trojan,
        NodeProtocol::Socks5,
        NodeProtocol::Http,
        NodeProtocol::Hysteria2,
        NodeProtocol::Tuic,
        NodeProtocol::WireGuard,
        NodeProtocol::Other,
    ]
    .into_iter()
    .map(|protocol| (protocol, 0))
    .collect()
}

fn tls_slots() -> Vec<(TlsVerification, u32)> {
    [
        TlsVerification::NotApplicable,
        TlsVerification::Verified,
        TlsVerification::Pinned,
        TlsVerification::Disabled,
    ]
    .into_iter()
    .map(|status| (status, 0))
    .collect()
}

fn add_count<T: Copy + Eq>(slots: &mut [(T, u32)], key: T) {
    if let Some(slot) = slots.iter_mut().find(|(item, _)| *item == key) {
        slot.1 = slot.1.saturating_add(1);
    }
}

/// Safe tallies for a dry-run report. No node names, servers, or secrets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourcePreviewCounts {
    pub format: ImportFormat,
    pub protocols: Vec<(NodeProtocol, u32)>,
    pub tls: Vec<(TlsVerification, u32)>,
    pub omissions: ImportOmissions,
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
    // One leading UTF-8 BOM is an encoding marker, not content.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let parsed = match format {
        ImportFormat::MihomoJson => parse_json(text.as_bytes())?,
        ImportFormat::MihomoYaml => {
            // One saphyr event stream is both checked and loaded (no second YAML parser).
            yaml_guard::load(text.as_bytes()).map_err(map_yaml_guard_error)?
        }
        _ => return Err(ParserError::UnsupportedFormat),
    };
    let mut source = build_source(core_version, format, parsed)?;
    source.body_digest_sha256 = sha256_hex(bytes);
    Ok(source)
}

fn build_source(
    core_version: &str,
    format: ImportFormat,
    root: Value,
) -> Result<ParsedSource, ParserError> {
    let Value::Object(mut object) = root else {
        return Err(ParserError::InvalidTopLevel);
    };
    let had_proxies = object.contains_key("proxies");
    let mut proxies = match object.remove("proxies") {
        None => Vec::new(),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(ParserError::InvalidTopLevel),
    };

    let mut default_fields = Vec::new();
    let mut omitted_sections = Vec::new();
    let mut inline_nodes = Vec::new();
    let mut policy_groups = None;
    let mut policy_rules = None;
    let mut policy_sub = None;
    let mut policy_providers = None;
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
            Ok(NativeFieldDisposition::ProxyGroups) => policy_groups = Some(value),
            Ok(NativeFieldDisposition::Rules) => policy_rules = Some(value),
            Ok(NativeFieldDisposition::SubRules) => policy_sub = Some(value),
            Ok(NativeFieldDisposition::ProviderDeclarations) if field == "rule-providers" => {
                policy_providers = Some(value);
            }
            Ok(NativeFieldDisposition::ProviderDeclarations) => {
                collect_proxy_providers(&value, &mut omitted_sections, &mut inline_nodes)?;
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
    omitted_sections.sort();
    omitted_sections.dedup();
    proxies.extend(inline_nodes);
    if proxies.is_empty() {
        return Err(if had_proxies {
            ParserError::EmptyProxies
        } else {
            ParserError::MissingProxies
        });
    }
    if proxies.len() > MAX_NATIVE_NODES {
        return Err(ParserError::TooManyNodes);
    }
    let defaults = GlobalDefaults::new(default_fields).map_err(|_| ParserError::InvalidDefault)?;

    let mut definitions = Vec::with_capacity(proxies.len());
    let mut names = BTreeSet::new();
    let mut disabled = 0u32;
    for (node_index, proxy) in proxies.into_iter().enumerate() {
        let (name, parsed) = node_definition(core_version, node_index, proxy)?;
        if parsed.tls_verification() == TlsVerification::Disabled {
            disabled = disabled.saturating_add(1);
        }
        if !names.insert(name) {
            return Err(ParserError::DuplicateNodeName { node_index });
        }
        definitions.push(parsed);
    }

    if policy_groups.is_some()
        || policy_rules.is_some()
        || policy_sub.is_some()
        || policy_providers.is_some()
    {
        crate::core::mihomo::policy::accept_publisher(
            &names,
            policy_groups.as_ref(),
            policy_rules.as_ref(),
            policy_sub.as_ref(),
            policy_providers.as_ref(),
        )
        .map_err(policy_parser_error)?;
    }

    Ok(ParsedSource {
        format,
        definitions,
        defaults,
        body_digest_sha256: String::new(),
        omissions: ImportOmissions {
            section_names: omitted_sections,
            tls_verification_disabled_count: disabled,
            ..ImportOmissions::default()
        },
    })
}

fn policy_parser_error(error: crate::core::mihomo::policy::PolicyError) -> ParserError {
    use crate::core::mihomo::policy::PolicyError;
    match error {
        PolicyError::Relay => ParserError::PublisherRelay,
        PolicyError::GhostTarget => ParserError::PublisherGhostTarget,
        PolicyError::Cycle => ParserError::PublisherCycle,
        PolicyError::UnsupportedProvider => ParserError::PublisherProvider,
        PolicyError::GeoRule | PolicyError::Invalid => ParserError::InvalidTopLevel,
    }
}

/// Validate one proxy object exactly as the native parser does; shared with URI lists.
pub(crate) fn node_definition(
    core_version: &str,
    node_index: usize,
    proxy: Value,
) -> Result<(String, NodeDefinitionInput), ParserError> {
    let object = proxy
        .as_object()
        .ok_or(ParserError::InvalidNode { node_index })?;
    let (protocol, transport, tls_verification) = validate_proxy(core_version, node_index, object)?;
    let name = required_string(object, "name", node_index, MAX_NAME_BYTES)?.to_owned();
    let parsed =
        NodeDefinitionInput::from_trusted_classifier(core_version, protocol, transport, proxy)
            .map_err(|_| ParserError::InvalidNode { node_index })?
            .with_tls_verification(tls_verification);
    Ok((name, parsed))
}

impl ParsedSource {
    /// Assemble a parser result from already validated parts (URI lists).
    pub(crate) fn from_parts(
        format: ImportFormat,
        definitions: Vec<NodeDefinitionInput>,
        omissions: ImportOmissions,
        body: &[u8],
    ) -> Self {
        Self {
            format,
            definitions,
            defaults: GlobalDefaults::default(),
            body_digest_sha256: sha256_hex(body),
            omissions,
        }
    }
}

fn collect_proxy_providers(
    value: &Value,
    omitted: &mut Vec<String>,
    inline: &mut Vec<Value>,
) -> Result<(), ParserError> {
    let Value::Object(providers) = value else {
        return Err(ParserError::InvalidTopLevel);
    };
    let mut remote = false;
    for provider in providers.values() {
        let Some(object) = provider.as_object() else {
            return Err(ParserError::InvalidTopLevel);
        };
        match object.get("type").and_then(Value::as_str) {
            Some("inline") => match object.get("payload") {
                Some(Value::Array(nodes)) => inline.extend(nodes.iter().cloned()),
                // An inline provider without a node list would otherwise vanish silently.
                _ => return Err(ParserError::InvalidTopLevel),
            },
            Some("http" | "file") => remote = true,
            _ => return Err(ParserError::UnsupportedNativeField),
        }
    }
    if remote {
        omitted.push("proxy-providers".into());
    }
    Ok(())
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
) -> Result<
    (
        NodeProtocol,
        crate::sources::capabilities::Transport,
        TlsVerification,
    ),
    ParserError,
> {
    let type_name = required_string(object, "type", node_index, 32)?;
    let (protocol, transport) = proto::protocol_and_transport(type_name, object, node_index)?;
    validate_capability(core_version, protocol, transport)
        .map_err(|error| map_capability(error, node_index))?;

    required_string(object, "name", node_index, MAX_NAME_BYTES)?;
    let peers_present = object
        .get("peers")
        .and_then(Value::as_array)
        .is_some_and(|peers| !peers.is_empty());
    if protocol != NodeProtocol::WireGuard || !peers_present {
        required_host(object, "server", node_index)?;
    } else if let Some(server) = object.get("server") {
        let _ = server;
        required_host(object, "server", node_index)?;
    }
    required_port(object, node_index)?;
    if protocol == NodeProtocol::WireGuard
        && object
            .get("peers")
            .and_then(Value::as_array)
            .is_some_and(|peers| peers.len() > 64)
    {
        return Err(ParserError::InvalidNode { node_index });
    }

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

    let fields = proto::supported_fields(protocol);
    for field in object.keys() {
        if !fields.contains(&field.as_str()) {
            return Err(ParserError::UnsupportedNodeField { node_index });
        }
    }

    if object.contains_key("udp") {
        common::bool_value(object, "udp", node_index)?;
    }
    let tls_verification = tls::classify(
        object,
        node_index,
        protocol != NodeProtocol::WireGuard,
        tls::has_tls_hop(protocol, object),
    )?;
    proto::validate_protocol(protocol, object, node_index)?;
    matrix::check(protocol, transport, object, node_index)?;

    Ok((protocol, transport, tls_verification))
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

fn map_yaml_guard_error(error: yaml_guard::GuardError) -> ParserError {
    match error {
        yaml_guard::GuardError::Malformed => ParserError::InvalidSyntax,
        yaml_guard::GuardError::UnsupportedYaml => ParserError::YamlFeaturesRejected,
        yaml_guard::GuardError::TooDeep => ParserError::TooDeep,
        yaml_guard::GuardError::TooManyValues => ParserError::TooManyEntries,
        yaml_guard::GuardError::DuplicateKey => ParserError::DuplicateKey,
    }
}

/// Strict JSON parse used by audit fixtures for duplicate keys and depth limits.
#[doc(hidden)]
pub fn audit_parse_strict_json(bytes: &[u8]) -> Result<Value, ParserError> {
    parse_json(bytes)
}

/// Validate the `fixture-opts` schema from grok-review `I03.T04.b`.
#[doc(hidden)]
pub fn audit_validate_fixture_opts(bytes: &[u8]) -> Result<Value, ParserError> {
    let value = parse_json(bytes)?;
    let object = value
        .as_object()
        .cloned()
        .ok_or(ParserError::InvalidSyntax)?;
    build_fixture_opts(object, 0)
}

/// Canonical digest for accepted `fixture-opts` values.
#[doc(hidden)]
pub fn audit_fixture_opts_digest(value: &Value) -> String {
    fixture_opts_definition_digest(value)
}
