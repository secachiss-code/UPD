//! Source capabilities, bounded negotiation, and private artifact publication.
//!

pub mod artifact;
mod bounded;
pub mod capabilities;
pub mod cli;
pub mod import_confirmation;
pub mod manual;
pub mod negotiation;
pub mod parser;
pub mod pipeline;
pub mod transport;

pub use artifact::{
    ArtifactError, GlobalDefaults, MAX_ARTIFACT_DEPTH, MAX_ARTIFACT_NODE_COUNT,
    MAX_RAW_SOURCE_BYTES, MAX_SERIALIZED_ARTIFACT_BYTES, NodeDefinitionInput,
    ResolvedNodeDefinition, SourceArtifact, SourceImportInput, SourceImportReceipt, create_source,
    read_node_definition, read_source_artifact, update_source,
};

pub use capabilities::{
    CapabilityError, ImportFormat, NativeFieldDisposition, NodeOptionDisposition,
    PINNED_CORE_COMMIT, PINNED_CORE_VERSION, ProviderMode, Transport, UriSchemeCapability,
    classify_native_config_field, classify_node_option, classify_provider_mode,
    protocol_for_uri_scheme, supported_import_formats, supported_transports, supported_uri_schemes,
    validate,
};

pub use negotiation::{
    BodyRejection, CachedWinner, ConfiguredEndpoint, FetchFailure, HttpResponse, Negotiated,
    NegotiationError, NegotiationPolicy, RequestSpec, UserAgent, negotiate, negotiate_with_clock,
};

pub use import_confirmation::{
    PendingImportSummary, apply_confirmation, evaluate_auto_refresh, pending_import_summary,
};

pub use transport::FetchTransport;

pub use parser::{
    BodyKind, MAX_NATIVE_DEPTH, MAX_NATIVE_ENTRIES, MAX_NATIVE_NODES, ParsedSource, ParserError,
    detect_body, parse_native, parse_share_uri, parse_source, parse_uri_list,
};
