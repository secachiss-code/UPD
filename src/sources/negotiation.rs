//! Bounded User-Agent negotiation for explicitly configured source endpoints.
//!
//! This module has no network client or parser. Callers inject both operations, so
//! policy and retry behavior can be tested without network access.

use super::capabilities::PINNED_CORE_VERSION;
use sha2::{Digest, Sha256};
use std::fmt;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;

const MAX_URL_BYTES: usize = 8 * 1024;
const MAX_USER_AGENT_BYTES: usize = 512;
const MAX_REQUESTS: usize = 8;
const MAX_CANDIDATES: usize = 8;
const MAX_TOTAL_BUDGET: Duration = Duration::from_secs(60);
const MAX_PER_REQUEST: Duration = Duration::from_secs(10);
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_WINNER_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const DEFAULT_TOTAL_BUDGET: Duration = Duration::from_secs(30);
const DEFAULT_PER_REQUEST: Duration = Duration::from_secs(10);
const DEFAULT_MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_WINNER_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// A source endpoint supplied explicitly by configuration.
///
/// The inner URL is deliberately private and is omitted from Debug output. The
/// trusted fetch adapter can retrieve it through [`ConfiguredEndpoint::expose_url`].
pub struct ConfiguredEndpoint {
    url: String,
}

impl ConfiguredEndpoint {
    /// Validate an explicitly configured endpoint. Plain HTTP requires an opt-in.
    pub fn new(url: impl Into<String>, allow_http: bool) -> Result<Self, NegotiationError> {
        let url = url.into();
        if url.is_empty()
            || url.len() > MAX_URL_BYTES
            || url.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(NegotiationError::InvalidEndpoint);
        }
        let parsed = Url::parse(&url).map_err(|_| NegotiationError::InvalidEndpoint)?;
        if parsed.fragment().is_some() {
            return Err(NegotiationError::InvalidEndpoint);
        }
        match parsed.scheme() {
            "https" => {}
            "http" if allow_http => {}
            _ => return Err(NegotiationError::InvalidEndpoint),
        }
        if parsed.host().is_none() {
            return Err(NegotiationError::InvalidEndpoint);
        }
        if parsed.as_str().len() > MAX_URL_BYTES {
            return Err(NegotiationError::InvalidEndpoint);
        }
        Ok(Self { url })
    }

    /// Expose the URL only to a trusted adapter that performs the configured fetch.
    pub fn expose_url(&self) -> &str {
        &self.url
    }
}

impl fmt::Debug for ConfiguredEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfiguredEndpoint")
            .field("url", &"[REDACTED]")
            .finish()
    }
}

/// A validated User-Agent string. Formatting never reveals its value.
pub struct UserAgent(String);

impl UserAgent {
    pub fn new(value: impl Into<String>) -> Result<Self, NegotiationError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_USER_AGENT_BYTES
            || !value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
        {
            return Err(NegotiationError::InvalidUserAgent);
        }
        Ok(Self(value))
    }

    /// Explicitly expose a configured value to a trusted fetch adapter.
    pub fn expose_value(&self) -> &str {
        &self.0
    }
}

impl Clone for UserAgent {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl PartialEq for UserAgent {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for UserAgent {}

impl fmt::Debug for UserAgent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UserAgent([REDACTED])")
    }
}

/// Hard-bounded negotiation limits. Every field is validated before the first fetch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NegotiationPolicy {
    pub max_requests: usize,
    pub max_candidates: usize,
    pub total_budget: Duration,
    pub per_request_timeout: Duration,
    pub max_body_bytes: usize,
    pub winner_ttl: Duration,
}

impl Default for NegotiationPolicy {
    fn default() -> Self {
        Self {
            max_requests: 6,
            max_candidates: 8,
            total_budget: DEFAULT_TOTAL_BUDGET,
            per_request_timeout: DEFAULT_PER_REQUEST,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            winner_ttl: DEFAULT_WINNER_TTL,
        }
    }
}

impl NegotiationPolicy {
    fn validate(&self) -> Result<(), NegotiationError> {
        if self.max_requests == 0
            || self.max_requests > MAX_REQUESTS
            || self.max_candidates == 0
            || self.max_candidates > MAX_CANDIDATES
            || self.total_budget.is_zero()
            || self.total_budget > MAX_TOTAL_BUDGET
            || self.per_request_timeout.is_zero()
            || self.per_request_timeout > MAX_PER_REQUEST
            || self.per_request_timeout > self.total_budget
            || self.max_body_bytes == 0
            || self.max_body_bytes > MAX_BODY_BYTES
            || self.winner_ttl.is_zero()
            || self.winner_ttl > MAX_WINNER_TTL
        {
            return Err(NegotiationError::InvalidPolicy);
        }
        Ok(())
    }
}

