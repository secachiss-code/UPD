//! Private, bounded source artifacts and provenance-aware Store publication.
//!
//! This stage accepts typed definitions from a trusted classifier. It does not parse
//! arbitrary URI/YAML input; the stricter format parser and manual entry points belong
//! to the next implementation stage.

use super::Negotiated;
use super::bounded::{BoundedJsonError, encode_json_bounded};
use super::capabilities::{
    CapabilityError, ImportFormat, PINNED_CORE_COMMIT, PINNED_CORE_VERSION, Transport,
    classify_node_option, validate,
};
use crate::profiles::store::{fresh_id, make_metadata, reserve_id};
use crate::profiles::{
    CredentialMaterial, EntityIdKind, GraphSnapshot, Id, Node, NodeProtocol, SCHEMA_VERSION,
    Source, SourceFormat, SourceKind, SourceOrigin, SourceProvenance, Store, StoreError,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

mod preflight;

pub const MAX_RAW_SOURCE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SERIALIZED_ARTIFACT_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_ARTIFACT_NODE_COUNT: usize = 4096;
pub const MAX_ARTIFACT_DEPTH: usize = 64;
const MAX_ARTIFACT_VALUE_NODES: usize = 1_000_000;
const MAX_ARTIFACT_SCHEMA_VERSION: u32 = 1;

/// Safe artifact errors. Input content, UA values, URLs, and serde diagnostics never escape.
#[derive(Debug)]
pub enum ArtifactError {
    InvalidInput,
    InvalidProvenance,
    UnsupportedCoreVersion,
    UnsupportedProtocol,
    UnsupportedTransport,
    UnsupportedFeature,
    RestrictedField,
    UnsupportedField,
    InvalidDefault,
    DuplicateDefault,
    TooManyNodes,
    TooDeep,
    RawBodyTooLarge,
    ArtifactTooLarge,
    Encoding,
    CorruptArtifact,
    SourceNotFound,
    SourceArtifactMissing,
    SameBodyPayloadChanged,
    RawBodyDigestCollision,
    SourceGenerationConflict,
    Store(StoreError),
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidInput => "invalid typed source import input",
            Self::InvalidProvenance => "invalid source provenance",
            Self::UnsupportedCoreVersion => "unsupported source artifact core pin",
            Self::UnsupportedProtocol => "unsupported node protocol",
            Self::UnsupportedTransport => "unsupported node transport",
            Self::UnsupportedFeature => "known node feature is outside the supported subset",
            Self::RestrictedField => "node or global field has host-level effects",
            Self::UnsupportedField => "unsupported source field",
            Self::InvalidDefault => "invalid constrained source default",
            Self::DuplicateDefault => "duplicate source default",
            Self::TooManyNodes => "source artifact node limit exceeded",
            Self::TooDeep => "source artifact value depth limit exceeded",
            Self::RawBodyTooLarge => "source body exceeds the import limit",
            Self::ArtifactTooLarge => "serialized source artifact exceeds the storage limit",
            Self::Encoding => "source artifact encoding failed",
            Self::CorruptArtifact => "private source artifact is inconsistent",
            Self::SourceNotFound => "source was not found",
            Self::SourceArtifactMissing => "source has no private artifact",
            Self::SameBodyPayloadChanged => {
                "same source body has changed normalized definitions or defaults"
            }
            Self::RawBodyDigestCollision => "source body digest does not match stored bytes",
            Self::SourceGenerationConflict => "source generation does not match",
            Self::Store(_) => "profile store operation failed",
        })
    }
}

impl std::error::Error for ArtifactError {}

impl From<StoreError> for ArtifactError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// A full typed native definition supplied by a trusted classifier/parser.
///
/// The JSON tree is preserved exactly and is never formatted. T04 remains responsible
/// for rejecting every unknown nested core field before any runtime construction.
pub struct NodeDefinitionInput {
    protocol: NodeProtocol,
    transport: Transport,
    full_definition: Value,
}

impl NodeDefinitionInput {
    pub fn from_trusted_classifier(
        core_version: &str,
        protocol: NodeProtocol,
        transport: Transport,
        full_definition: Value,
    ) -> Result<Self, ArtifactError> {
        validate(core_version, protocol, transport).map_err(map_capability_error)?;
        if !full_definition
            .as_object()
            .is_some_and(|object| !object.is_empty())
        {
            return Err(ArtifactError::InvalidInput);
        }
        validate_value_tree(&full_definition, 1, &mut 0usize)?;
        validate_known_restrictions(&full_definition, 1)?;
        Ok(Self {
            protocol,
            transport,
            full_definition,
        })
    }

    pub fn protocol(&self) -> NodeProtocol {
        self.protocol
    }

    pub fn transport(&self) -> Transport {
        self.transport
    }

    /// Explicit trusted access to the complete normalized definition.
    pub fn full_definition(&self) -> &Value {
        &self.full_definition
    }
}

impl fmt::Debug for NodeDefinitionInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeDefinitionInput")
            .field("protocol", &self.protocol)
            .field("transport", &self.transport)
            .field("full_definition", &"[REDACTED]")
            .finish()
    }
}

/// Constrained, sorted top-level defaults carried into every definition digest.
pub struct GlobalDefaults(BTreeMap<DefaultField, Value>);

