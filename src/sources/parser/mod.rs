//! Pure, bounded import parsers. No parser performs I/O or Store publication.

mod list;
mod native;
mod uri;
mod yaml_guard;

pub use list::{BodyKind, detect_body, parse_source, parse_uri_list};
pub use uri::parse_share_uri;

pub use native::{
    MAX_NATIVE_DEPTH, MAX_NATIVE_ENTRIES, MAX_NATIVE_NODES, ParsedSource, ParserError,
    audit_fixture_opts_digest, audit_parse_strict_json, audit_validate_fixture_opts, parse_native,
};

/// Validate one node object (manual entry) exactly as an imported node.
pub(crate) fn validate_single_node(
    core_version: &str,
    node: serde_json::Value,
) -> Result<(String, crate::sources::artifact::NodeDefinitionInput), ParserError> {
    if core_version.strip_prefix('v').unwrap_or(core_version)
        != crate::sources::capabilities::PINNED_CORE_VERSION
    {
        return Err(ParserError::UnsupportedCoreVersion);
    }
    native::node_definition(core_version, 0, node)
}