/// Safe terminal classifications returned by a body classifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BodyRejection {
    /// The successful response was not a usable subscription/configuration body.
    RetryableUnusableBody,
    /// The classifier identified a terminal body condition.
    TerminalBody,
}

/// Safe failure classes returned by a fetch adapter; raw client errors are not accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FetchFailure {
    Transport,
    Timeout,
    Tls,
    Redirect,
}

/// Negotiation failures contain no URL, response bytes, headers, or parser text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NegotiationError {
    InvalidEndpoint,
    InvalidUserAgent,
    InvalidPolicy,
    InvalidCandidates,
    TooManyCandidates,
    UnsupportedCoreVersion,
    Fetch(FetchFailure),
    AuthenticationRejected,
    QuotaRejected,
    EndpointUnavailable,
    RedirectRejected,
    ServerFailure,
    HttpRejected,
    BodyTooLarge,
    TerminalBody,
    NoUsableBody,
    RequestLimitReached,
    DeadlineExceeded,
    ClockFailure,
    TimestampOverflow,
}

impl fmt::Display for NegotiationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidEndpoint => "invalid configured endpoint",
            Self::InvalidUserAgent => "invalid User-Agent",
            Self::InvalidPolicy => "invalid negotiation policy",
            Self::InvalidCandidates => "no User-Agent candidates configured",
            Self::TooManyCandidates => "User-Agent candidate limit exceeded",
            Self::UnsupportedCoreVersion => "unsupported core version",
            Self::Fetch(FetchFailure::Transport) => "source fetch transport failure",
            Self::Fetch(FetchFailure::Timeout) => "source fetch timeout",
            Self::Fetch(FetchFailure::Tls) => "source fetch TLS failure",
            Self::Fetch(FetchFailure::Redirect) => "source fetch redirect refused",
            Self::AuthenticationRejected => "source authentication rejected",
            Self::QuotaRejected => "source quota or rate limit rejected the request",
            Self::EndpointUnavailable => "source endpoint unavailable",
            Self::RedirectRejected => "source redirect refused",
            Self::ServerFailure => "source server failure",
            Self::HttpRejected => "source HTTP response rejected",
            Self::BodyTooLarge => "source response exceeds configured size limit",
            Self::TerminalBody => "source response body rejected",
            Self::NoUsableBody => "no candidate produced a usable source body",
            Self::RequestLimitReached => "source request limit reached",
            Self::DeadlineExceeded => "source negotiation deadline exceeded",
            Self::ClockFailure => "source negotiation clock failure",
            Self::TimestampOverflow => "source negotiation timestamp overflow",
        })
    }
}

impl std::error::Error for NegotiationError {}

/// A cache record bound to the endpoint, ordered candidate set, core pin, and TTL.
///
/// It deliberately has no serialization and never exposes its endpoint digest or UA
/// through formatting. A future store layer may persist an equivalent private record.
pub struct CachedWinner {
    winner: UserAgent,
    endpoint_digest: [u8; 32],
    agent_set_digest: [u8; 32],
    core_version: String,
    created_at_unix_ms: i64,
    ttl: Duration,
}

impl fmt::Debug for CachedWinner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CachedWinner")
            .field("winner", &"[REDACTED]")
            .field("endpoint_digest", &"[REDACTED]")
            .field("agent_set_digest", &"[REDACTED]")
            .field("core_version", &self.core_version)
            .field("created_at_unix_ms", &self.created_at_unix_ms)
            .field("ttl", &self.ttl)
            .finish()
    }
}

/// A request contract supplied to the injected fetch callback.
#[derive(Clone, Copy)]
pub struct RequestSpec<'a> {
    endpoint: &'a ConfiguredEndpoint,
    user_agent: &'a UserAgent,
    remaining_budget: Duration,
    request_timeout: Duration,
    max_body_bytes: usize,
}

impl<'a> RequestSpec<'a> {
    pub fn endpoint(&self) -> &'a ConfiguredEndpoint {
        self.endpoint
    }

    pub fn user_agent(&self) -> &'a UserAgent {
        self.user_agent
    }

    pub fn remaining_budget(&self) -> Duration {
        self.remaining_budget
    }

    /// Timeout the adapter must enforce for this request.
    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    pub fn max_body_bytes(&self) -> usize {
        self.max_body_bytes
    }

    /// Redirect following is always disabled by this contract.
    pub const fn max_redirects(&self) -> u8 {
        0
    }
}