impl GlobalDefaults {
    pub fn new(values: impl IntoIterator<Item = (String, Value)>) -> Result<Self, ArtifactError> {
        let mut defaults = BTreeMap::new();
        for (field, value) in values {
            let field = parse_default_field(&field)?;
            validate_default_value(field, &value)?;
            validate_value_tree(&value, 1, &mut 0usize)?;
            if defaults.insert(field, value).is_some() {
                return Err(ArtifactError::DuplicateDefault);
            }
        }
        Ok(Self(defaults))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Explicit access to one constrained default value.
    pub fn get(&self, field: &str) -> Option<&Value> {
        parse_default_field(field)
            .ok()
            .and_then(|field| self.0.get(&field))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &Value)> + '_ {
        self.0.iter().map(|(field, value)| (field.as_str(), value))
    }
}

impl Default for GlobalDefaults {
    fn default() -> Self {
        Self(BTreeMap::new())
    }
}

impl fmt::Debug for GlobalDefaults {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GlobalDefaults")
            .field("count", &self.0.len())
            .field("values", &"[REDACTED]")
            .finish()
    }
}

/// Validated source inputs. Raw body and actual UA remain private and redacted.
pub struct SourceImportInput {
    raw_body: Vec<u8>,
    format: SourceFormat,
    origin: SourceOrigin,
    accepted_at_unix_ms: i64,
    actual_user_agent: Option<String>,
    definitions: Vec<NodeDefinitionInput>,
    defaults: GlobalDefaults,
}

impl SourceImportInput {
    /// Capture body, actual successful UA, and accepted time from negotiation output.
    pub fn from_negotiated<T>(
        format: ImportFormat,
        negotiated: &Negotiated<T>,
        definitions: Vec<NodeDefinitionInput>,
        defaults: GlobalDefaults,
    ) -> Result<Self, ArtifactError> {
        if negotiated.body().len() > MAX_RAW_SOURCE_BYTES {
            return Err(ArtifactError::RawBodyTooLarge);
        }
        if sha256_hex(negotiated.body()) != negotiated.source_body_sha256() {
            return Err(ArtifactError::CorruptArtifact);
        }
        let raw_body = negotiated.body().to_vec();
        let actual_user_agent = negotiated.actual_user_agent().expose_value().to_owned();
        let input = Self {
            raw_body,
            format: source_format(format),
            origin: SourceOrigin::Negotiated,
            accepted_at_unix_ms: negotiated.accepted_at_unix_ms(),
            actual_user_agent: Some(actual_user_agent),
            definitions,
            defaults,
        };
        validate_input(&input, SourceKind::Subscription)?;
        Ok(input)
    }

