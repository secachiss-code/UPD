//! Publication of strictly parsed, accepted responses. No implicit fetch or conversion.

use super::artifact::{self, ArtifactError, SourceImportInput, SourceImportReceipt};
use super::capabilities::ImportFormat;
use super::import_confirmation::{
    PendingImportSummary, apply_confirmation, evaluate_auto_refresh, pending_import_summary,
};
use super::negotiation::{
    self, BodyRejection, CachedWinner, ConfiguredEndpoint, FetchFailure, HttpResponse, Negotiated,
    NegotiationError, NegotiationPolicy, RequestSpec, UserAgent,
};
use super::parser::{ParsedSource, ParserError, parse_native, parse_source};
use crate::profiles::{Id, Store};

#[derive(Debug)]
pub enum FetchSourceError {
    Negotiation(NegotiationError),
    Parser(ParserError),
    Artifact(ArtifactError),
}

impl std::fmt::Display for FetchSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Negotiation(error) => std::fmt::Display::fmt(error, f),
            Self::Parser(error) => std::fmt::Display::fmt(error, f),
            Self::Artifact(error) => std::fmt::Display::fmt(error, f),
        }
    }
}
impl std::error::Error for FetchSourceError {}

impl From<ArtifactError> for FetchSourceError {
    fn from(error: ArtifactError) -> Self {
        Self::Artifact(error)
    }
}

impl From<crate::profiles::StoreError> for FetchSourceError {
    fn from(error: crate::profiles::StoreError) -> Self {
        Self::Artifact(ArtifactError::from(error))
    }
}

/// Explicit native format with an injected bounded transport. No provider discovery.
/// The callback must enforce RequestSpec's deadline, size and redirect contract.
#[allow(clippy::too_many_arguments)] // mirrors negotiate() for native-only import entry
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
    negotiate_with_parser(
        endpoint,
        core_version,
        preferred,
        configured,
        cached,
        policy,
        fetch,
        |bytes| parse_native(core_version, format, bytes),
    )
}

/// Any supported format (native YAML/JSON, URI list, base64 list), detected from the body.
///
/// Per I03.T04.t: an HTML page or provider stub, an unusable or empty-of-nodes body is
/// retried with the next User-Agent; an empty body, an oversized body, too deep encoding and
/// unsupported semantics (host controls, unsupported fields) are terminal at once.
#[allow(clippy::too_many_arguments)] // mirrors negotiate()
pub fn negotiate_source<F>(
    endpoint: &ConfiguredEndpoint,
    core_version: &str,
    preferred: &[UserAgent],
    configured: &[UserAgent],
    cached: Option<&CachedWinner>,
    policy: &NegotiationPolicy,
    fetch: F,
) -> Result<Negotiated<ParsedSource>, FetchSourceError>
where
    F: for<'a> FnMut(RequestSpec<'a>) -> Result<HttpResponse, FetchFailure>,
{
    negotiate_with_parser(
        endpoint,
        core_version,
        preferred,
        configured,
        cached,
        policy,
        fetch,
        |bytes| parse_source(core_version, bytes),
    )
}