impl fmt::Debug for RequestSpec<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestSpec")
            .field("endpoint", &"[REDACTED]")
            .field("user_agent", &"[REDACTED]")
            .field("remaining_budget", &self.remaining_budget)
            .field("request_timeout", &self.request_timeout)
            .field("max_body_bytes", &self.max_body_bytes)
            .field("max_redirects", &0)
            .finish()
    }
}

/// A fetch response with private, redacted bytes.
pub struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

impl HttpResponse {
    /// Construct a response inside the trusted fetch adapter or synthetic fixture.
    pub fn new(status: u16, body: Vec<u8>) -> Self {
        Self { status, body }
    }
}

impl fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("body", &"[REDACTED]")
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// Accepted body and parse result. Raw bytes and generic parsed value are private.
pub struct Negotiated<T> {
    body: Vec<u8>,
    parsed: T,
    actual_user_agent: UserAgent,
    source_body_sha256: String,
    accepted_at_unix_ms: i64,
    request_count: usize,
    used_cached_winner: bool,
    endpoint_digest: [u8; 32],
    agent_set_digest: [u8; 32],
    core_version: &'static str,
    winner_ttl: Duration,
}

impl<T> Negotiated<T> {
    /// Explicit access to the accepted raw body for the trusted import pipeline.
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Explicit access to the classifier's accepted typed result.
    pub fn parsed(&self) -> &T {
        &self.parsed
    }

    /// The User-Agent that actually produced the accepted body.
    pub fn actual_user_agent(&self) -> &UserAgent {
        &self.actual_user_agent
    }

    pub fn source_body_sha256(&self) -> &str {
        &self.source_body_sha256
    }

    pub fn accepted_at_unix_ms(&self) -> i64 {
        self.accepted_at_unix_ms
    }

    pub fn request_count(&self) -> usize {
        self.request_count
    }

    pub fn used_cached_winner(&self) -> bool {
        self.used_cached_winner
    }

    /// Bind the accepted UA as a future cache hint. No endpoint URL is retained.
    pub fn make_cache_record(&self) -> CachedWinner {
        CachedWinner {
            winner: self.actual_user_agent.clone(),
            endpoint_digest: self.endpoint_digest,
            agent_set_digest: self.agent_set_digest,
            core_version: self.core_version.to_owned(),
            created_at_unix_ms: self.accepted_at_unix_ms,
            ttl: self.winner_ttl,
        }
    }
}

impl<T> fmt::Debug for Negotiated<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Negotiated")
            .field("body", &"[REDACTED]")
            .field("parsed", &"[REDACTED]")
            .field("actual_user_agent", &"[REDACTED]")
            .field("source_body_sha256", &self.source_body_sha256)
            .field("accepted_at_unix_ms", &self.accepted_at_unix_ms)
            .field("request_count", &self.request_count)
            .field("used_cached_winner", &self.used_cached_winner)
            .field("endpoint_digest", &"[REDACTED]")
            .field("agent_set_digest", &"[REDACTED]")
            .finish()
    }
}

/// Negotiate using a real monotonic clock and the system's current Unix time.
///
/// The injected fetch callback must enforce `RequestSpec::request_timeout` and must
/// not follow redirects. This function cannot preempt an arbitrary blocking callback.
pub fn negotiate<T, F, C>(
    endpoint: &ConfiguredEndpoint,
    core_version: &str,
    preferred: &[UserAgent],
    configured: &[UserAgent],
    cached_winner: Option<&CachedWinner>,
    policy: &NegotiationPolicy,
    fetch: F,
    classify: C,
) -> Result<Negotiated<T>, NegotiationError>
where
    F: for<'a> FnMut(RequestSpec<'a>) -> Result<HttpResponse, FetchFailure>,
    C: FnMut(&[u8]) -> Result<T, BodyRejection>,
{
    let start = Instant::now();
    negotiate_with_clock(
        endpoint,
        core_version,
        preferred,
        configured,
        cached_winner,
        policy,
        || start.elapsed(),
        system_unix_time_ms,
        fetch,
        classify,
    )
}

