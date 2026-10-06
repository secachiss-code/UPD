//! Import omission summaries, confirmation digests, and automatic refresh policy.

use super::artifact::{ArtifactError, SourceImportInput};
use crate::profiles::{ImportOmissions, Source, import_omissions_digest, omissions_within_bound};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingImportSummary {
    pub omissions: ImportOmissions,
    pub omissions_digest_sha256: String,
}

pub fn pending_import_summary(
    omissions: ImportOmissions,
) -> Result<PendingImportSummary, ArtifactError> {
    omissions
        .validate()
        .map_err(|_| ArtifactError::InvalidProvenance)?;
    let omissions_digest_sha256 =
        import_omissions_digest(&omissions).map_err(|_| ArtifactError::InvalidProvenance)?;
    Ok(PendingImportSummary {
        omissions,
        omissions_digest_sha256,
    })
}

/// Attach omissions and the user's confirmation. The digest is checked against the omissions
/// as the input records them, including the disabled-TLS count derived from the nodes.
pub fn apply_confirmation(
    input: SourceImportInput,
    omissions: ImportOmissions,
    confirm_digest: &str,
) -> Result<SourceImportInput, ArtifactError> {
    let input = input.with_omissions(omissions);
    let expected =
        import_omissions_digest(input.omissions()).map_err(|_| ArtifactError::InvalidProvenance)?;
    if confirm_digest != expected {
        return Err(ArtifactError::OmissionsDigestMismatch);
    }
    Ok(input.with_omissions_confirmation_digest(Some(expected)))
}

/// Pre-publication policy for unattended refresh: no empty source and no drop of more than
/// half of the nodes. Whether omissions stay within the confirmed bound is checked at
/// publication against the stored provenance, not against caller-supplied data.
pub fn evaluate_auto_refresh(
    old_source: &Source,
    new_input: &SourceImportInput,
) -> Result<(), ArtifactError> {
    if new_input.node_definition_count() == 0 {
        return Err(ArtifactError::AutoUpdateBlocked);
    }
    let old_count = old_source.current_node_ids.len();
    let new_count = new_input.node_definition_count();
    if old_count > 0 && new_count * 2 < old_count {
        return Err(ArtifactError::AutoUpdateBlocked);
    }
    let bound = old_source
        .provenance
        .as_ref()
        .map(|provenance| &provenance.accepted_omissions_bound)
        .ok_or(ArtifactError::SourceArtifactMissing)?;
    if !omissions_within_bound(new_input.omissions(), bound) {
        return Err(ArtifactError::AutoUpdateBlocked);
    }
    Ok(())
}
