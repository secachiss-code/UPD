//! Schema-1 profile graph types and validation helpers.
//!
//! This module contains no storage or operating-system integration. Credential
//! payloads are intentionally absent: records may carry opaque references and
//! non-secret metadata only.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const SCHEMA_VERSION: u32 = 2;
pub const MAX_OMISSION_SECTIONS: usize = 32;
pub const MAX_SKIPPED_LINE_REPORTS: usize = 64;
pub const MAX_SKIPPED_LINES_PER_CLASS: usize = 4096;
pub const MAX_ID_BYTES: usize = 128;

/// Opaque graph identifier. IDs are ASCII, stable, and shared by every entity type.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Id(String);

impl Id {
    pub fn new(value: impl Into<String>) -> Result<Self, ModelError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_ID_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        {
            return Err(ModelError::InvalidId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Id {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Id {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

/// Stable error codes. Error display strings never include record contents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelError {
    InvalidId,
    UnsupportedSchemaVersion,
    DuplicateId,
    MissingEntity,
    KeyMismatch,
    InvalidReference,
    InvalidGeneration,
    InvalidDigest,
    InvalidPolicy,
    InvalidOwnership,
    InvalidLifecycle,
    InvalidCredentialMetadata,
    InvalidProvenance,
    InvalidVerification,
    InvalidRemovalStage,
    IdReuse,
    NodeMutation,
    ActiveSessionPinsChanged,
    SourceInUse,
    SourceGenerationChangedIncorrectly,
    RevisionChangedIncorrectly,
    IssuedIdsShrank,
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidId => "invalid identifier",
            Self::UnsupportedSchemaVersion => "unsupported schema version",
            Self::DuplicateId => "duplicate identifier",
            Self::MissingEntity => "referenced entity is missing",
            Self::KeyMismatch => "record key does not match record identifier",
            Self::InvalidReference => "invalid graph reference",
            Self::InvalidGeneration => "invalid generation",
            Self::InvalidDigest => "invalid digest",
            Self::InvalidPolicy => "invalid policy",
            Self::InvalidOwnership => "invalid owner reference",
            Self::InvalidLifecycle => "invalid lifecycle transition",
            Self::InvalidCredentialMetadata => "invalid credential metadata",
            Self::InvalidProvenance => "invalid source provenance metadata",
            Self::InvalidVerification => "invalid verification record",
            Self::InvalidRemovalStage => "invalid staged removal record",
            Self::IdReuse => "identifier has already been issued",
            Self::NodeMutation => "node identifier refers to an immutable node",
            Self::ActiveSessionPinsChanged => "session pins are immutable",
            Self::SourceInUse => "source is still referenced",
            Self::SourceGenerationChangedIncorrectly => "source generation is inconsistent",
            Self::RevisionChangedIncorrectly => "graph revision is inconsistent",
            Self::IssuedIdsShrank => "issued identifier registry cannot shrink",
        })
    }
}

impl std::error::Error for ModelError {}

pub type ModelResult<T> = Result<T, ModelError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Subscription,
    ManualServer,
}