    /// Build a local/manual source with no configured User-Agent.
    pub fn local(
        raw_body: Vec<u8>,
        accepted_at_unix_ms: i64,
        definitions: Vec<NodeDefinitionInput>,
        defaults: GlobalDefaults,
    ) -> Result<Self, ArtifactError> {
        let input = Self {
            raw_body,
            format: SourceFormat::LocalDefinition,
            origin: SourceOrigin::Local,
            accepted_at_unix_ms,
            actual_user_agent: None,
            definitions,
            defaults,
        };
        validate_input(&input, SourceKind::ManualServer)?;
        Ok(input)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum DefaultField {
    Mode,
    LogLevel,
    UnifiedDelay,
    TcpConcurrent,
    GlobalClientFingerprint,
}

impl DefaultField {
    fn as_str(self) -> &'static str {
        match self {
            Self::Mode => "mode",
            Self::LogLevel => "log-level",
            Self::UnifiedDelay => "unified-delay",
            Self::TcpConcurrent => "tcp-concurrent",
            Self::GlobalClientFingerprint => "global-client-fingerprint",
        }
    }
}

fn parse_default_field(field: &str) -> Result<DefaultField, ArtifactError> {
    match field {
        "mode" => Ok(DefaultField::Mode),
        "log-level" => Ok(DefaultField::LogLevel),
        "unified-delay" => Ok(DefaultField::UnifiedDelay),
        "tcp-concurrent" => Ok(DefaultField::TcpConcurrent),
        "global-client-fingerprint" => Ok(DefaultField::GlobalClientFingerprint),
        _ => Err(ArtifactError::UnsupportedField),
    }
}

fn validate_default_value(field: DefaultField, value: &Value) -> Result<(), ArtifactError> {
    match field {
        DefaultField::Mode => match value.as_str() {
            Some("rule" | "global" | "direct") => Ok(()),
            _ => Err(ArtifactError::InvalidDefault),
        },
        DefaultField::LogLevel => match value.as_str() {
            Some("silent" | "error" | "warning" | "info" | "debug") => Ok(()),
            _ => Err(ArtifactError::InvalidDefault),
        },
        DefaultField::UnifiedDelay | DefaultField::TcpConcurrent => {
            if value.is_boolean() {
                Ok(())
            } else {
                Err(ArtifactError::InvalidDefault)
            }
        }
        DefaultField::GlobalClientFingerprint => match value.as_str() {
            Some(text)
                if !text.is_empty()
                    && text.len() <= 256
                    && !text.bytes().any(|byte| byte.is_ascii_control()) =>
            {
                Ok(())
            }
            _ => Err(ArtifactError::InvalidDefault),
        },
    }
}

fn validate_value_tree(
    value: &Value,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), ArtifactError> {
    if depth > MAX_ARTIFACT_DEPTH {
        return Err(ArtifactError::TooDeep);
    }
    *nodes = (*nodes).saturating_add(1);
    if *nodes > MAX_ARTIFACT_VALUE_NODES {
        return Err(ArtifactError::ArtifactTooLarge);
    }
    match value {
        Value::Array(items) => {
            for item in items {
                validate_value_tree(item, depth + 1, nodes)?;
            }
        }
        Value::Object(items) => {
            for item in items.values() {
                validate_value_tree(item, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Only inspect fields at the node's top level. Nested headers and plugin-specific maps are
/// not mistaken for host controls; T04's format parser will provide complete nested schemas.
fn validate_known_restrictions(definition: &Value, _depth: usize) -> Result<(), ArtifactError> {
    let object = definition.as_object().ok_or(ArtifactError::InvalidInput)?;
    for field in object.keys() {
        match classify_node_option(field) {
            Ok(super::capabilities::NodeOptionDisposition::RestrictedNative) => {
                return Err(ArtifactError::RestrictedField);
            }
            Err(CapabilityError::UnsupportedFeature) => {
                return Err(ArtifactError::UnsupportedFeature);
            }
            Ok(super::capabilities::NodeOptionDisposition::PreservedNodeOption)
            | Err(CapabilityError::UnsupportedField) => {}
            Err(_) => return Err(ArtifactError::UnsupportedField),
        }
    }
    Ok(())
}

fn source_format(format: ImportFormat) -> SourceFormat {
    match format {
        ImportFormat::UriList => SourceFormat::UriList,
        ImportFormat::Base64UriList => SourceFormat::Base64UriList,
        ImportFormat::MihomoYaml => SourceFormat::MihomoYaml,
        ImportFormat::MihomoJson => SourceFormat::MihomoJson,
    }
}

fn map_capability_error(error: CapabilityError) -> ArtifactError {
    match error {
        CapabilityError::UnsupportedCoreVersion => ArtifactError::UnsupportedCoreVersion,
        CapabilityError::UnsupportedProtocol => ArtifactError::UnsupportedProtocol,
        CapabilityError::UnsupportedTransport => ArtifactError::UnsupportedTransport,
        CapabilityError::UnsupportedFeature => ArtifactError::UnsupportedFeature,
        CapabilityError::UnsupportedField
        | CapabilityError::UnsupportedFormat
        | CapabilityError::UnsupportedUriScheme
        | CapabilityError::UnsupportedProviderMode => ArtifactError::UnsupportedField,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validate_input(input: &SourceImportInput, kind: SourceKind) -> Result<(), ArtifactError> {
    if input.raw_body.len() > MAX_RAW_SOURCE_BYTES {
        return Err(ArtifactError::RawBodyTooLarge);
    }
    if input.definitions.len() > MAX_ARTIFACT_NODE_COUNT {
        return Err(ArtifactError::TooManyNodes);
    }
    if input.accepted_at_unix_ms < 0
        || input.definitions.is_empty()
        || (input.origin == SourceOrigin::Negotiated
            && (kind != SourceKind::Subscription || input.actual_user_agent.is_none()))
        || (input.origin == SourceOrigin::Local
            && (kind != SourceKind::ManualServer || input.actual_user_agent.is_some()))
        || (input.origin == SourceOrigin::Local && input.format != SourceFormat::LocalDefinition)
    {
        return Err(ArtifactError::InvalidInput);
    }
    let mut total_nodes = 0;
    for definition in &input.definitions {
        validate(
            PINNED_CORE_VERSION,
            definition.protocol,
            definition.transport,
        )
        .map_err(map_capability_error)?;
        validate_value_tree(&definition.full_definition, 1, &mut total_nodes)?;
    }
    for (field, value) in &input.defaults.0 {
        validate_default_value(*field, value)?;
        validate_value_tree(value, 1, &mut total_nodes)?;
    }
    preflight::validate_payload_budget(input)?;
    Ok(())
}

#[derive(Serialize)]
struct DefinitionDigestInput<'a> {
    defaults: &'a BTreeMap<DefaultField, Value>,
    protocol: NodeProtocol,
    transport: ArtifactTransport,
    definition: &'a Value,
}

fn definition_digest(
    protocol: NodeProtocol,
    transport: Transport,
    definition: &Value,
    defaults: &GlobalDefaults,
) -> Result<String, ArtifactError> {
    let mut depth_nodes = 0;
    validate_value_tree(definition, 1, &mut depth_nodes)?;
    for value in defaults.0.values() {
        validate_value_tree(value, 1, &mut depth_nodes)?;
    }
    let bytes = encode_json_bounded(
        &DefinitionDigestInput {
            defaults: &defaults.0,
            protocol,
            transport: transport.into(),
            definition,
        },
        MAX_SERIALIZED_ARTIFACT_BYTES,
    )
    .map_err(map_bounded_error)?;
    Ok(sha256_hex(&bytes))
}

fn map_bounded_error(error: BoundedJsonError) -> ArtifactError {
    match error {
        BoundedJsonError::Limit => ArtifactError::ArtifactTooLarge,
        BoundedJsonError::Encoding => ArtifactError::Encoding,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ArtifactTransport {
    Tcp,
    Ws,
    Http,
    H2,
    Grpc,
    Xhttp,
    Quic,
    WireGuard,
}

impl From<Transport> for ArtifactTransport {
    fn from(value: Transport) -> Self {
        match value {
            Transport::Tcp => Self::Tcp,
            Transport::Ws => Self::Ws,
            Transport::Http => Self::Http,
            Transport::H2 => Self::H2,
            Transport::Grpc => Self::Grpc,
            Transport::Xhttp => Self::Xhttp,
            Transport::Quic => Self::Quic,
            Transport::WireGuard => Self::WireGuard,
        }
    }
}

impl From<ArtifactTransport> for Transport {
    fn from(value: ArtifactTransport) -> Self {
        match value {
            ArtifactTransport::Tcp => Self::Tcp,
            ArtifactTransport::Ws => Self::Ws,
            ArtifactTransport::Http => Self::Http,
            ArtifactTransport::H2 => Self::H2,
            ArtifactTransport::Grpc => Self::Grpc,
            ArtifactTransport::Xhttp => Self::Xhttp,
            ArtifactTransport::Quic => Self::Quic,
            ArtifactTransport::WireGuard => Self::WireGuard,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateArtifact {
    schema_version: u32,
    credential_ref: Id,
    source_id: Id,
    source_generation: u64,
    format: SourceFormat,
    origin: SourceOrigin,
    core_version: String,
    core_commit: String,
    raw_body_digest_sha256: String,
    actual_user_agent: Option<String>,
    actual_user_agent_sha256: Option<String>,
    raw_body: Vec<u8>,
    defaults: BTreeMap<DefaultField, Value>,
    definitions: Vec<PrivateNodeDefinition>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateNodeDefinition {
    node_id: Id,
    protocol: NodeProtocol,
    transport: ArtifactTransport,
    definition_digest_sha256: String,
    definition: Value,
}

struct BuiltArtifact {
    bytes: Vec<u8>,
    metadata: crate::profiles::CredentialMetadata,
    nodes: Vec<Node>,
    provenance: SourceProvenance,
    content_digest_sha256: String,
}

fn build_artifact(
    input: SourceImportInput,
    source_id: Id,
    credential_ref: Id,
    generation: u64,
    node_ids: Vec<Id>,
) -> Result<BuiltArtifact, ArtifactError> {
    if node_ids.len() != input.definitions.len() {
        return Err(ArtifactError::InvalidInput);
    }

    let content_digest_sha256 = sha256_hex(&input.raw_body);
    let actual_user_agent_sha256 = input
        .actual_user_agent
        .as_ref()
        .map(|agent| sha256_hex(agent.as_bytes()));
    let mut definitions = Vec::with_capacity(input.definitions.len());
    let mut nodes = Vec::with_capacity(input.definitions.len());
    for (definition, node_id) in input.definitions.into_iter().zip(node_ids) {
        let digest = definition_digest(
            definition.protocol,
            definition.transport,
            &definition.full_definition,
            &input.defaults,
        )?;
        definitions.push(PrivateNodeDefinition {
            node_id: node_id.clone(),
            protocol: definition.protocol,
            transport: definition.transport.into(),
            definition_digest_sha256: digest.clone(),
            definition: definition.full_definition,
        });
        nodes.push(Node {
            schema_version: SCHEMA_VERSION,
            id: node_id,
            source_id: source_id.clone(),
            source_generation: generation,
            definition_digest_sha256: digest,
            protocol: definition.protocol,
            credential_refs: vec![credential_ref.clone()],
        });
    }

    let provenance = SourceProvenance {
        schema_version: SCHEMA_VERSION,
        format: input.format,
        accepted_at_unix_ms: input.accepted_at_unix_ms,
        raw_body_digest_sha256: content_digest_sha256.clone(),
        core_version: PINNED_CORE_VERSION.to_owned(),
        core_commit: PINNED_CORE_COMMIT.to_owned(),
        origin: input.origin,
        actual_user_agent_sha256: actual_user_agent_sha256.clone(),
    };
    let artifact = PrivateArtifact {
        schema_version: MAX_ARTIFACT_SCHEMA_VERSION,
        credential_ref: credential_ref.clone(),
        source_id: source_id.clone(),
        source_generation: generation,
        format: input.format,
        origin: input.origin,
        core_version: PINNED_CORE_VERSION.to_owned(),
        core_commit: PINNED_CORE_COMMIT.to_owned(),
        raw_body_digest_sha256: content_digest_sha256.clone(),
        actual_user_agent: input.actual_user_agent,
        actual_user_agent_sha256,
        raw_body: input.raw_body,
        defaults: input.defaults.0,
        definitions,
    };
    validate_private_artifact(&artifact)?;
    let bytes =
        encode_json_bounded(&artifact, MAX_SERIALIZED_ARTIFACT_BYTES).map_err(map_bounded_error)?;
    let metadata = make_metadata(credential_ref, source_id, &bytes);
    Ok(BuiltArtifact {
        bytes,
        metadata,
        nodes,
        provenance,
        content_digest_sha256,
    })
}

fn validate_private_artifact(artifact: &PrivateArtifact) -> Result<(), ArtifactError> {
    if artifact.schema_version != MAX_ARTIFACT_SCHEMA_VERSION
        || artifact.source_generation == 0
        || artifact.raw_body.len() > MAX_RAW_SOURCE_BYTES
        || artifact.definitions.is_empty()
        || artifact.definitions.len() > MAX_ARTIFACT_NODE_COUNT
        || artifact.core_version != PINNED_CORE_VERSION
        || artifact.core_commit != PINNED_CORE_COMMIT
        || !is_sha256(&artifact.raw_body_digest_sha256)
        || sha256_hex(&artifact.raw_body) != artifact.raw_body_digest_sha256
        || (artifact.origin == SourceOrigin::Local
            && (artifact.format != SourceFormat::LocalDefinition
                || artifact.actual_user_agent.is_some()
                || artifact.actual_user_agent_sha256.is_some()))
        || (artifact.origin == SourceOrigin::Negotiated
            && (artifact.format == SourceFormat::LocalDefinition
                || artifact.actual_user_agent.is_none()))
    {
        return Err(ArtifactError::CorruptArtifact);
    }
    let expected_ua_hash = artifact
        .actual_user_agent
        .as_ref()
        .map(|agent| {
            if agent.is_empty()
                || agent.len() > 512
                || !agent.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
            {
                return Err(ArtifactError::CorruptArtifact);
            }
            Ok(sha256_hex(agent.as_bytes()))
        })
        .transpose()?;
    if expected_ua_hash != artifact.actual_user_agent_sha256 {
        return Err(ArtifactError::CorruptArtifact);
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut total_value_nodes = 0;
    for (field, value) in &artifact.defaults {
        validate_default_value(*field, value).map_err(|_| ArtifactError::CorruptArtifact)?;
        validate_value_tree(value, 1, &mut total_value_nodes)
            .map_err(|_| ArtifactError::CorruptArtifact)?;
    }
    if artifact.defaults.len() > 5 {
        return Err(ArtifactError::CorruptArtifact);
    }
    let defaults = GlobalDefaults(artifact.defaults.clone());
    for definition in &artifact.definitions {
        if !ids.insert(definition.node_id.clone())
            || !is_sha256(&definition.definition_digest_sha256)
        {
            return Err(ArtifactError::CorruptArtifact);
        }
        validate(
            &artifact.core_version,
            definition.protocol,
            definition.transport.into(),
        )
        .map_err(|_| ArtifactError::CorruptArtifact)?;
        if !definition.definition.is_object() {
            return Err(ArtifactError::CorruptArtifact);
        }
        validate_value_tree(&definition.definition, 1, &mut total_value_nodes)
            .map_err(|_| ArtifactError::CorruptArtifact)?;
        validate_known_restrictions(&definition.definition, 1)
            .map_err(|_| ArtifactError::CorruptArtifact)?;
        if definition_digest(
            definition.protocol,
            definition.transport.into(),
            &definition.definition,
            &defaults,
        )
        .map_err(|_| ArtifactError::CorruptArtifact)?
            != definition.definition_digest_sha256
        {
            return Err(ArtifactError::CorruptArtifact);
        }
    }
    Ok(())
}

fn decode_private_artifact(bytes: &[u8]) -> Result<PrivateArtifact, ArtifactError> {
    if bytes.len() > MAX_SERIALIZED_ARTIFACT_BYTES {
        return Err(ArtifactError::ArtifactTooLarge);
    }
    let artifact: PrivateArtifact =
        serde_json::from_slice(bytes).map_err(|_| ArtifactError::CorruptArtifact)?;
    validate_private_artifact(&artifact)?;
    // All accepted blobs are emitted by the bounded canonical encoder. This rejects duplicate
    // nested Value keys that serde_json::Value would otherwise silently collapse.
    let canonical = encode_json_bounded(&artifact, MAX_SERIALIZED_ARTIFACT_BYTES)
        .map_err(|_| ArtifactError::CorruptArtifact)?;
    if canonical != bytes {
        return Err(ArtifactError::CorruptArtifact);
    }
    Ok(artifact)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Safe receipt for a source import publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceImportReceipt {
    pub source_id: Id,
    pub source_generation: u64,
    pub graph_revision: u64,
    pub node_ids: Vec<Id>,
    pub artifact_reused: bool,
}

/// Private artifact material returned only through explicit trusted getters.
pub struct SourceArtifact {
    inner: PrivateArtifact,
}

impl SourceArtifact {
    pub fn source_id(&self) -> &Id {
        &self.inner.source_id
    }

    pub fn source_generation(&self) -> u64 {
        self.inner.source_generation
    }

    pub fn format(&self) -> SourceFormat {
        self.inner.format
    }

    pub fn origin(&self) -> SourceOrigin {
        self.inner.origin
    }

    pub fn core_version(&self) -> &str {
        &self.inner.core_version
    }

    pub fn core_commit(&self) -> &str {
        &self.inner.core_commit
    }

    pub fn raw_body(&self) -> &[u8] {
        &self.inner.raw_body
    }

    pub fn actual_user_agent(&self) -> Option<&str> {
        self.inner.actual_user_agent.as_deref()
    }

    pub fn node_count(&self) -> usize {
        self.inner.definitions.len()
    }

    pub fn definition(&self, node_id: &Id) -> Option<ResolvedNodeDefinition> {
        self.inner
            .definitions
            .iter()
            .find(|definition| &definition.node_id == node_id)
            .map(|definition| ResolvedNodeDefinition::from_private(definition, &self.inner))
    }

    pub fn default_count(&self) -> usize {
        self.inner.defaults.len()
    }

    /// Explicit access to persisted defaults that influenced every node digest.
    pub fn defaults(&self) -> GlobalDefaults {
        GlobalDefaults(self.inner.defaults.clone())
    }
}

impl fmt::Debug for SourceArtifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceArtifact")
            .field("source_id", &self.inner.source_id)
            .field("source_generation", &self.inner.source_generation)
            .field("format", &self.inner.format)
            .field("origin", &self.inner.origin)
            .field("core_version", &self.inner.core_version)
            .field("raw_body", &"[REDACTED]")
            .field("actual_user_agent", &"[REDACTED]")
            .field("node_count", &self.inner.definitions.len())
            .field("defaults", &"[REDACTED]")
            .finish()
    }
}

/// Explicit, redacted-by-default view of one complete normalized node definition.
pub struct ResolvedNodeDefinition {
    source_id: Id,
    source_generation: u64,
    format: SourceFormat,
    origin: SourceOrigin,
    core_version: String,
    core_commit: String,
    protocol: NodeProtocol,
    transport: Transport,
    definition_digest_sha256: String,
    full_definition: Value,
    defaults: GlobalDefaults,
}

impl ResolvedNodeDefinition {
    pub fn source_id(&self) -> &Id {
        &self.source_id
    }

    pub fn source_generation(&self) -> u64 {
        self.source_generation
    }

    pub fn format(&self) -> SourceFormat {
        self.format
    }

    pub fn origin(&self) -> SourceOrigin {
        self.origin
    }

    pub fn core_version(&self) -> &str {
        &self.core_version
    }

    pub fn core_commit(&self) -> &str {
        &self.core_commit
    }

    pub fn protocol(&self) -> NodeProtocol {
        self.protocol
    }

    pub fn transport(&self) -> Transport {
        self.transport
    }

    pub fn definition_digest_sha256(&self) -> &str {
        &self.definition_digest_sha256
    }

    /// Explicit trusted access; values may contain credentials.
    pub fn full_definition(&self) -> &Value {
        &self.full_definition
    }

    /// Explicit access to the defaults included in this node's immutable digest.
    pub fn defaults(&self) -> &GlobalDefaults {
        &self.defaults
    }
}

impl ResolvedNodeDefinition {
    fn from_private(definition: &PrivateNodeDefinition, artifact: &PrivateArtifact) -> Self {
        Self {
            source_id: artifact.source_id.clone(),
            source_generation: artifact.source_generation,
            format: artifact.format,
            origin: artifact.origin,
            core_version: artifact.core_version.clone(),
            core_commit: artifact.core_commit.clone(),
            protocol: definition.protocol,
            transport: definition.transport.into(),
            definition_digest_sha256: definition.definition_digest_sha256.clone(),
            full_definition: definition.definition.clone(),
            defaults: GlobalDefaults(artifact.defaults.clone()),
        }
    }
}

impl fmt::Debug for ResolvedNodeDefinition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedNodeDefinition")
            .field("source_id", &self.source_id)
            .field("source_generation", &self.source_generation)
            .field("format", &self.format)
            .field("origin", &self.origin)
            .field("core_version", &self.core_version)
            .field("protocol", &self.protocol)
            .field("transport", &self.transport)
            .field("definition_digest_sha256", &self.definition_digest_sha256)
            .field("full_definition", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Debug for SourceImportInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceImportInput")
            .field("raw_body", &"[REDACTED]")
            .field("raw_body_len", &self.raw_body.len())
            .field("format", &self.format)
            .field("origin", &self.origin)
            .field("accepted_at_unix_ms", &self.accepted_at_unix_ms)
            .field("actual_user_agent", &"[REDACTED]")
            .field("definition_count", &self.definitions.len())
            .field("defaults", &"[REDACTED]")
            .finish()
    }
}

/// Create a new durable source and generation 1 from already-classified typed records.
pub fn create_source(
    store: &Store,
    expected_revision: u64,
    input: SourceImportInput,
) -> Result<SourceImportReceipt, ArtifactError> {
    let mut candidate = store.read_snapshot()?;
    if candidate.revision != expected_revision {
        return Err(StoreError::Conflict {
            expected: expected_revision,
            current: candidate.revision,
        }
        .into());
    }
    let kind = match input.origin {
        SourceOrigin::Negotiated => SourceKind::Subscription,
        SourceOrigin::Local => SourceKind::ManualServer,
    };
    validate_input(&input, kind)?;
    let source_id = reserve_fresh(&mut candidate, "source", EntityIdKind::Source)?;
    let credential_ref = reserve_fresh(&mut candidate, "artifact", EntityIdKind::CredentialRef)?;
    let mut node_ids = Vec::with_capacity(input.definitions.len());
    for _ in 0..input.definitions.len() {
        node_ids.push(reserve_fresh(&mut candidate, "node", EntityIdKind::Node)?);
    }
    let built = build_artifact(
        input,
        source_id.clone(),
        credential_ref.clone(),
        1,
        node_ids.clone(),
    )?;
    candidate
        .credentials
        .insert(credential_ref.clone(), built.metadata.clone());
    candidate.sources.insert(
        source_id.clone(),
        Source {
            schema_version: SCHEMA_VERSION,
            id: source_id.clone(),
            kind,
            generation: 1,
            content_digest_sha256: built.content_digest_sha256,
            credential: Some(built.metadata),
            provenance: Some(built.provenance),
            current_node_ids: node_ids.clone(),
        },
    );
    for node in &built.nodes {
        candidate.nodes.insert(node.id.clone(), node.clone());
    }
    let material = CredentialMaterial::new(credential_ref, built.bytes)?;
    let committed = store.commit(expected_revision, candidate, vec![material])?;
    let source = committed
        .sources
        .get(&source_id)
        .ok_or(ArtifactError::CorruptArtifact)?;
    Ok(SourceImportReceipt {
        source_id,
        source_generation: source.generation,
        graph_revision: committed.revision,
        node_ids: source.current_node_ids.clone(),
        artifact_reused: false,
    })
}

/// Refresh one source using both graph-revision and source-generation CAS checks.
pub fn update_source(
    store: &Store,
    source_id: &Id,
    expected_revision: u64,
    expected_source_generation: u64,
    input: SourceImportInput,
    evidence_at_unix_ms: i64,
) -> Result<SourceImportReceipt, ArtifactError> {
    if evidence_at_unix_ms < 0 {
        return Err(ArtifactError::InvalidInput);
    }
    let graph = store.read_snapshot()?;
    if graph.revision != expected_revision {
        return Err(StoreError::Conflict {
            expected: expected_revision,
            current: graph.revision,
        }
        .into());
    }
    let old_source = graph
        .sources
        .get(source_id)
        .ok_or(ArtifactError::SourceNotFound)?;
    if old_source.generation != expected_source_generation {
        return Err(StoreError::SourceGenerationConflict {
            expected: expected_source_generation,
            current: old_source.generation,
        }
        .into());
    }
    if old_source.provenance.is_none() || old_source.credential.is_none() {
        return Err(ArtifactError::SourceArtifactMissing);
    }
    validate_input(&input, old_source.kind)?;
    let old_artifact = read_source_artifact_from_graph(store, &graph, source_id)?;
    let old_provenance = old_source
        .provenance
        .as_ref()
        .ok_or(ArtifactError::SourceArtifactMissing)?;
    if input.accepted_at_unix_ms < old_provenance.accepted_at_unix_ms {
        return Err(ArtifactError::InvalidProvenance);
    }

    let next_digest = sha256_hex(&input.raw_body);
    let same_digest = next_digest == old_provenance.raw_body_digest_sha256;
    if same_digest && input.raw_body != old_artifact.raw_body() {
        return Err(ArtifactError::RawBodyDigestCollision);
    }
    if same_digest && !same_normalized_payload(&old_artifact.inner, &input) {
        return Err(ArtifactError::SameBodyPayloadChanged);
    }
    let same_user_agent = old_artifact.actual_user_agent() == input.actual_user_agent.as_deref();

    let (next_source, next_nodes, materials, next_generation, artifact_reused) =
        if same_digest && same_user_agent {
            let mut source = old_source.clone();
            source.provenance = Some(SourceProvenance {
                accepted_at_unix_ms: input.accepted_at_unix_ms,
                ..old_provenance.clone()
            });
            let nodes = source_nodes_from_graph(&graph, &source.current_node_ids)?;
            (source, nodes, Vec::new(), old_source.generation, true)
        } else {
            let mut reservations = graph.clone();
            let generation = if same_digest {
                old_source.generation
            } else {
                old_source
                    .generation
                    .checked_add(1)
                    .ok_or(ArtifactError::InvalidProvenance)?
            };
            let credential_ref =
                reserve_fresh(&mut reservations, "artifact", EntityIdKind::CredentialRef)?;
            let node_ids = if same_digest {
                old_source.current_node_ids.clone()
            } else {
                let mut ids = Vec::with_capacity(input.definitions.len());
                for _ in 0..input.definitions.len() {
                    ids.push(reserve_fresh(
                        &mut reservations,
                        "node",
                        EntityIdKind::Node,
                    )?);
                }
                ids
            };
            let built = build_artifact(
                input,
                source_id.clone(),
                credential_ref.clone(),
                generation,
                node_ids.clone(),
            )?;
            let mut source = old_source.clone();
            source.generation = generation;
            source.content_digest_sha256 = built.content_digest_sha256;
            source.credential = Some(built.metadata);
            source.provenance = Some(built.provenance);
            source.current_node_ids = node_ids;
            let nodes = if same_digest {
                // The newer UA artifact is source-owned; immutable Node pins keep their old refs.
                source_nodes_from_graph(&graph, &source.current_node_ids)?
            } else {
                built.nodes
            };
            let material = CredentialMaterial::new(credential_ref, built.bytes)?;
            (source, nodes, vec![material], generation, false)
        };

    let updated = store.update_source(
        expected_revision,
        expected_source_generation,
        next_source,
        next_nodes,
        materials,
        evidence_at_unix_ms,
    )?;
    let source = updated
        .sources
        .get(source_id)
        .ok_or(ArtifactError::CorruptArtifact)?;
    Ok(SourceImportReceipt {
        source_id: source_id.clone(),
        source_generation: next_generation,
        graph_revision: updated.revision,
        node_ids: source.current_node_ids.clone(),
        artifact_reused,
    })
}

fn source_nodes_from_graph(graph: &GraphSnapshot, ids: &[Id]) -> Result<Vec<Node>, ArtifactError> {
    ids.iter()
        .map(|id| {
            graph
                .nodes
                .get(id)
                .cloned()
                .ok_or(ArtifactError::CorruptArtifact)
        })
        .collect()
}

fn same_normalized_payload(artifact: &PrivateArtifact, input: &SourceImportInput) -> bool {
    artifact.format == input.format
        && artifact.origin == input.origin
        && artifact.core_version == PINNED_CORE_VERSION
        && artifact.core_commit == PINNED_CORE_COMMIT
        && artifact.defaults == input.defaults.0
        && artifact.definitions.len() == input.definitions.len()
        && artifact
            .definitions
            .iter()
            .zip(&input.definitions)
            .all(|(stored, new)| {
                stored.protocol == new.protocol
                    && Transport::from(stored.transport) == new.transport
                    && stored.definition == new.full_definition
            })
}

fn reserve_fresh(
    graph: &mut GraphSnapshot,
    prefix: &str,
    kind: EntityIdKind,
) -> Result<Id, ArtifactError> {
    let id = fresh_id(graph, prefix)?;
    reserve_id(graph, id.clone(), kind)?;
    Ok(id)
}

/// Read the current source artifact, checking safe graph metadata and every current node link.
pub fn read_source_artifact(
    store: &Store,
    source_id: &Id,
) -> Result<SourceArtifact, ArtifactError> {
    let graph = store.read_snapshot()?;
    read_source_artifact_from_graph(store, &graph, source_id)
}

fn read_source_artifact_from_graph(
    store: &Store,
    graph: &GraphSnapshot,
    source_id: &Id,
) -> Result<SourceArtifact, ArtifactError> {
    let source = graph
        .sources
        .get(source_id)
        .ok_or(ArtifactError::SourceNotFound)?;
    let provenance = source
        .provenance
        .as_ref()
        .ok_or(ArtifactError::SourceArtifactMissing)?;
    let metadata = source
        .credential
        .as_ref()
        .ok_or(ArtifactError::SourceArtifactMissing)?;
    if metadata.size_bytes > MAX_SERIALIZED_ARTIFACT_BYTES as u64
        || graph.credentials.get(&metadata.credential_ref) != Some(metadata)
    {
        return Err(ArtifactError::CorruptArtifact);
    }
    let material = store.read_credential(&metadata.credential_ref)?;
    let artifact = decode_private_artifact(material.as_bytes())?;
    if artifact.credential_ref != metadata.credential_ref
        || artifact.source_id != source.id
        || artifact.source_generation != source.generation
        || artifact.raw_body_digest_sha256 != source.content_digest_sha256
        || artifact.raw_body_digest_sha256 != provenance.raw_body_digest_sha256
        || artifact.format != provenance.format
        || artifact.origin != provenance.origin
        || artifact.core_version != provenance.core_version
        || artifact.core_commit != provenance.core_commit
        || artifact.actual_user_agent_sha256 != provenance.actual_user_agent_sha256
        || source.kind
            != match artifact.origin {
                SourceOrigin::Negotiated => SourceKind::Subscription,
                SourceOrigin::Local => SourceKind::ManualServer,
            }
        || artifact.definitions.len() != source.current_node_ids.len()
    {
        return Err(ArtifactError::InvalidProvenance);
    }
    for (definition, node_id) in artifact.definitions.iter().zip(&source.current_node_ids) {
        let node = graph
            .nodes
            .get(node_id)
            .ok_or(ArtifactError::CorruptArtifact)?;
        if &definition.node_id != node_id
            || node.source_id != source.id
            || node.source_generation != source.generation
            || node.protocol != definition.protocol
            || node.definition_digest_sha256 != definition.definition_digest_sha256
        {
            return Err(ArtifactError::CorruptArtifact);
        }
    }
    Ok(SourceArtifact { inner: artifact })
}

/// Resolve the complete immutable definition via the Node's historical artifact reference.
pub fn read_node_definition(
    store: &Store,
    node_id: &Id,
) -> Result<ResolvedNodeDefinition, ArtifactError> {
    let graph = store.read_snapshot()?;
    let node = graph
        .nodes
        .get(node_id)
        .ok_or(ArtifactError::SourceNotFound)?;
    if node.credential_refs.len() != 1 {
        return Err(ArtifactError::CorruptArtifact);
    }
    let credential_ref = &node.credential_refs[0];
    let metadata = graph
        .credentials
        .get(credential_ref)
        .ok_or(ArtifactError::CorruptArtifact)?;
    if metadata.owner_source_id != node.source_id
        || metadata.size_bytes > MAX_SERIALIZED_ARTIFACT_BYTES as u64
    {
        return Err(ArtifactError::CorruptArtifact);
    }
    let material = store.read_credential(credential_ref)?;
    let artifact = decode_private_artifact(material.as_bytes())?;
    if artifact.credential_ref != *credential_ref
        || artifact.source_id != node.source_id
        || artifact.source_generation != node.source_generation
    {
        return Err(ArtifactError::CorruptArtifact);
    }
    let definition = artifact
        .definitions
        .iter()
        .find(|definition| &definition.node_id == node_id)
        .ok_or(ArtifactError::CorruptArtifact)?;
    if definition.protocol != node.protocol
        || definition.definition_digest_sha256 != node.definition_digest_sha256
    {
        return Err(ArtifactError::CorruptArtifact);
    }
    Ok(ResolvedNodeDefinition::from_private(definition, &artifact))
}