#[allow(clippy::too_many_arguments)]
fn negotiate_with_parser<F, P>(
    endpoint: &ConfiguredEndpoint,
    core_version: &str,
    preferred: &[UserAgent],
    configured: &[UserAgent],
    cached: Option<&CachedWinner>,
    policy: &NegotiationPolicy,
    fetch: F,
    mut parse: P,
) -> Result<Negotiated<ParsedSource>, FetchSourceError>
where
    F: for<'a> FnMut(RequestSpec<'a>) -> Result<HttpResponse, FetchFailure>,
    P: FnMut(&[u8]) -> Result<ParsedSource, ParserError>,
{
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
        |bytes| {
            if bytes.trim_ascii().is_empty() {
                // An empty answer is not a different format for another User-Agent.
                terminal_parse_error = Some(ParserError::InvalidSyntax);
                return Err(BodyRejection::TerminalBody);
            }
            match parse(bytes) {
                Ok(parsed) => Ok(parsed),
                Err(error) if is_retryable(error) => Err(BodyRejection::RetryableUnusableBody),
                Err(error) => {
                    terminal_parse_error = Some(error);
                    Err(BodyRejection::TerminalBody)
                }
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

/// Bodies that another User-Agent may legitimately answer differently.
fn is_retryable(error: ParserError) -> bool {
    matches!(
        error,
        ParserError::InvalidSyntax
            | ParserError::InvalidUtf8
            | ParserError::InvalidTopLevel
            | ParserError::MissingProxies
            | ParserError::EmptyProxies
            | ParserError::UnusableBody
    )
}

/// Move the response once after checking that the parser validated these exact bytes.
pub fn accepted_import_input(
    accepted: Negotiated<ParsedSource>,
) -> Result<SourceImportInput, ArtifactError> {
    if accepted.parsed().source_body_sha256() != accepted.source_body_sha256() {
        return Err(ArtifactError::InvalidInput);
    }
    let omissions = accepted.parsed().omissions().clone();
    Ok(SourceImportInput::from_owned_negotiated(
        accepted.into_import_payload(),
        ParsedSource::into_parts,
    )?
    .with_omissions(omissions))
}

#[derive(Debug)]
pub enum ImportPublishOutcome {
    Published(SourceImportReceipt),
    PendingConfirmation(super::import_confirmation::PendingImportSummary),
}

pub fn create_accepted_source(
    store: &Store,
    expected_revision: u64,
    accepted: Negotiated<ParsedSource>,
) -> Result<SourceImportReceipt, ArtifactError> {
    let input = accepted_import_input(accepted)?;
    artifact::create_source(store, expected_revision, input)
}

/// Publish a new source. Omissions come from the parser; when they need confirmation and
/// `confirm_digest` is absent, nothing is written and the summary to confirm is returned.
pub fn create_accepted_source_with_omissions(
    store: &Store,
    expected_revision: u64,
    accepted: Negotiated<ParsedSource>,
    confirm_digest: Option<&str>,
) -> Result<ImportPublishOutcome, FetchSourceError> {
    let input = accepted_import_input(accepted)?;
    let input = match confirmation_step(input, confirm_digest)? {
        Ok(input) => input,
        Err(summary) => return Ok(ImportPublishOutcome::PendingConfirmation(summary)),
    };
    let receipt = artifact::create_source(store, expected_revision, input)?;
    Ok(ImportPublishOutcome::Published(receipt))
}

/// Attach a user confirmation, or return the pending summary when one is required.
fn confirmation_step(
    input: SourceImportInput,
    confirm_digest: Option<&str>,
) -> Result<Result<SourceImportInput, PendingImportSummary>, FetchSourceError> {
    let omissions = input.omissions().clone();
    if !omissions.requires_confirmation() {
        return Ok(Ok(input));
    }
    match confirm_digest {
        Some(digest) => Ok(Ok(apply_confirmation(input, omissions, digest)?)),
        None => Ok(Err(pending_import_summary(omissions)?)),
    }
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

/// Refresh an existing source. An unattended refresh (`auto_refresh`) never asks: it
/// publishes only within the previously confirmed bound and node-count policy, otherwise the
/// old generation stays and the error explains why.
#[allow(clippy::too_many_arguments)] // explicit CAS revision, generation and evidence time
pub fn update_accepted_source_with_omissions(
    store: &Store,
    source_id: &Id,
    expected_revision: u64,
    expected_source_generation: u64,
    accepted: Negotiated<ParsedSource>,
    confirm_digest: Option<&str>,
    auto_refresh: bool,
    evidence_at_unix_ms: i64,
) -> Result<ImportPublishOutcome, FetchSourceError> {
    let input = accepted_import_input(accepted)?;
    let input = if auto_refresh {
        if confirm_digest.is_some() {
            return Err(ArtifactError::InvalidInput.into());
        }
        let graph = store.read_snapshot()?;
        let old_source = graph
            .sources
            .get(source_id)
            .ok_or(ArtifactError::SourceNotFound)?;
        evaluate_auto_refresh(old_source, &input)?;
        input.as_auto_refresh()
    } else {
        match confirmation_step(input, confirm_digest)? {
            Ok(input) => input,
            Err(summary) => return Ok(ImportPublishOutcome::PendingConfirmation(summary)),
        }
    };
    let receipt = artifact::update_source(
        store,
        source_id,
        expected_revision,
        expected_source_generation,
        input,
        evidence_at_unix_ms,
    )?;
    Ok(ImportPublishOutcome::Published(receipt))
}