/// Deterministic-clock variant for synthetic tests and callers with an injected clock.
///
/// `elapsed` must be monotonic; every sample is checked for regression. The Unix clock
/// is sampled once for TTL lookup; its monotonic anchor is captured immediately
/// afterward. Accepted time includes only duration after that anchor, while the
/// total budget also includes time spent obtaining the Unix timestamp.
pub fn negotiate_with_clock<T, F, C, E, N>(
    endpoint: &ConfiguredEndpoint,
    core_version: &str,
    preferred: &[UserAgent],
    configured: &[UserAgent],
    cached_winner: Option<&CachedWinner>,
    policy: &NegotiationPolicy,
    elapsed: E,
    mut unix_time_ms: N,
    mut fetch: F,
    mut classify: C,
) -> Result<Negotiated<T>, NegotiationError>
where
    F: for<'a> FnMut(RequestSpec<'a>) -> Result<HttpResponse, FetchFailure>,
    C: FnMut(&[u8]) -> Result<T, BodyRejection>,
    E: FnMut() -> Duration,
    N: FnMut() -> Result<i64, NegotiationError>,
{
    policy.validate()?;
    validate_core_version(core_version)?;
    let candidates = ordered_candidates(preferred, configured, policy.max_candidates)?;
    let core_pin = PINNED_CORE_VERSION;
    let mut elapsed = CheckedElapsed::new(elapsed);
    let start_elapsed = elapsed.sample()?;
    let start_unix_ms = unix_time_ms()?;
    if start_unix_ms < 0 {
        return Err(NegotiationError::ClockFailure);
    }
    let epoch_anchor = elapsed.sample()?;
    if epoch_anchor - start_elapsed >= policy.total_budget {
        return Err(NegotiationError::DeadlineExceeded);
    }
    let bound_endpoint_digest = endpoint_digest(endpoint);
    let bound_agent_set_digest = agent_set_digest(&candidates);
    let cached_is_valid = cached_winner.is_some_and(|cached| {
        cache_is_valid(
            cached,
            bound_endpoint_digest,
            bound_agent_set_digest,
            &candidates,
            core_pin,
            start_unix_ms,
            policy.winner_ttl,
        )
    });
    let mut ordered = candidates;
    if let (true, Some(cached)) = (cached_is_valid, cached_winner) {
        if let Some(position) = ordered.iter().position(|agent| agent == &cached.winner) {
            if position != 0 {
                let winner = ordered.remove(position);
                ordered.insert(0, winner);
            }
        }
    }

    let max_attempts = policy.max_requests.min(ordered.len());
    let mut last_was_body_rejection = false;
    for (index, user_agent) in ordered.iter().take(max_attempts).enumerate() {
        // Use one monotonic sample for both deadline validation and remaining-time
        // calculation, so a deadline crossing cannot slip between those operations.
        let request_start = elapsed.sample()?;
        let elapsed_before_request = request_start
            .checked_sub(start_elapsed)
            .ok_or(NegotiationError::ClockFailure)?;
        if elapsed_before_request >= policy.total_budget {
            return Err(NegotiationError::DeadlineExceeded);
        }
        let remaining = policy.total_budget - elapsed_before_request;
        let request_timeout = remaining.min(policy.per_request_timeout);
        let response = fetch(RequestSpec {
            endpoint,
            user_agent,
            remaining_budget: remaining,
            request_timeout,
            max_body_bytes: policy.max_body_bytes,
        });
        let request_elapsed = elapsed
            .sample()?
            .checked_sub(request_start)
            .ok_or(NegotiationError::ClockFailure)?;
        if request_elapsed >= request_timeout {
            return Err(NegotiationError::Fetch(FetchFailure::Timeout));
        }
        remaining_budget(&mut elapsed, start_elapsed, policy.total_budget)?;
        let response = response.map_err(NegotiationError::Fetch)?;
        if response.status < 200 || response.status >= 300 {
            return Err(http_status_error(response.status));
        }
        if response.body.len() > policy.max_body_bytes {
            return Err(NegotiationError::BodyTooLarge);
        }

        remaining_budget(&mut elapsed, start_elapsed, policy.total_budget)?;
        let classified = classify(&response.body);
        remaining_budget(&mut elapsed, start_elapsed, policy.total_budget)?;
        match classified {
            Ok(parsed) => {
                let body = response.body;
                let body_hash = sha256_hex(&body);
                let end_sample = elapsed.sample()?;
                let end_elapsed = end_sample
                    .checked_sub(start_elapsed)
                    .ok_or(NegotiationError::ClockFailure)?;
                if end_elapsed >= policy.total_budget {
                    return Err(NegotiationError::DeadlineExceeded);
                }
                let epoch_elapsed = end_sample
                    .checked_sub(epoch_anchor)
                    .ok_or(NegotiationError::ClockFailure)?;
                let accepted_at_unix_ms = start_unix_ms
                    .checked_add(
                        i64::try_from(epoch_elapsed.as_millis())
                            .map_err(|_| NegotiationError::TimestampOverflow)?,
                    )
                    .ok_or(NegotiationError::TimestampOverflow)?;
                return Ok(Negotiated {
                    body,
                    parsed,
                    actual_user_agent: user_agent.clone(),
                    source_body_sha256: body_hash,
                    accepted_at_unix_ms,
                    request_count: index + 1,
                    used_cached_winner: cached_is_valid && index == 0,
                    endpoint_digest: bound_endpoint_digest,
                    agent_set_digest: bound_agent_set_digest,
                    core_version: core_pin,
                    winner_ttl: policy.winner_ttl,
                });
            }
            Err(BodyRejection::RetryableUnusableBody) => {
                last_was_body_rejection = true;
            }
            Err(BodyRejection::TerminalBody) => return Err(NegotiationError::TerminalBody),
        }
    }
    if max_attempts < ordered.len() {
        return Err(NegotiationError::RequestLimitReached);
    }
    if last_was_body_rejection {
        Err(NegotiationError::NoUsableBody)
    } else {
        Err(NegotiationError::InvalidCandidates)
    }
}