/// Persisted source representation. `LocalDefinition` is used for manually entered
/// typed definitions and is not one of the subscription adapter formats.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    UriList,
    Base64UriList,
    MihomoYaml,
    MihomoJson,
    LocalDefinition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceOrigin {
    Negotiated,
    Local,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TlsVerification {
    /// No TLS hop to the proxy (Shadowsocks, plain VLESS/VMess, WireGuard, plain HTTP/SOCKS5).
    /// Never shown as "verified": the channel to the proxy is not authenticated by TLS.
    #[default]
    NotApplicable,
    Verified,
    Pinned,
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkippedLineClass {
    UnsupportedScheme,
    UnsupportedFeature,
    Malformed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkippedLines {
    pub class: SkippedLineClass,
    pub line_numbers: Vec<u32>,
}

/// Safe import summary stored in provenance. No line text, URIs, or field values.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ImportOmissions {
    #[serde(default)]
    pub section_names: Vec<String>,
    #[serde(default)]
    pub skipped_lines: Vec<SkippedLines>,
    #[serde(default)]
    pub tls_verification_disabled_count: u32,
}

impl ImportOmissions {
    pub fn is_empty(&self) -> bool {
        self.section_names.is_empty()
            && self.skipped_lines.is_empty()
            && self.tls_verification_disabled_count == 0
    }

    pub fn requires_confirmation(&self) -> bool {
        !self.is_empty()
    }

    pub fn validate(&self) -> ModelResult<()> {
        if self.section_names.len() > MAX_OMISSION_SECTIONS
            || self.skipped_lines.len() > MAX_SKIPPED_LINE_REPORTS
        {
            return Err(ModelError::InvalidProvenance);
        }
        let mut seen_sections = BTreeSet::new();
        for name in &self.section_names {
            if !is_known_omission_section(name) || !seen_sections.insert(name.as_str()) {
                return Err(ModelError::InvalidProvenance);
            }
        }
        let mut seen_classes = BTreeSet::new();
        for report in &self.skipped_lines {
            if report.line_numbers.is_empty()
                || report.line_numbers.len() > MAX_SKIPPED_LINES_PER_CLASS
                || !seen_classes.insert(report.class)
            {
                return Err(ModelError::InvalidProvenance);
            }
            let mut seen_lines = BTreeSet::new();
            for line in &report.line_numbers {
                if *line == 0 || !seen_lines.insert(line) {
                    return Err(ModelError::InvalidProvenance);
                }
            }
        }
        Ok(())
    }
}

pub fn is_known_omission_section(name: &str) -> bool {
    matches!(
        name,
        "proxy-groups" | "rules" | "sub-rules" | "rule-providers" | "proxy-providers"
    )
}

pub fn import_omissions_digest(omissions: &ImportOmissions) -> ModelResult<String> {
    omissions.validate()?;
    let bytes = serde_json::to_vec(omissions).map_err(|_| ModelError::InvalidProvenance)?;
    Ok(hex_sha256(&bytes))
}

pub fn omissions_within_bound(candidate: &ImportOmissions, bound: &ImportOmissions) -> bool {
    if candidate.tls_verification_disabled_count > bound.tls_verification_disabled_count {
        return false;
    }
    let bound_sections: BTreeSet<&str> = bound.section_names.iter().map(String::as_str).collect();
    let candidate_sections: BTreeSet<&str> =
        candidate.section_names.iter().map(String::as_str).collect();
    if bound_sections != candidate_sections {
        return false;
    }
    let bound_classes: BTreeSet<SkippedLineClass> = bound
        .skipped_lines
        .iter()
        .map(|report| report.class)
        .collect();
    for report in &candidate.skipped_lines {
        if !bound_classes.contains(&report.class) {
            return false;
        }
        let bound_set: BTreeSet<u32> = bound
            .skipped_lines
            .iter()
            .find(|item| item.class == report.class)
            .map(|item| item.line_numbers.iter().copied().collect())
            .unwrap_or_default();
        if !report
            .line_numbers
            .iter()
            .all(|line| bound_set.contains(line))
        {
            return false;
        }
    }
    true
}

/// Safe graph metadata only; URL, raw UA, body bytes and definitions live privately.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceProvenance {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub format: SourceFormat,
    pub accepted_at_unix_ms: i64,
    pub raw_body_digest_sha256: String,
    pub core_version: String,
    pub core_commit: String,
    pub origin: SourceOrigin,
    pub actual_user_agent_sha256: Option<String>,
    #[serde(default)]
    pub omissions: ImportOmissions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_omissions_digest_sha256: Option<String>,
    /// Upper bound accepted by the user for automatic refresh; safe metadata only.
    #[serde(default)]
    pub accepted_omissions_bound: ImportOmissions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialMetadata {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub credential_ref: Id,
    pub owner_source_id: Id,
    pub digest_sha256: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub id: Id,
    pub kind: SourceKind,
    pub generation: u64,
    pub content_digest_sha256: String,
    pub credential: Option<CredentialMetadata>,
    /// Safe, optional metadata added by source imports; absent in legacy schema-1 records.
    #[serde(default)]
    pub provenance: Option<SourceProvenance>,
    /// Nodes from the current generation only; archived nodes live in `GraphSnapshot::nodes`.
    pub current_node_ids: Vec<Id>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeProtocol {
    Vless,
    Vmess,
    Shadowsocks,
    Trojan,
    Socks5,
    Http,
    Hysteria2,
    Tuic,
    WireGuard,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub id: Id,
    pub source_id: Id,
    pub source_generation: u64,
    pub definition_digest_sha256: String,
    pub protocol: NodeProtocol,
    #[serde(default)]
    pub tls_verification: TlsVerification,
    /// References resolve only through the private credential metadata registry.
    pub credential_refs: Vec<Id>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceNodeRef {
    pub source_id: Id,
    pub source_generation: u64,
    pub node_id: Id,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "selection", rename_all = "snake_case", deny_unknown_fields)]
pub enum NodeSelection {
    Pinned { node: SourceNodeRef },
    Policy { name: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelKind {
    Mihomo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ipv6Policy {
    Pass,
    Block,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FakeIpPolicy {
    Allowed,
    Forbidden,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DnsPolicy {
    pub ipv6: Ipv6Policy,
    pub fake_ip: FakeIpPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionProfile {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub id: Id,
    pub source_id: Id,
    pub node_selection: NodeSelection,
    pub kernel: KernelKind,
    pub dns: DnsPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "owner", rename_all = "snake_case", deny_unknown_fields)]
pub enum TunnelOwner {
    Host,
    Application { application_id: Id },
    Group { group_id: Id },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelLifecycle {
    Stopped,
    Starting,
    Running,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelInstance {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub id: Id,
    pub connection_profile_id: Id,
    pub generation: u64,
    pub owner: TunnelOwner,
    pub lifecycle: TunnelLifecycle,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataProfile {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub id: Id,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationEnvironmentVariable {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "assignment", rename_all = "snake_case", deny_unknown_fields)]
pub enum ApplicationAssignment {
    OwnTunnel { tunnel_instance_id: Id },
    Group { group_id: Id },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationDefinition {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub id: Id,
    pub executable: String,
    pub argv: Vec<String>,
    pub cwd: Option<String>,
    pub data_profile_id: Id,
    pub environment: Vec<ApplicationEnvironmentVariable>,
    pub environment_preset_id: Id,
    pub assignment: ApplicationAssignment,
    pub autostart: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationGroup {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub id: Id,
    pub tunnel_instance_id: Id,
    pub mutual_network_access: bool,
    pub autostart: bool,
}

impl ApplicationGroup {
    /// Membership is canonical on application assignments and is derived here.
    pub fn application_ids<'a>(
        &'a self,
        applications: &'a BTreeMap<Id, ApplicationDefinition>,
    ) -> Vec<&'a Id> {
        applications
            .values()
            .filter_map(|application| match &application.assignment {
                ApplicationAssignment::Group { group_id } if group_id == &self.id => {
                    Some(&application.id)
                }
                _ => None,
            })
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPreset {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub id: Id,
    pub timezone: String,
    pub locale: String,
    pub languages: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentProfile {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub id: Id,
    pub session_id: Id,
    pub preset_id: Id,
    pub timezone: String,
    pub locale: String,
    pub languages: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserIdentityProfile {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub id: Id,
    pub data_profile_id: Id,
    /// User-visible label only; fingerprint controls are outside this schema stage.
    pub label: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLifecycle {
    Active,
    Ended,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub id: Id,
    pub lifecycle: SessionLifecycle,
    pub application_id: Id,
    pub data_profile_id: Id,
    pub environment_profile_id: Id,
    pub tunnel_instance_id: Id,
    pub tunnel_generation: u64,
    pub source_id: Id,
    pub source_generation: u64,
    pub node_id: Id,
    pub created_at_unix_ms: i64,
    pub ended_at_unix_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationAxis {
    Net,
    Region,
    State,
    App,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationValue {
    #[default]
    Unknown,
    Partial,
    Verified,
    Blocked,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub id: Id,
    pub session_id: Id,
    pub tunnel_instance_id: Id,
    pub tunnel_generation: u64,
    pub axis: VerificationAxis,
    pub value: VerificationValue,
    pub evidence_at_unix_ms: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostMode {
    Off,
    Proxy,
    Tunnel,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostPolicy {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub id: Id,
    pub mode: HostMode,
    pub connection_profile_id: Option<Id>,
    pub tunnel_instance_id: Option<Id>,
    pub autostart: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovalStage {
    LinksPending,
    BlobPending,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StagedRemoval {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub plan_id: Id,
    pub source_id: Id,
    pub credential_metadata: Vec<CredentialMetadata>,
    #[serde(deserialize_with = "deserialize_unique_set")]
    pub removed_credential_refs: BTreeSet<Id>,
    pub stage: RemovalStage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityIdKind {
    Source,
    Node,
    ConnectionProfile,
    TunnelInstance,
    DataProfile,
    Application,
    ApplicationGroup,
    EnvironmentPreset,
    EnvironmentProfile,
    BrowserIdentityProfile,
    Session,
    Verification,
    HostPolicy,
    RemovalPlan,
    CredentialRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSnapshot {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub revision: u64,
    /// Monotonic shared registry: current, removed, credential, and plan IDs stay reserved.
    #[serde(deserialize_with = "deserialize_unique_set")]
    pub issued_ids: BTreeSet<Id>,
    /// Persistent type assignment lets future writers reject cross-type ID reuse after deletion.
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub issued_id_kinds: BTreeMap<Id, EntityIdKind>,
    /// Global immutable metadata index; payload bytes are kept outside the public graph.
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub credentials: BTreeMap<Id, CredentialMetadata>,
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub sources: BTreeMap<Id, Source>,
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub nodes: BTreeMap<Id, Node>,
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub connection_profiles: BTreeMap<Id, ConnectionProfile>,
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub tunnel_instances: BTreeMap<Id, TunnelInstance>,
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub data_profiles: BTreeMap<Id, DataProfile>,
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub applications: BTreeMap<Id, ApplicationDefinition>,
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub application_groups: BTreeMap<Id, ApplicationGroup>,
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub environment_presets: BTreeMap<Id, EnvironmentPreset>,
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub environment_profiles: BTreeMap<Id, EnvironmentProfile>,
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub browser_identity_profiles: BTreeMap<Id, BrowserIdentityProfile>,
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub sessions: BTreeMap<Id, Session>,
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub verifications: BTreeMap<Id, Verification>,
    pub host_policy: Option<HostPolicy>,
    #[serde(deserialize_with = "deserialize_unique_map")]
    pub staged_removals: BTreeMap<Id, StagedRemoval>,
}

impl Default for GraphSnapshot {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            revision: 0,
            issued_ids: BTreeSet::new(),
            issued_id_kinds: BTreeMap::new(),
            credentials: BTreeMap::new(),
            sources: BTreeMap::new(),
            nodes: BTreeMap::new(),
            connection_profiles: BTreeMap::new(),
            tunnel_instances: BTreeMap::new(),
            data_profiles: BTreeMap::new(),
            applications: BTreeMap::new(),
            application_groups: BTreeMap::new(),
            environment_presets: BTreeMap::new(),
            environment_profiles: BTreeMap::new(),
            browser_identity_profiles: BTreeMap::new(),
            sessions: BTreeMap::new(),
            verifications: BTreeMap::new(),
            host_policy: None,
            staged_removals: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicCredentialPresence {
    Absent,
    Referenced,
}

/// Explicit status projection. It does not serialize the private graph or secret payloads.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicSourceStatus {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub id: Id,
    pub kind: SourceKind,
    pub generation: u64,
    pub current_node_count: usize,
    pub credential_presence: PublicCredentialPresence,
    pub credential_ref: Option<Id>,
    pub credential_digest_sha256: Option<String>,
    pub credential_size_bytes: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationAxes {
    pub net: VerificationValue,
    pub region: VerificationValue,
    pub state: VerificationValue,
    pub app: VerificationValue,
}

impl Default for VerificationAxes {
    fn default() -> Self {
        Self {
            net: VerificationValue::Unknown,
            region: VerificationValue::Unknown,
            state: VerificationValue::Unknown,
            app: VerificationValue::Unknown,
        }
    }
}

impl VerificationAxes {
    pub fn set(&mut self, axis: VerificationAxis, value: VerificationValue) {
        match axis {
            VerificationAxis::Net => self.net = value,
            VerificationAxis::Region => self.region = value,
            VerificationAxis::State => self.state = value,
            VerificationAxis::App => self.app = value,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicSessionStatus {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub id: Id,
    pub lifecycle: SessionLifecycle,
    pub application_id: Id,
    pub data_profile_id: Id,
    pub tunnel_instance_id: Id,
    pub tunnel_generation: u64,
    pub source_id: Id,
    pub source_generation: u64,
    pub node_id: Id,
    pub environment: EnvironmentSnapshotStatus,
    pub verification: VerificationAxes,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentSnapshotStatus {
    pub timezone: String,
    pub locale: String,
    pub languages: Vec<String>,
}

impl PublicSessionStatus {
    pub fn from_graph(graph: &GraphSnapshot, session_id: &Id) -> ModelResult<Self> {
        graph.validate()?;
        let session = graph
            .sessions
            .get(session_id)
            .ok_or(ModelError::MissingEntity)?;
        let environment = graph
            .environment_profiles
            .get(&session.environment_profile_id)
            .ok_or(ModelError::MissingEntity)?;
        let mut verification = VerificationAxes::default();
        for item in graph
            .verifications
            .values()
            .filter(|item| item.session_id == session.id)
        {
            verification.set(item.axis, item.value);
        }
        Ok(Self {
            schema_version: SCHEMA_VERSION,
            id: session.id.clone(),
            lifecycle: session.lifecycle,
            application_id: session.application_id.clone(),
            data_profile_id: session.data_profile_id.clone(),
            tunnel_instance_id: session.tunnel_instance_id.clone(),
            tunnel_generation: session.tunnel_generation,
            source_id: session.source_id.clone(),
            source_generation: session.source_generation,
            node_id: session.node_id.clone(),
            environment: EnvironmentSnapshotStatus {
                timezone: environment.timezone.clone(),
                locale: environment.locale.clone(),
                languages: environment.languages.clone(),
            },
            verification,
        })
    }
}

impl GraphSnapshot {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate schema, identity ownership, generations, and all live graph references.
    pub fn validate(&self) -> ModelResult<()> {
        require_schema(self.schema_version)?;
        let claims = self.identity_claims()?;
        for (id, kind) in &claims {
            if !self.issued_ids.contains(id) || self.issued_id_kinds.get(id) != Some(kind) {
                return Err(ModelError::IdReuse);
            }
        }
        for id in &self.issued_ids {
            validate_id(id)?;
            if !self.issued_id_kinds.contains_key(id) {
                return Err(ModelError::IdReuse);
            }
        }
        if self
            .issued_id_kinds
            .keys()
            .any(|id| !self.issued_ids.contains(id))
        {
            return Err(ModelError::IdReuse);
        }
        self.validate_records()?;
        self.validate_references()?;
        Ok(())
    }

    /// Validate a proposed complete graph replacement against its previous snapshot.
    /// This is a pure validator; it performs no persistence or synchronization.
    pub fn validate_transition(old: &Self, new: &Self) -> ModelResult<()> {
        old.validate()?;
        new.validate()?;
        let expected_revision = old
            .revision
            .checked_add(1)
            .ok_or(ModelError::InvalidGeneration)?;
        if new.revision != expected_revision {
            return Err(ModelError::RevisionChangedIncorrectly);
        }
        if !old.issued_ids.is_subset(&new.issued_ids) {
            return Err(ModelError::IssuedIdsShrank);
        }
        for (id, kind) in &old.issued_id_kinds {
            if new.issued_id_kinds.get(id) != Some(kind) {
                return Err(ModelError::IdReuse);
            }
        }

        let old_claims = old.identity_claims()?;
        let new_claims = new.identity_claims()?;
        for (id, new_type) in &new_claims {
            match old_claims.get(id) {
                Some(old_type) if old_type == new_type => {}
                Some(_) => return Err(ModelError::IdReuse),
                None if old.issued_ids.contains(id) => return Err(ModelError::IdReuse),
                None => {}
            }
        }

        for (id, old_node) in &old.nodes {
            if let Some(new_node) = new.nodes.get(id)
                && old_node != new_node
            {
                return Err(ModelError::NodeMutation);
            }
        }

        for (id, previous) in &old.tunnel_instances {
            if let Some(current) = new.tunnel_instances.get(id) {
                if previous.owner != current.owner
                    || (previous.generation != current.generation
                        && previous.generation.checked_add(1) != Some(current.generation))
                {
                    return Err(ModelError::InvalidGeneration);
                }
                if previous.connection_profile_id != current.connection_profile_id
                    && previous.generation == current.generation
                {
                    return Err(ModelError::InvalidGeneration);
                }
            }
        }
        for (id, current) in &new.tunnel_instances {
            if !old.tunnel_instances.contains_key(id) && current.generation != 1 {
                return Err(ModelError::InvalidGeneration);
            }
        }

        for (id, old_source) in &old.sources {
            match new.sources.get(id) {
                Some(new_source) => {
                    if old_source.kind != new_source.kind {
                        return Err(ModelError::InvalidLifecycle);
                    }
                    if let Some(old_provenance) = &old_source.provenance {
                        let new_provenance = new_source
                            .provenance
                            .as_ref()
                            .ok_or(ModelError::InvalidProvenance)?;
                        if new_provenance.accepted_at_unix_ms < old_provenance.accepted_at_unix_ms {
                            return Err(ModelError::InvalidProvenance);
                        }
                        if old_source.content_digest_sha256 == new_source.content_digest_sha256
                            && (old_provenance.format != new_provenance.format
                                || old_provenance.origin != new_provenance.origin
                                || old_provenance.core_version != new_provenance.core_version
                                || old_provenance.core_commit != new_provenance.core_commit)
                        {
                            return Err(ModelError::InvalidProvenance);
                        }
                    }
                    if old_source.content_digest_sha256 == new_source.content_digest_sha256 {
                        if old_source.generation != new_source.generation
                            || old_source.current_node_ids != new_source.current_node_ids
                        {
                            return Err(ModelError::SourceGenerationChangedIncorrectly);
                        }
                    } else if old_source.generation.checked_add(1) != Some(new_source.generation) {
                        return Err(ModelError::SourceGenerationChangedIncorrectly);
                    } else {
                        for node_id in &new_source.current_node_ids {
                            if old.issued_ids.contains(node_id) {
                                return Err(ModelError::IdReuse);
                            }
                        }
                    }
                }
                None if old.source_is_referenced(id) => return Err(ModelError::SourceInUse),
                None => {
                    let has_durable_stage = new.staged_removals.values().any(|removal| {
                        removal.source_id == *id && removal.stage == RemovalStage::BlobPending
                    });
                    if !has_durable_stage {
                        return Err(ModelError::InvalidRemovalStage);
                    }
                    let old_candidates = source_removal_candidates(old, id);
                    let removal = new
                        .staged_removals
                        .values()
                        .find(|removal| {
                            removal.source_id == *id && removal.stage == RemovalStage::BlobPending
                        })
                        .ok_or(ModelError::InvalidRemovalStage)?;
                    if removal.credential_metadata != old_candidates {
                        return Err(ModelError::InvalidRemovalStage);
                    }
                }
            }
        }
        for (id, new_source) in &new.sources {
            if !old.sources.contains_key(id) && new_source.generation != 1 {
                return Err(ModelError::SourceGenerationChangedIncorrectly);
            }
        }

        for (id, old_session) in &old.sessions {
            let new_session = new.sessions.get(id).ok_or(ModelError::InvalidLifecycle)?;
            if session_pins(old_session) != session_pins(new_session)
                || old_session.created_at_unix_ms != new_session.created_at_unix_ms
                || (old_session.lifecycle == SessionLifecycle::Ended
                    && old_session.ended_at_unix_ms != new_session.ended_at_unix_ms)
            {
                return Err(ModelError::ActiveSessionPinsChanged);
            }
            match (old_session.lifecycle, new_session.lifecycle) {
                (SessionLifecycle::Active, SessionLifecycle::Active | SessionLifecycle::Ended) => {}
                (SessionLifecycle::Ended, SessionLifecycle::Ended) => {}
                _ => return Err(ModelError::InvalidLifecycle),
            }
            if old_session.lifecycle == SessionLifecycle::Active {
                let old_environment = old
                    .environment_profiles
                    .get(&old_session.environment_profile_id)
                    .ok_or(ModelError::MissingEntity)?;
                let new_environment = new
                    .environment_profiles
                    .get(&old_session.environment_profile_id)
                    .ok_or(ModelError::MissingEntity)?;
                if old_environment != new_environment {
                    return Err(ModelError::ActiveSessionPinsChanged);
                }
                let old_identity = old
                    .browser_identity_profiles
                    .values()
                    .find(|identity| identity.data_profile_id == old_session.data_profile_id);
                let new_identity = new
                    .browser_identity_profiles
                    .values()
                    .find(|identity| identity.data_profile_id == old_session.data_profile_id);
                if old_identity != new_identity {
                    return Err(ModelError::ActiveSessionPinsChanged);
                }
            }
        }

        for (id, new_session) in &new.sessions {
            if old.sessions.contains_key(id) {
                continue;
            }
            if new_session.lifecycle != SessionLifecycle::Active {
                return Err(ModelError::InvalidLifecycle);
            }
            let app = new
                .applications
                .get(&new_session.application_id)
                .ok_or(ModelError::MissingEntity)?;
            let environment = new
                .environment_profiles
                .get(&new_session.environment_profile_id)
                .ok_or(ModelError::MissingEntity)?;
            let preset = new
                .environment_presets
                .get(&app.environment_preset_id)
                .ok_or(ModelError::MissingEntity)?;
            if environment.preset_id != preset.id
                || environment.timezone != preset.timezone
                || environment.locale != preset.locale
                || environment.languages != preset.languages
            {
                return Err(ModelError::InvalidReference);
            }
            let tunnel = new
                .tunnel_instances
                .get(&new_session.tunnel_instance_id)
                .ok_or(ModelError::MissingEntity)?;
            let profile = new
                .connection_profiles
                .get(&tunnel.connection_profile_id)
                .ok_or(ModelError::MissingEntity)?;
            let source = new
                .sources
                .get(&new_session.source_id)
                .ok_or(ModelError::MissingEntity)?;
            let node = new
                .nodes
                .get(&new_session.node_id)
                .ok_or(ModelError::MissingEntity)?;
            if new_session.tunnel_generation != tunnel.generation
                || profile.source_id != new_session.source_id
                || node.source_id != new_session.source_id
                || node.source_generation != new_session.source_generation
            {
                return Err(ModelError::InvalidReference);
            }
            match &profile.node_selection {
                NodeSelection::Pinned { node: pin }
                    if pin.node_id != new_session.node_id
                        || pin.source_generation != new_session.source_generation =>
                {
                    return Err(ModelError::InvalidReference);
                }
                NodeSelection::Policy { .. }
                    if source.generation != new_session.source_generation
                        || !source.current_node_ids.contains(&new_session.node_id) =>
                {
                    return Err(ModelError::InvalidReference);
                }
                _ => {}
            }
            let assigned_tunnel = match &app.assignment {
                ApplicationAssignment::OwnTunnel { tunnel_instance_id } => tunnel_instance_id,
                ApplicationAssignment::Group { group_id } => {
                    &new.application_groups
                        .get(group_id)
                        .ok_or(ModelError::MissingEntity)?
                        .tunnel_instance_id
                }
            };
            if assigned_tunnel != &new_session.tunnel_instance_id {
                return Err(ModelError::InvalidReference);
            }
        }

        for (id, previous) in &old.verifications {
            if let Some(current) = new.verifications.get(id)
                && (previous.session_id != current.session_id
                    || previous.tunnel_instance_id != current.tunnel_instance_id
                    || previous.tunnel_generation != current.tunnel_generation
                    || previous.axis != current.axis
                    || current.evidence_at_unix_ms < previous.evidence_at_unix_ms)
            {
                return Err(ModelError::InvalidVerification);
            }
        }

        for (id, old_removal) in &old.staged_removals {
            let new_removal = new
                .staged_removals
                .get(id)
                .ok_or(ModelError::InvalidRemovalStage)?;
            if old_removal.plan_id != new_removal.plan_id
                || old_removal.source_id != new_removal.source_id
                || old_removal.credential_metadata != new_removal.credential_metadata
                || !old_removal
                    .removed_credential_refs
                    .is_subset(&new_removal.removed_credential_refs)
                || !removal_stage_advances(old_removal.stage, new_removal.stage)
            {
                return Err(ModelError::InvalidRemovalStage);
            }
        }
        for (id, new_removal) in &new.staged_removals {
            if old.staged_removals.contains_key(id) {
                continue;
            }
            let source = old
                .sources
                .get(&new_removal.source_id)
                .ok_or(ModelError::InvalidRemovalStage)?;
            let expected = source_removal_candidates(old, &source.id);
            let inline_removal = new_removal.stage == RemovalStage::BlobPending
                && !new.sources.contains_key(&source.id);
            if (!inline_removal && new_removal.stage != RemovalStage::LinksPending)
                || new_removal.credential_metadata != expected
                || !new_removal.removed_credential_refs.is_empty()
            {
                return Err(ModelError::InvalidRemovalStage);
            }
        }
        for (credential_id, old_metadata) in &old.credentials {
            match new.credentials.get(credential_id) {
                Some(new_metadata) if new_metadata == old_metadata => {}
                Some(_) => return Err(ModelError::InvalidCredentialMetadata),
                None => {
                    let is_completed_removal = new.staged_removals.values().any(|removal| {
                        removal.stage == RemovalStage::Complete
                            && removal.removed_credential_refs.contains(credential_id)
                    });
                    if !is_completed_removal {
                        return Err(ModelError::InvalidCredentialMetadata);
                    }
                }
            }
        }
        for (credential_id, new_metadata) in &new.credentials {
            if !old.credentials.contains_key(credential_id)
                && !new.sources.contains_key(&new_metadata.owner_source_id)
            {
                return Err(ModelError::InvalidCredentialMetadata);
            }
        }
        for (id, old_source) in &old.sources {
            if let Some(new_source) = new.sources.get(id) {
                let generation_changed = old_source.generation != new_source.generation;
                if generation_changed {
                    let old_digests: BTreeSet<&str> = old_source
                        .current_node_ids
                        .iter()
                        .filter_map(|node_id| old.nodes.get(node_id))
                        .map(|node| node.definition_digest_sha256.as_str())
                        .collect();
                    let new_digests: BTreeSet<&str> = new_source
                        .current_node_ids
                        .iter()
                        .filter_map(|node_id| new.nodes.get(node_id))
                        .map(|node| node.definition_digest_sha256.as_str())
                        .collect();
                    let lost_active_pins: Vec<&Session> = old
                        .sessions
                        .values()
                        .filter(|session| {
                            session.lifecycle == SessionLifecycle::Active
                                && session.source_id == *id
                                && old.nodes.get(&session.node_id).is_some_and(|node| {
                                    old_digests.contains(node.definition_digest_sha256.as_str())
                                        && !new_digests
                                            .contains(node.definition_digest_sha256.as_str())
                                })
                        })
                        .collect();
                    for session in lost_active_pins {
                        let new_axes = session_axes(new, &session.id);
                        if !matches!(
                            new_axes.net,
                            VerificationValue::Blocked | VerificationValue::Error
                        ) || axis_record(old, &session.id, VerificationAxis::Region)
                            != axis_record(new, &session.id, VerificationAxis::Region)
                            || axis_record(old, &session.id, VerificationAxis::State)
                                != axis_record(new, &session.id, VerificationAxis::State)
                            || axis_record(old, &session.id, VerificationAxis::App)
                                != axis_record(new, &session.id, VerificationAxis::App)
                        {
                            return Err(ModelError::InvalidVerification);
                        }
                        let old_time =
                            session_axis_evidence(old, &session.id, VerificationAxis::Net)
                                .unwrap_or(i64::MIN);
                        let new_time =
                            session_axis_evidence(new, &session.id, VerificationAxis::Net)
                                .ok_or(ModelError::InvalidVerification)?;
                        if new_time <= old_time {
                            return Err(ModelError::InvalidVerification);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn source_is_referenced(&self, source_id: &Id) -> bool {
        if self
            .connection_profiles
            .values()
            .any(|profile| &profile.source_id == source_id)
        {
            return true;
        }
        self.sessions.values().any(|session| {
            session.lifecycle == SessionLifecycle::Active && &session.source_id == source_id
        })
    }

    pub fn node_is_referenced(&self, node_id: &Id) -> bool {
        self.connection_profiles.values().any(|profile| {
            matches!(&profile.node_selection, NodeSelection::Pinned { node } if &node.node_id == node_id)
        }) || self.sessions.values().any(|session| {
            session.lifecycle == SessionLifecycle::Active && &session.node_id == node_id
        })
    }

    pub fn public_source_status(&self, source_id: &Id) -> ModelResult<PublicSourceStatus> {
        self.validate()?;
        let source = self
            .sources
            .get(source_id)
            .ok_or(ModelError::MissingEntity)?;
        let (presence, reference, digest, size) = match &source.credential {
            Some(credential) => (
                PublicCredentialPresence::Referenced,
                Some(credential.credential_ref.clone()),
                Some(credential.digest_sha256.clone()),
                Some(credential.size_bytes),
            ),
            None => (PublicCredentialPresence::Absent, None, None, None),
        };
        Ok(PublicSourceStatus {
            schema_version: SCHEMA_VERSION,
            id: source.id.clone(),
            kind: source.kind,
            generation: source.generation,
            current_node_count: source.current_node_ids.len(),
            credential_presence: presence,
            credential_ref: reference,
            credential_digest_sha256: digest,
            credential_size_bytes: size,
        })
    }

    fn identity_claims(&self) -> ModelResult<BTreeMap<Id, EntityIdKind>> {
        let mut claims = BTreeMap::new();
        macro_rules! add_records {
            ($map:expr, $kind:literal) => {
                for (key, record) in &$map {
                    validate_id(key)?;
                    if key != &record.id {
                        return Err(ModelError::KeyMismatch);
                    }
                    add_claim(&mut claims, record.id.clone(), entity_kind($kind))?;
                }
            };
        }
        add_records!(self.sources, "source");
        add_records!(self.nodes, "node");
        add_records!(self.connection_profiles, "connection_profile");
        add_records!(self.tunnel_instances, "tunnel_instance");
        add_records!(self.data_profiles, "data_profile");
        add_records!(self.applications, "application");
        add_records!(self.application_groups, "application_group");
        add_records!(self.environment_presets, "environment_preset");
        add_records!(self.environment_profiles, "environment_profile");
        add_records!(self.browser_identity_profiles, "browser_identity_profile");
        add_records!(self.sessions, "session");
        add_records!(self.verifications, "verification");
        if let Some(host) = &self.host_policy {
            add_claim(&mut claims, host.id.clone(), EntityIdKind::HostPolicy)?;
        }
        for (key, removal) in &self.staged_removals {
            if key != &removal.plan_id {
                return Err(ModelError::KeyMismatch);
            }
            add_claim(
                &mut claims,
                removal.plan_id.clone(),
                EntityIdKind::RemovalPlan,
            )?;
        }
        for credential_id in self.credentials.keys() {
            add_claim(
                &mut claims,
                credential_id.clone(),
                EntityIdKind::CredentialRef,
            )?;
        }
        Ok(claims)
    }

    fn validate_records(&self) -> ModelResult<()> {
        for source in self.sources.values() {
            require_schema(source.schema_version)?;
            if source.generation == 0 || !is_sha256(&source.content_digest_sha256) {
                return Err(ModelError::InvalidGeneration);
            }
            if let Some(provenance) = &source.provenance {
                require_schema(provenance.schema_version)?;
                if source.credential.is_none()
                    || provenance.accepted_at_unix_ms < 0
                    || provenance.raw_body_digest_sha256 != source.content_digest_sha256
                    || !is_sha256(&provenance.raw_body_digest_sha256)
                    || !is_bounded_core_version(&provenance.core_version)
                    || !is_git_commit(&provenance.core_commit)
                    || matches!(
                        (
                            provenance.origin,
                            provenance.actual_user_agent_sha256.as_deref()
                        ),
                        (SourceOrigin::Negotiated, None) | (SourceOrigin::Local, Some(_))
                    )
                    || (provenance.origin == SourceOrigin::Negotiated
                        && source.kind != SourceKind::Subscription)
                    || provenance
                        .actual_user_agent_sha256
                        .as_deref()
                        .is_some_and(|digest| !is_sha256(digest))
                    || (provenance.format == SourceFormat::LocalDefinition
                        && provenance.origin != SourceOrigin::Local)
                    || provenance.omissions.validate().is_err()
                    || provenance
                        .confirmed_omissions_digest_sha256
                        .as_deref()
                        .is_some_and(|digest| !is_sha256(digest))
                {
                    return Err(ModelError::InvalidProvenance);
                }
                if provenance.omissions.requires_confirmation()
                    && provenance.confirmed_omissions_digest_sha256.is_none()
                {
                    return Err(ModelError::InvalidProvenance);
                }
                if let Some(confirmed) = &provenance.confirmed_omissions_digest_sha256 {
                    let expected = import_omissions_digest(&provenance.omissions)?;
                    if confirmed != &expected {
                        return Err(ModelError::InvalidProvenance);
                    }
                    if provenance.accepted_omissions_bound.validate().is_err()
                        || !omissions_within_bound(
                            &provenance.omissions,
                            &provenance.accepted_omissions_bound,
                        )
                    {
                        return Err(ModelError::InvalidProvenance);
                    }
                } else if !provenance.accepted_omissions_bound.is_empty() {
                    return Err(ModelError::InvalidProvenance);
                }
            }
            if let Some(credential) = &source.credential {
                require_schema(credential.schema_version)?;
                if !is_sha256(&credential.digest_sha256)
                    || (source.provenance.is_some() && credential.owner_source_id != source.id)
                {
                    return Err(ModelError::InvalidCredentialMetadata);
                }
            }
            ensure_unique_ids(&source.current_node_ids)?;
        }
        for (key, credential) in &self.credentials {
            require_schema(credential.schema_version)?;
            if key != &credential.credential_ref
                || !is_sha256(&credential.digest_sha256)
                || self.issued_id_kinds.get(&credential.owner_source_id)
                    != Some(&EntityIdKind::Source)
            {
                return Err(ModelError::InvalidCredentialMetadata);
            }
        }
        for node in self.nodes.values() {
            require_schema(node.schema_version)?;
            if node.source_generation == 0 || !is_sha256(&node.definition_digest_sha256) {
                return Err(ModelError::InvalidGeneration);
            }
            ensure_unique_ids(&node.credential_refs)?;
        }
        for profile in self.connection_profiles.values() {
            require_schema(profile.schema_version)?;
            if let NodeSelection::Policy { name } = &profile.node_selection
                && name.is_empty()
            {
                return Err(ModelError::InvalidPolicy);
            }
        }
        for tunnel in self.tunnel_instances.values() {
            require_schema(tunnel.schema_version)?;
            if tunnel.generation == 0 {
                return Err(ModelError::InvalidGeneration);
            }
        }
        for data in self.data_profiles.values() {
            require_schema(data.schema_version)?;
        }
        for app in self.applications.values() {
            require_schema(app.schema_version)?;
            if app.executable.is_empty() || app.environment.iter().any(|item| item.name.is_empty())
            {
                return Err(ModelError::InvalidPolicy);
            }
        }
        for group in self.application_groups.values() {
            require_schema(group.schema_version)?;
        }
        for preset in self.environment_presets.values() {
            require_schema(preset.schema_version)?;
            if preset.timezone.is_empty() || preset.locale.is_empty() || preset.languages.is_empty()
            {
                return Err(ModelError::InvalidPolicy);
            }
        }
        for profile in self.environment_profiles.values() {
            require_schema(profile.schema_version)?;
            if profile.timezone.is_empty()
                || profile.locale.is_empty()
                || profile.languages.is_empty()
            {
                return Err(ModelError::InvalidPolicy);
            }
        }
        for profile in self.browser_identity_profiles.values() {
            require_schema(profile.schema_version)?;
        }
        for session in self.sessions.values() {
            require_schema(session.schema_version)?;
            if session.tunnel_generation == 0
                || session.source_generation == 0
                || session.created_at_unix_ms < 0
            {
                return Err(ModelError::InvalidGeneration);
            }
            match (session.lifecycle, session.ended_at_unix_ms) {
                (SessionLifecycle::Active, None) => {}
                (SessionLifecycle::Ended, Some(ended)) if ended >= session.created_at_unix_ms => {}
                _ => return Err(ModelError::InvalidLifecycle),
            }
        }
        for verification in self.verifications.values() {
            require_schema(verification.schema_version)?;
            if verification.tunnel_generation == 0 || verification.evidence_at_unix_ms < 0 {
                return Err(ModelError::InvalidGeneration);
            }
        }
        if let Some(host) = &self.host_policy {
            require_schema(host.schema_version)?;
            match host.mode {
                HostMode::Off
                    if host.connection_profile_id.is_none()
                        && host.tunnel_instance_id.is_none() => {}
                HostMode::Off => return Err(ModelError::InvalidPolicy),
                HostMode::Proxy | HostMode::Tunnel
                    if host.connection_profile_id.is_some()
                        && host.tunnel_instance_id.is_some() => {}
                HostMode::Proxy | HostMode::Tunnel => return Err(ModelError::InvalidPolicy),
            }
        }
        for removal in self.staged_removals.values() {
            require_schema(removal.schema_version)?;
            ensure_unique_ids(
                &removal
                    .credential_metadata
                    .iter()
                    .map(|item| item.credential_ref.clone())
                    .collect::<Vec<_>>(),
            )?;
            if removal.removed_credential_refs.iter().any(|id| {
                !removal
                    .credential_metadata
                    .iter()
                    .any(|item| &item.credential_ref == id)
            }) {
                return Err(ModelError::InvalidRemovalStage);
            }
            for metadata in &removal.credential_metadata {
                require_schema(metadata.schema_version)?;
                if !is_sha256(&metadata.digest_sha256)
                    || self.issued_id_kinds.get(&metadata.owner_source_id)
                        != Some(&EntityIdKind::Source)
                    || self.issued_id_kinds.get(&metadata.credential_ref)
                        != Some(&EntityIdKind::CredentialRef)
                {
                    return Err(ModelError::InvalidCredentialMetadata);
                }
            }
            match removal.stage {
                RemovalStage::LinksPending if self.sources.contains_key(&removal.source_id) => {}
                RemovalStage::BlobPending if !self.sources.contains_key(&removal.source_id) => {}
                RemovalStage::Complete if !self.sources.contains_key(&removal.source_id) => {}
                _ => return Err(ModelError::InvalidRemovalStage),
            }
        }
        Ok(())
    }

    fn validate_references(&self) -> ModelResult<()> {
        for source in self.sources.values() {
            if let Some(metadata) = &source.credential
                && self.credentials.get(&metadata.credential_ref) != Some(metadata)
            {
                return Err(ModelError::InvalidCredentialMetadata);
            }
            for node_id in &source.current_node_ids {
                let node = self.nodes.get(node_id).ok_or(ModelError::MissingEntity)?;
                if node.source_id != source.id || node.source_generation != source.generation {
                    return Err(ModelError::InvalidReference);
                }
            }
            for node in self.nodes.values().filter(|node| {
                node.source_id == source.id && node.source_generation == source.generation
            }) {
                if !source.current_node_ids.contains(&node.id) {
                    return Err(ModelError::InvalidReference);
                }
            }
        }
        for node in self.nodes.values() {
            let source = self
                .sources
                .get(&node.source_id)
                .ok_or(ModelError::MissingEntity)?;
            if node.source_generation > source.generation {
                return Err(ModelError::InvalidGeneration);
            }
            for credential_ref in &node.credential_refs {
                self.credentials
                    .get(credential_ref)
                    .ok_or(ModelError::MissingEntity)?;
            }
        }
        for credential in self.credentials.values() {
            let source_ref = self.sources.values().any(|source| {
                source
                    .credential
                    .as_ref()
                    .is_some_and(|metadata| metadata.credential_ref == credential.credential_ref)
            });
            let node_ref = self
                .nodes
                .values()
                .any(|node| node.credential_refs.contains(&credential.credential_ref));
            let pending_ref = self.staged_removals.values().any(|removal| {
                removal.stage != RemovalStage::Complete
                    && removal
                        .credential_metadata
                        .iter()
                        .any(|metadata| metadata.credential_ref == credential.credential_ref)
                    && !removal
                        .removed_credential_refs
                        .contains(&credential.credential_ref)
            });
            let owner_source_live = self.sources.contains_key(&credential.owner_source_id);
            if !source_ref && !node_ref && !pending_ref && !owner_source_live {
                return Err(ModelError::InvalidCredentialMetadata);
            }
        }
        for profile in self.connection_profiles.values() {
            let source = self
                .sources
                .get(&profile.source_id)
                .ok_or(ModelError::MissingEntity)?;
            if let NodeSelection::Pinned { node } = &profile.node_selection {
                let selected = self
                    .nodes
                    .get(&node.node_id)
                    .ok_or(ModelError::MissingEntity)?;
                if node.source_id != profile.source_id
                    || selected.source_id != node.source_id
                    || selected.source_generation != node.source_generation
                    || node.source_generation == 0
                {
                    return Err(ModelError::InvalidReference);
                }
                let _ = source;
            }
        }

        for tunnel in self.tunnel_instances.values() {
            self.connection_profiles
                .get(&tunnel.connection_profile_id)
                .ok_or(ModelError::MissingEntity)?;
            match &tunnel.owner {
                TunnelOwner::Host => {
                    if tunnel.lifecycle != TunnelLifecycle::Stopped {
                        let host = self.host_policy.as_ref().ok_or(ModelError::MissingEntity)?;
                        if host.tunnel_instance_id.as_ref() != Some(&tunnel.id)
                            || host.connection_profile_id.as_ref()
                                != Some(&tunnel.connection_profile_id)
                        {
                            return Err(ModelError::InvalidOwnership);
                        }
                    }
                }
                TunnelOwner::Application { application_id } => {
                    let app = self
                        .applications
                        .get(application_id)
                        .ok_or(ModelError::MissingEntity)?;
                    if app.assignment
                        != (ApplicationAssignment::OwnTunnel {
                            tunnel_instance_id: tunnel.id.clone(),
                        })
                    {
                        return Err(ModelError::InvalidOwnership);
                    }
                }
                TunnelOwner::Group { group_id } => {
                    let group = self
                        .application_groups
                        .get(group_id)
                        .ok_or(ModelError::MissingEntity)?;
                    if group.tunnel_instance_id != tunnel.id {
                        return Err(ModelError::InvalidOwnership);
                    }
                }
            }
        }
        if let Some(host) = &self.host_policy
            && let (Some(profile_id), Some(tunnel_id)) =
                (&host.connection_profile_id, &host.tunnel_instance_id)
        {
            let tunnel = self
                .tunnel_instances
                .get(tunnel_id)
                .ok_or(ModelError::MissingEntity)?;
            if tunnel.owner != TunnelOwner::Host || &tunnel.connection_profile_id != profile_id {
                return Err(ModelError::InvalidOwnership);
            }
        }

        for app in self.applications.values() {
            self.data_profiles
                .get(&app.data_profile_id)
                .ok_or(ModelError::MissingEntity)?;
            self.environment_presets
                .get(&app.environment_preset_id)
                .ok_or(ModelError::MissingEntity)?;
            match &app.assignment {
                ApplicationAssignment::OwnTunnel { tunnel_instance_id } => {
                    let tunnel = self
                        .tunnel_instances
                        .get(tunnel_instance_id)
                        .ok_or(ModelError::MissingEntity)?;
                    if tunnel.owner
                        != (TunnelOwner::Application {
                            application_id: app.id.clone(),
                        })
                    {
                        return Err(ModelError::InvalidOwnership);
                    }
                }
                ApplicationAssignment::Group { group_id } => {
                    let group = self
                        .application_groups
                        .get(group_id)
                        .ok_or(ModelError::MissingEntity)?;
                    let tunnel = self
                        .tunnel_instances
                        .get(&group.tunnel_instance_id)
                        .ok_or(ModelError::MissingEntity)?;
                    if tunnel.owner
                        != (TunnelOwner::Group {
                            group_id: group.id.clone(),
                        })
                    {
                        return Err(ModelError::InvalidOwnership);
                    }
                }
            }
        }
        for group in self.application_groups.values() {
            let tunnel = self
                .tunnel_instances
                .get(&group.tunnel_instance_id)
                .ok_or(ModelError::MissingEntity)?;
            if tunnel.owner
                != (TunnelOwner::Group {
                    group_id: group.id.clone(),
                })
            {
                return Err(ModelError::InvalidOwnership);
            }
        }

        let mut identity_by_data = BTreeSet::new();
        for identity in self.browser_identity_profiles.values() {
            self.data_profiles
                .get(&identity.data_profile_id)
                .ok_or(ModelError::MissingEntity)?;
            if !identity_by_data.insert(identity.data_profile_id.clone()) {
                return Err(ModelError::DuplicateId);
            }
        }

        for profile in self.environment_profiles.values() {
            let session = self
                .sessions
                .get(&profile.session_id)
                .ok_or(ModelError::MissingEntity)?;
            self.environment_presets
                .get(&profile.preset_id)
                .ok_or(ModelError::MissingEntity)?;
            if session.environment_profile_id != profile.id {
                return Err(ModelError::InvalidReference);
            }
        }
        for session in self.sessions.values() {
            let app = self
                .applications
                .get(&session.application_id)
                .ok_or(ModelError::MissingEntity)?;
            if app.data_profile_id != session.data_profile_id {
                return Err(ModelError::InvalidReference);
            }
            let environment = self
                .environment_profiles
                .get(&session.environment_profile_id)
                .ok_or(ModelError::MissingEntity)?;
            if environment.session_id != session.id {
                return Err(ModelError::InvalidReference);
            }
            if session.lifecycle == SessionLifecycle::Active {
                let tunnel = self
                    .tunnel_instances
                    .get(&session.tunnel_instance_id)
                    .ok_or(ModelError::MissingEntity)?;
                let profile = self
                    .connection_profiles
                    .get(&tunnel.connection_profile_id)
                    .ok_or(ModelError::MissingEntity)?;
                let source = self
                    .sources
                    .get(&session.source_id)
                    .ok_or(ModelError::MissingEntity)?;
                let node = self
                    .nodes
                    .get(&session.node_id)
                    .ok_or(ModelError::MissingEntity)?;
                if session.tunnel_generation > tunnel.generation
                    || node.source_id != session.source_id
                    || node.source_generation != session.source_generation
                    || source.id != session.source_id
                {
                    return Err(ModelError::InvalidReference);
                }
                let _ = profile;
                match &app.assignment {
                    ApplicationAssignment::OwnTunnel { tunnel_instance_id }
                        if tunnel_instance_id != &session.tunnel_instance_id =>
                    {
                        return Err(ModelError::InvalidReference);
                    }
                    ApplicationAssignment::Group { group_id } => {
                        let group = self
                            .application_groups
                            .get(group_id)
                            .ok_or(ModelError::MissingEntity)?;
                        if group.tunnel_instance_id != session.tunnel_instance_id {
                            return Err(ModelError::InvalidReference);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut verification_axes = BTreeSet::new();
        for verification in self.verifications.values() {
            let session = self
                .sessions
                .get(&verification.session_id)
                .ok_or(ModelError::MissingEntity)?;
            if (verification.value != VerificationValue::Unknown
                && verification.evidence_at_unix_ms < session.created_at_unix_ms)
                || session.tunnel_instance_id != verification.tunnel_instance_id
                || session.tunnel_generation != verification.tunnel_generation
                || !verification_axes.insert((verification.session_id.clone(), verification.axis))
            {
                return Err(ModelError::InvalidVerification);
            }
        }
        for removal in self.staged_removals.values() {
            if !self.issued_ids.contains(&removal.source_id) {
                return Err(ModelError::InvalidRemovalStage);
            }
            for metadata in &removal.credential_metadata {
                if self.issued_id_kinds.get(&metadata.credential_ref)
                    != Some(&EntityIdKind::CredentialRef)
                {
                    return Err(ModelError::InvalidRemovalStage);
                }
                let ref_is_live = self.sources.values().any(|source| {
                    source
                        .credential
                        .as_ref()
                        .is_some_and(|item| item.credential_ref == metadata.credential_ref)
                }) || self
                    .nodes
                    .values()
                    .any(|node| node.credential_refs.contains(&metadata.credential_ref));
                if removal.stage == RemovalStage::BlobPending && ref_is_live {
                    return Err(ModelError::InvalidRemovalStage);
                }
                if removal
                    .removed_credential_refs
                    .contains(&metadata.credential_ref)
                {
                    if self.credentials.contains_key(&metadata.credential_ref) || ref_is_live {
                        return Err(ModelError::InvalidRemovalStage);
                    }
                } else if let Some(current) = self.credentials.get(&metadata.credential_ref) {
                    if current != metadata {
                        return Err(ModelError::InvalidRemovalStage);
                    }
                    if removal.stage == RemovalStage::Complete && !ref_is_live {
                        return Err(ModelError::InvalidRemovalStage);
                    }
                } else {
                    let removed_by_another_completed_plan =
                        self.staged_removals.values().any(|other| {
                            other.stage == RemovalStage::Complete
                                && other
                                    .removed_credential_refs
                                    .contains(&metadata.credential_ref)
                        });
                    if !removed_by_another_completed_plan {
                        return Err(ModelError::InvalidRemovalStage);
                    }
                }
            }
            let expected = source_removal_candidates(self, &removal.source_id);
            if removal.stage == RemovalStage::LinksPending
                && removal.credential_metadata != expected
            {
                return Err(ModelError::InvalidRemovalStage);
            }
            if removal.stage == RemovalStage::BlobPending {
                for credential_id in &removal.removed_credential_refs {
                    if self.sources.values().any(|source| {
                        source
                            .credential
                            .as_ref()
                            .is_some_and(|meta| &meta.credential_ref == credential_id)
                    }) || self
                        .nodes
                        .values()
                        .any(|node| node.credential_refs.contains(credential_id))
                    {
                        return Err(ModelError::InvalidRemovalStage);
                    }
                }
            }
        }
        Ok(())
    }
}

/// Validate a complete graph transition; equivalent to `GraphSnapshot::validate_transition`.
pub fn validate_transition(old: &GraphSnapshot, new: &GraphSnapshot) -> ModelResult<()> {
    GraphSnapshot::validate_transition(old, new)
}

/// Return the next Source generation, keeping a no-op update at the same generation.
pub fn next_source_generation(
    current_generation: u64,
    old_content_digest_sha256: &str,
    new_content_digest_sha256: &str,
) -> ModelResult<u64> {
    if current_generation == 0
        || !is_sha256(old_content_digest_sha256)
        || !is_sha256(new_content_digest_sha256)
    {
        return Err(ModelError::InvalidGeneration);
    }
    if old_content_digest_sha256 == new_content_digest_sha256 {
        Ok(current_generation)
    } else {
        current_generation
            .checked_add(1)
            .ok_or(ModelError::InvalidGeneration)
    }
}

/// Hash definition bytes at the ingestion boundary without retaining or returning them.
pub fn node_definition_digest(canonical_definition_bytes: &[u8]) -> String {
    hex_sha256(canonical_definition_bytes)
}

pub fn verify_node_reference<'a>(
    graph: &'a GraphSnapshot,
    reference: &SourceNodeRef,
) -> ModelResult<&'a Node> {
    graph.validate()?;
    let node = graph
        .nodes
        .get(&reference.node_id)
        .ok_or(ModelError::MissingEntity)?;
    if node.source_id != reference.source_id
        || node.source_generation != reference.source_generation
    {
        return Err(ModelError::InvalidReference);
    }
    Ok(node)
}

fn session_pins(session: &Session) -> (&Id, &Id, &Id, &Id, u64, &Id, u64, &Id) {
    (
        &session.application_id,
        &session.data_profile_id,
        &session.environment_profile_id,
        &session.tunnel_instance_id,
        session.tunnel_generation,
        &session.source_id,
        session.source_generation,
        &session.node_id,
    )
}

fn removal_stage_advances(old: RemovalStage, new: RemovalStage) -> bool {
    matches!(
        (old, new),
        (
            RemovalStage::LinksPending,
            RemovalStage::LinksPending | RemovalStage::BlobPending | RemovalStage::Complete
        ) | (
            RemovalStage::BlobPending,
            RemovalStage::BlobPending | RemovalStage::Complete
        ) | (RemovalStage::Complete, RemovalStage::Complete)
    )
}

fn add_claim(
    claims: &mut BTreeMap<Id, EntityIdKind>,
    id: Id,
    kind: EntityIdKind,
) -> ModelResult<()> {
    validate_id(&id)?;
    if let Some(existing) = claims.get(&id) {
        if *existing == kind && kind == EntityIdKind::CredentialRef {
            return Ok(());
        }
        return Err(ModelError::DuplicateId);
    }
    claims.insert(id, kind);
    Ok(())
}

fn entity_kind(name: &str) -> EntityIdKind {
    match name {
        "source" => EntityIdKind::Source,
        "node" => EntityIdKind::Node,
        "connection_profile" => EntityIdKind::ConnectionProfile,
        "tunnel_instance" => EntityIdKind::TunnelInstance,
        "data_profile" => EntityIdKind::DataProfile,
        "application" => EntityIdKind::Application,
        "application_group" => EntityIdKind::ApplicationGroup,
        "environment_preset" => EntityIdKind::EnvironmentPreset,
        "environment_profile" => EntityIdKind::EnvironmentProfile,
        "browser_identity_profile" => EntityIdKind::BrowserIdentityProfile,
        "session" => EntityIdKind::Session,
        "verification" => EntityIdKind::Verification,
        _ => unreachable!("identity claim kind is a module constant"),
    }
}

pub(crate) fn source_removal_candidates(
    graph: &GraphSnapshot,
    source_id: &Id,
) -> Vec<CredentialMetadata> {
    graph
        .credentials
        .values()
        .filter(|metadata| {
            let owned_by_source = &metadata.owner_source_id == source_id;
            let referenced_by_source = graph.sources.get(source_id).is_some_and(|source| {
                source
                    .credential
                    .as_ref()
                    .is_some_and(|candidate| candidate.credential_ref == metadata.credential_ref)
            }) || graph.nodes.values().any(|node| {
                &node.source_id == source_id
                    && node.credential_refs.contains(&metadata.credential_ref)
            });
            let owned_by_other_live_source = &metadata.owner_source_id != source_id
                && graph.sources.contains_key(&metadata.owner_source_id);
            let referenced_by_others = graph.sources.values().any(|source| {
                &source.id != source_id
                    && source.credential.as_ref().is_some_and(|candidate| {
                        candidate.credential_ref == metadata.credential_ref
                    })
            }) || graph.nodes.values().any(|node| {
                &node.source_id != source_id
                    && node.credential_refs.contains(&metadata.credential_ref)
            }) || graph.staged_removals.values().any(|removal| {
                &removal.source_id != source_id
                    && removal.stage != RemovalStage::Complete
                    && removal
                        .credential_metadata
                        .iter()
                        .any(|candidate| candidate.credential_ref == metadata.credential_ref)
            });
            !owned_by_other_live_source
                && !referenced_by_others
                && (owned_by_source || referenced_by_source)
        })
        .cloned()
        .collect()
}

fn axis_record<'a>(
    graph: &'a GraphSnapshot,
    session_id: &Id,
    axis: VerificationAxis,
) -> Option<&'a Verification> {
    graph
        .verifications
        .values()
        .find(|record| &record.session_id == session_id && record.axis == axis)
}

fn session_axis_evidence(
    graph: &GraphSnapshot,
    session_id: &Id,
    axis: VerificationAxis,
) -> Option<i64> {
    axis_record(graph, session_id, axis).map(|record| record.evidence_at_unix_ms)
}

fn session_axes(graph: &GraphSnapshot, session_id: &Id) -> VerificationAxes {
    let mut axes = VerificationAxes::default();
    for record in graph
        .verifications
        .values()
        .filter(|record| &record.session_id == session_id)
    {
        axes.set(record.axis, record.value);
    }
    axes
}

fn ensure_unique_ids(ids: &[Id]) -> ModelResult<()> {
    let mut seen = BTreeSet::new();
    if ids.iter().any(|id| !seen.insert(id)) {
        return Err(ModelError::DuplicateId);
    }
    Ok(())
}

fn require_schema(version: u32) -> ModelResult<()> {
    if version == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(ModelError::UnsupportedSchemaVersion)
    }
}

fn deserialize_schema_version<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let version = u32::deserialize(deserializer)?;
    if version == SCHEMA_VERSION {
        Ok(version)
    } else {
        Err(de::Error::custom("unsupported schema version"))
    }
}

fn validate_id(id: &Id) -> ModelResult<()> {
    if id.0.is_empty()
        || id.0.len() > MAX_ID_BYTES
        || !id
            .0
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(ModelError::InvalidId);
    }
    Ok(())
}

fn is_sha256(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_bounded_core_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 32
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'v'))
}

fn is_git_commit(commit: &str) -> bool {
    commit.len() == 40
        && commit.bytes().all(|byte| {
            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) || (b'A'..=b'F').contains(&byte)
        })
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

// Dictionary/registry duplicates are ambiguous state, not last-writer-wins input.
fn deserialize_unique_map<'de, D, V>(deserializer: D) -> Result<BTreeMap<Id, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct MapVisitor<V>(std::marker::PhantomData<V>);
    impl<'de, V: Deserialize<'de>> de::Visitor<'de> for MapVisitor<V> {
        type Value = BTreeMap<Id, V>;
        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a map with unique identifiers")
        }
        fn visit_map<A: de::MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
            let mut records = BTreeMap::new();
            while let Some(key) = access.next_key::<Id>()? {
                if records.contains_key(&key) {
                    return Err(de::Error::custom("duplicate graph identifier"));
                }
                records.insert(key, access.next_value::<V>()?);
            }
            Ok(records)
        }
    }
    deserializer.deserialize_map(MapVisitor(std::marker::PhantomData))
}

fn deserialize_unique_set<'de, D>(deserializer: D) -> Result<BTreeSet<Id>, D::Error>
where
    D: Deserializer<'de>,
{
    struct SetVisitor;
    impl<'de> de::Visitor<'de> for SetVisitor {
        type Value = BTreeSet<Id>;
        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a registry of unique identifiers")
        }
        fn visit_seq<A: de::SeqAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
            let mut ids = BTreeSet::new();
            while let Some(id) = access.next_element::<Id>()? {
                if !ids.insert(id) {
                    return Err(de::Error::custom("duplicate registry identifier"));
                }
            }
            Ok(ids)
        }
    }
    deserializer.deserialize_seq(SetVisitor)
}
