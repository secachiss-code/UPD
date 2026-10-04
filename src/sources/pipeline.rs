//! Publication of strictly parsed, accepted responses. No implicit fetch or conversion.

use super::artifact::{self, ArtifactError, SourceImportInput, SourceImportReceipt};
use super::capabilities::ImportFormat;
use super::negotiation::{
    self, BodyRejection, CachedWinner, ConfiguredEndpoint, FetchFailure, HttpResponse, Negotiated,
    NegotiationError, NegotiationPolicy, RequestSpec, UserAgent,
};
use super::parser::{ParsedSource, ParserError, parse_native};
use crate::profiles::{Id, Store};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FetchSourceError {
    Negotiation(NegotiationError),
    Parser(ParserError),
}

impl std::fmt::Display for FetchSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Negotiation(error) => std::fmt::Display::fmt(error, f),
            Self::Parser(error) => std::fmt::Display::fmt(error, f),
        }
    }
}
impl std::error::Error for FetchSourceError {}

/// Explicit native format with an injected bounded transport. No provider discovery.
/// The callback must enforce RequestSpec's deadline, size and redirect contract.
pub fn negotiate_native_source<F>(
    endpoint: &ConfiguredEndpoint,
    core_version: &str,
    format: ImportFormat,
    preferred: &[UserAgent],
    configured: &[UserAgent],
    cached: Option<&CachedWinner>,
    policy: &NegotiationPolicy,
    fetch: F,
) -> Result<Negotiated<ParsedSource>, FetchSourceError>
where
    F: for<'a> FnMut(RequestSpec<'a>) -> Result<HttpResponse, FetchFailure>,
{
    if !matches!(format, ImportFormat::MihomoYaml | ImportFormat::MihomoJson) {
        return Err(FetchSourceError::Parser(ParserError::UnsupportedFormat));
    }
    if policy.max_body_bytes > super::artifact::MAX_RAW_SOURCE_BYTES {
        return Err(FetchSourceError::Negotiation(
            NegotiationError::InvalidPolicy,
        ));
    }
    let mut terminal_parse_error = None;
    let result = negotiation::negotiate(
        endpoint,
        core_version,
        preferred,
        configured,
        cached,
        policy,
        fetch,
        |bytes| match parse_native(core_version, format, bytes) {
            Ok(parsed) => Ok(parsed),
            Err(
                ParserError::InvalidSyntax
                | ParserError::InvalidUtf8
                | ParserError::InvalidTopLevel
                | ParserError::MissingProxies
                | ParserError::EmptyProxies,
            ) => Err(BodyRejection::RetryableUnusableBody),
            Err(error) => {
                terminal_parse_error = Some(error);
                Err(BodyRejection::TerminalBody)
            }
        },
    );
    match result {
        Ok(accepted) => Ok(accepted),
        Err(NegotiationError::TerminalBody) => Err(FetchSourceError::Parser(
            terminal_parse_error.unwrap_or(ParserError::InvalidSyntax),
        )),
        Err(error) => Err(FetchSourceError::Negotiation(error)),
    }
}

/// Move the response once after checking that the parser validated these exact bytes.
pub fn accepted_import_input(
    accepted: Negotiated<ParsedSource>,
) -> Result<SourceImportInput, ArtifactError> {
    if accepted.parsed().source_body_sha256() != accepted.source_body_sha256() {
        return Err(ArtifactError::InvalidInput);
    }
    SourceImportInput::from_owned_negotiated(
        accepted.into_import_payload(),
        ParsedSource::into_parts,
    )
}

pub fn create_accepted_source(
    store: &Store,
    expected_revision: u64,
    accepted: Negotiated<ParsedSource>,
) -> Result<SourceImportReceipt, ArtifactError> {
    let input = accepted_import_input(accepted)?;
    artifact::create_source(store, expected_revision, input)
}

pub fn update_accepted_source(
    store: &Store,
    source_id: &Id,
    expected_revision: u64,
    expected_source_generation: u64,
    accepted: Negotiated<ParsedSource>,
    evidence_at_unix_ms: i64,
) -> Result<SourceImportReceipt, ArtifactError> {
    let input = accepted_import_input(accepted)?;
    artifact::update_source(
        store,
        source_id,
        expected_revision,
        expected_source_generation,
        input,
        evidence_at_unix_ms,
    )
}