fn validate_core_version(value: &str) -> Result<(), NegotiationError> {
    let normalized = value.strip_prefix('v').unwrap_or(value);
    if normalized == PINNED_CORE_VERSION {
        Ok(())
    } else {
        Err(NegotiationError::UnsupportedCoreVersion)
    }
}

fn ordered_candidates(
    preferred: &[UserAgent],
    configured: &[UserAgent],
    max_candidates: usize,
) -> Result<Vec<UserAgent>, NegotiationError> {
    let mut ordered = Vec::new();
    for candidate in preferred.iter().chain(configured) {
        if !ordered.iter().any(|existing| existing == candidate) {
            if ordered.len() == max_candidates {
                return Err(NegotiationError::TooManyCandidates);
            }
            ordered.push(candidate.clone());
        }
    }
    if ordered.is_empty() {
        return Err(NegotiationError::InvalidCandidates);
    }
    Ok(ordered)
}

fn endpoint_digest(endpoint: &ConfiguredEndpoint) -> [u8; 32] {
    Sha256::digest(endpoint.url.as_bytes()).into()
}

fn agent_set_digest(agents: &[UserAgent]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"cm-ua-set-v1\0");
    for agent in agents {
        let bytes = agent.0.as_bytes();
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    hash.finalize().into()
}

fn cache_is_valid(
    cached: &CachedWinner,
    endpoint_digest: [u8; 32],
    ordered_agent_set_digest: [u8; 32],
    candidates: &[UserAgent],
    core_pin: &str,
    now_unix_ms: i64,
    configured_ttl: Duration,
) -> bool {
    if cached.core_version != core_pin
        || cached.endpoint_digest != endpoint_digest
        || cached.agent_set_digest != ordered_agent_set_digest
        || !candidates
            .iter()
            .any(|candidate| candidate == &cached.winner)
    {
        return false;
    }
    let Some(age_ms) = now_unix_ms.checked_sub(cached.created_at_unix_ms) else {
        return false;
    };
    if age_ms < 0 {
        return false;
    }
    let age = Duration::from_millis(age_ms as u64);
    age < cached.ttl.min(configured_ttl)
}

struct CheckedElapsed<E> {
    source: E,
    last: Option<Duration>,
}

impl<E: FnMut() -> Duration> CheckedElapsed<E> {
    fn new(source: E) -> Self {
        Self { source, last: None }
    }

    fn sample(&mut self) -> Result<Duration, NegotiationError> {
        let current = (self.source)();
        if self.last.is_some_and(|last| current < last) {
            return Err(NegotiationError::ClockFailure);
        }
        self.last = Some(current);
        Ok(current)
    }
}

fn remaining_budget<E: FnMut() -> Duration>(
    elapsed: &mut CheckedElapsed<E>,
    start_elapsed: Duration,
    total_budget: Duration,
) -> Result<Duration, NegotiationError> {
    let elapsed = elapsed
        .sample()?
        .checked_sub(start_elapsed)
        .ok_or(NegotiationError::ClockFailure)?;
    if elapsed >= total_budget {
        return Err(NegotiationError::DeadlineExceeded);
    }
    Ok(total_budget - elapsed)
}

fn http_status_error(status: u16) -> NegotiationError {
    match status {
        401 | 403 => NegotiationError::AuthenticationRejected,
        429 => NegotiationError::QuotaRejected,
        404 | 410 => NegotiationError::EndpointUnavailable,
        300..=399 => NegotiationError::RedirectRejected,
        500..=599 => NegotiationError::ServerFailure,
        _ => NegotiationError::HttpRejected,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn system_unix_time_ms() -> Result<i64, NegotiationError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| NegotiationError::ClockFailure)
        .and_then(|duration| {
            i64::try_from(duration.as_millis()).map_err(|_| NegotiationError::TimestampOverflow)
        })
}
