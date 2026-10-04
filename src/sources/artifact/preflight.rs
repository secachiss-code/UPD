//! Reject oversized aggregate inputs before hashing every individual definition.

use super::{
    ArtifactError, ArtifactTransport, DefaultField, MAX_ARTIFACT_NODE_COUNT,
    MAX_SERIALIZED_ARTIFACT_BYTES, SourceImportInput, map_bounded_error,
};
use crate::profiles::NodeProtocol;
use crate::sources::bounded::encode_json_bounded;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Serialize)]
struct PayloadProjection<'a> {
    raw_body: &'a [u8],
    actual_user_agent: Option<&'a str>,
    fetch_settings: Option<&'a crate::sources::negotiation::PrivateFetchSettings>,
    defaults: &'a BTreeMap<DefaultField, Value>,
    definitions: Vec<DefinitionProjection<'a>>,
}

#[derive(Serialize)]
struct DefinitionProjection<'a> {
    protocol: NodeProtocol,
    transport: ArtifactTransport,
    definition: &'a Value,
}

pub(super) fn validate_payload_budget(input: &SourceImportInput) -> Result<(), ArtifactError> {
    if input.definitions.len() > MAX_ARTIFACT_NODE_COUNT {
        return Err(ArtifactError::TooManyNodes);
    }
    let projection = PayloadProjection {
        raw_body: &input.raw_body,
        actual_user_agent: input.actual_user_agent.as_deref(),
        fetch_settings: input.fetch_settings.as_ref(),
        defaults: &input.defaults.0,
        definitions: input
            .definitions
            .iter()
            .map(|definition| DefinitionProjection {
                protocol: definition.protocol,
                transport: definition.transport.into(),
                definition: &definition.full_definition,
            })
            .collect(),
    };
    // Borrow the trees and stop encoding at the limit; no payload clones or hashes
    // are needed. Final artifact encoding also bounds IDs, digests, and metadata.
    drop(
        encode_json_bounded(&projection, MAX_SERIALIZED_ARTIFACT_BYTES)
            .map_err(map_bounded_error)?,
    );
    Ok(())
}
