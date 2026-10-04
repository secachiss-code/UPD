use std::cell::{Cell, RefCell};
use std::time::Duration;

use cm::sources::{
    BodyRejection, CachedWinner, ConfiguredEndpoint, FetchFailure, HttpResponse, NegotiationError,
    NegotiationPolicy, RequestSpec, UserAgent, negotiate_with_clock,
};

fn endpoint(value: &str) -> ConfiguredEndpoint {
    ConfiguredEndpoint::new(value, false).expect("synthetic HTTPS endpoint")
}

fn ua(value: &str) -> UserAgent {
    UserAgent::new(value).expect("valid synthetic User-Agent")
}

fn epoch(value: i64) -> impl FnMut() -> Result<i64, NegotiationError> {
    move || Ok(value)
}

fn fetch_ok(
    body: &'static [u8],
) -> impl for<'a> FnMut(RequestSpec<'a>) -> Result<HttpResponse, FetchFailure> {
    move |_| Ok(HttpResponse::new(200, body.to_vec()))
}

#[test]
fn unusable_body_retries_and_records_actual_successful_user_agent() {
    let endpoint = endpoint("https://feed.invalid/private?token=synthetic");
    let preferred = [ua("Preferred/1")];
    let configured = [ua("Configured/2"), ua("Preferred/1")];
    let calls = Cell::new(0usize);
    let elapsed = Cell::new(Duration::ZERO);
    let result = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &preferred,
        &configured,
        None,
        &NegotiationPolicy::default(),
        || elapsed.get(),
        epoch(1_000),
        |request| {
            calls.set(calls.get() + 1);
            assert_eq!(request.max_redirects(), 0);
            assert_eq!(request.endpoint().expose_url(), endpoint.expose_url());
            assert!(request.request_timeout() <= request.remaining_budget());
            if calls.get() == 1 {
                Ok(HttpResponse::new(200, b"<html>login</html>".to_vec()))
            } else {
                Ok(HttpResponse::new(200, b"synthetic-valid-body".to_vec()))
            }
        },
        |body| {
            if body.starts_with(b"<html>") {
                Err(BodyRejection::RetryableUnusableBody)
            } else {
                Ok("typed synthetic source".to_owned())
            }
        },
    )
    .expect("second candidate is usable");

    assert_eq!(calls.get(), 2);
    assert_eq!(result.request_count(), 2);
    assert_eq!(result.actual_user_agent().expose_value(), "Configured/2");
    assert_eq!(result.parsed(), "typed synthetic source");
    assert_eq!(result.body(), b"synthetic-valid-body");
    assert_eq!(result.source_body_sha256().len(), 64);
    assert_eq!(result.accepted_at_unix_ms(), 1_000);
    assert!(!result.used_cached_winner());
}

fn make_cached_second_candidate() -> (
    ConfiguredEndpoint,
    [UserAgent; 1],
    [UserAgent; 1],
    CachedWinner,
) {
    let endpoint = endpoint("https://cache.invalid/subscription?secret=fixture");
    let preferred = [ua("UA-first")];
    let configured = [ua("UA-second")];
    let calls = Cell::new(0usize);
    let mut policy = NegotiationPolicy::default();
    policy.winner_ttl = Duration::from_secs(10);
    let accepted = negotiate_with_clock(
        &endpoint,
        "v1.19.32",
        &preferred,
        &configured,
        None,
        &policy,
        || Duration::ZERO,
        epoch(10_000),
        |_| {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                Ok(HttpResponse::new(200, b"not a feed".to_vec()))
            } else {
                Ok(HttpResponse::new(200, b"feed".to_vec()))
            }
        },
        |body| {
            if body == b"feed" {
                Ok(())
            } else {
                Err(BodyRejection::RetryableUnusableBody)
            }
        },
    )
    .expect("second candidate succeeds");
    let cached = accepted.make_cache_record();
    (endpoint, preferred, configured, cached)
}

#[test]
fn cached_winner_is_first_only_when_all_cache_bindings_match() {
    let (base_endpoint, preferred, configured, cached) = make_cached_second_candidate();
    let endpoint_mismatch = endpoint("https://other.invalid/subscription?secret=fixture");
    let expired_endpoint = endpoint("https://cache.invalid/subscription?secret=fixture");
    let changed_agents = [ua("UA-third")];
    let cases = [
        (&endpoint_mismatch, configured.clone(), 10_001),
        (&base_endpoint, changed_agents, 10_001),
        (&expired_endpoint, configured.clone(), 20_000),
        (&base_endpoint, configured.clone(), 9_999),
    ];

    for (test_endpoint, test_configured, now) in cases {
        let actual = RefCell::new(String::new());
        let result = negotiate_with_clock(
            test_endpoint,
            "1.19.32",
            &preferred,
            &test_configured,
            Some(&cached),
            &NegotiationPolicy::default(),
            || Duration::ZERO,
            epoch(now),
            |request| {
                *actual.borrow_mut() = request.user_agent().expose_value().to_owned();
                Ok(HttpResponse::new(200, b"any usable body".to_vec()))
            },
            |_| Ok(()),
        )
        .expect("an invalid cache is ignored, not fatal");
        assert_eq!(actual.borrow().as_str(), "UA-first");
        assert!(!result.used_cached_winner());
    }

    let actual = RefCell::new(String::new());
    let reused = negotiate_with_clock(
        &base_endpoint,
        "1.19.32",
        &preferred,
        &configured,
        Some(&cached),
        &NegotiationPolicy::default(),
        || Duration::ZERO,
        epoch(10_500),
        |request| {
            *actual.borrow_mut() = request.user_agent().expose_value().to_owned();
            Ok(HttpResponse::new(200, b"usable".to_vec()))
        },
        |_| Ok(()),
    )
    .expect("matching unexpired cache is reusable");
    assert_eq!(actual.borrow().as_str(), "UA-second");
    assert!(reused.used_cached_winner());
    assert_eq!(reused.request_count(), 1);
}

#[test]
fn terminal_http_and_transport_failures_do_not_retry() {
    let endpoint = endpoint("https://status.invalid/feed");
    let preferred = [ua("UA-one")];
    let configured = [ua("UA-two")];
    let cases = [
        (401, NegotiationError::AuthenticationRejected),
        (403, NegotiationError::AuthenticationRejected),
        (429, NegotiationError::QuotaRejected),
        (404, NegotiationError::EndpointUnavailable),
        (410, NegotiationError::EndpointUnavailable),
        (302, NegotiationError::RedirectRejected),
        (503, NegotiationError::ServerFailure),
    ];
    for (status, expected) in cases {
        let calls = Cell::new(0usize);
        let result = negotiate_with_clock(
            &endpoint,
            "1.19.32",
            &preferred,
            &configured,
            None,
            &NegotiationPolicy::default(),
            || Duration::ZERO,
            epoch(5_000),
            |_| {
                calls.set(calls.get() + 1);
                Ok(HttpResponse::new(status, b"secret response".to_vec()))
            },
            |_| Ok(()),
        );
        assert_eq!(result.err(), Some(expected));
        assert_eq!(calls.get(), 1);
    }

    let calls = Cell::new(0usize);
    let transport = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &preferred,
        &configured,
        None,
        &NegotiationPolicy::default(),
        || Duration::ZERO,
        epoch(5_000),
        |_| -> Result<HttpResponse, FetchFailure> {
            calls.set(calls.get() + 1);
            Err(FetchFailure::Transport)
        },
        |_| Ok(()),
    );
    assert_eq!(
        transport.err(),
        Some(NegotiationError::Fetch(FetchFailure::Transport))
    );
    assert_eq!(calls.get(), 1);

    let calls = Cell::new(0usize);
    let terminal_body = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &preferred,
        &configured,
        None,
        &NegotiationPolicy::default(),
        || Duration::ZERO,
        epoch(5_000),
        |_| {
            calls.set(calls.get() + 1);
            Ok(HttpResponse::new(200, b"terminal".to_vec()))
        },
        |_| -> Result<(), BodyRejection> { Err(BodyRejection::TerminalBody) },
    );
    assert_eq!(terminal_body.err(), Some(NegotiationError::TerminalBody));
    assert_eq!(calls.get(), 1);
}

#[test]
fn candidate_request_and_time_budgets_are_bounded() {
    let endpoint = endpoint("https://bounds.invalid/feed");
    let preferred = [ua("UA-one")];
    let configured = [ua("UA-two")];
    let mut policy = NegotiationPolicy::default();
    policy.max_requests = 1;
    let calls = Cell::new(0usize);
    let limited = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &preferred,
        &configured,
        None,
        &policy,
        || Duration::ZERO,
        epoch(8_000),
        |request| {
            calls.set(calls.get() + 1);
            assert_eq!(request.request_timeout(), Duration::from_secs(10));
            Ok(HttpResponse::new(200, b"unusable".to_vec()))
        },
        |_| -> Result<(), BodyRejection> { Err(BodyRejection::RetryableUnusableBody) },
    );
    assert_eq!(limited.err(), Some(NegotiationError::RequestLimitReached));
    assert_eq!(calls.get(), 1);

    let mut policy = NegotiationPolicy::default();
    policy.total_budget = Duration::from_secs(10);
    policy.per_request_timeout = Duration::from_secs(2);
    let elapsed = Cell::new(Duration::ZERO);
    let request_timeout = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &preferred,
        &[],
        None,
        &policy,
        || elapsed.get(),
        epoch(8_000),
        |request| {
            assert_eq!(request.request_timeout(), Duration::from_secs(2));
            elapsed.set(Duration::from_secs(2));
            Ok(HttpResponse::new(200, b"valid".to_vec()))
        },
        |_| -> Result<(), BodyRejection> { panic!("timed out fetch must not be classified") },
    );
    assert_eq!(
        request_timeout.err(),
        Some(NegotiationError::Fetch(FetchFailure::Timeout))
    );

    let mut policy = NegotiationPolicy::default();
    policy.total_budget = Duration::from_secs(5);
    policy.per_request_timeout = Duration::from_secs(5);
    let elapsed = Cell::new(Duration::ZERO);
    let total_timeout = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &preferred,
        &[],
        None,
        &policy,
        || elapsed.get(),
        epoch(8_000),
        |_| {
            elapsed.set(Duration::from_secs(1));
            Ok(HttpResponse::new(200, b"valid".to_vec()))
        },
        |_| {
            elapsed.set(Duration::from_secs(5));
            Ok(())
        },
    );
    assert_eq!(
        total_timeout.err(),
        Some(NegotiationError::DeadlineExceeded)
    );
}

#[test]
fn acceptance_timestamp_includes_fetch_and_classification_elapsed_time() {
    let endpoint = endpoint("https://clock.invalid/feed");
    let candidates = [ua("UA-clock")];
    let elapsed = Cell::new(Duration::ZERO);
    let accepted = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &candidates,
        &[],
        None,
        &NegotiationPolicy::default(),
        || elapsed.get(),
        epoch(50_000),
        |_| {
            elapsed.set(Duration::from_millis(750));
            Ok(HttpResponse::new(200, b"valid".to_vec()))
        },
        |_| {
            elapsed.set(Duration::from_millis(1_250));
            Ok(())
        },
    )
    .expect("bounded synthetic result");
    assert_eq!(accepted.accepted_at_unix_ms(), 51_250);
}

#[test]
fn unix_clock_sampling_time_uses_budget_without_being_added_twice_to_timestamp() {
    let endpoint = endpoint("https://clock.invalid/feed");
    let candidates = [ua("UA-clock")];
    let elapsed = Cell::new(Duration::ZERO);
    let accepted = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &candidates,
        &[],
        None,
        &NegotiationPolicy::default(),
        || elapsed.get(),
        || {
            elapsed.set(Duration::from_secs(3));
            Ok(50_000)
        },
        |request| {
            assert_eq!(request.remaining_budget(), Duration::from_secs(27));
            elapsed.set(Duration::from_millis(3_750));
            Ok(HttpResponse::new(200, b"valid".to_vec()))
        },
        |_| {
            elapsed.set(Duration::from_millis(4_250));
            Ok(())
        },
    )
    .expect("sampling time counted once");
    assert_eq!(accepted.accepted_at_unix_ms(), 51_250);
}

#[test]
fn monotonic_clock_regression_is_a_safe_terminal_error() {
    let endpoint = endpoint("https://clock.invalid/feed");
    let candidates = [ua("UA-clock")];
    let mut samples = [
        Duration::ZERO,
        Duration::from_secs(1),
        Duration::from_millis(999),
    ]
    .into_iter();
    let result = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &candidates,
        &[],
        None,
        &NegotiationPolicy::default(),
        || samples.next().unwrap_or(Duration::MAX),
        epoch(60_000),
        fetch_ok(b"valid"),
        |_| Ok(()),
    );
    assert_eq!(result.err(), Some(NegotiationError::ClockFailure));
}

#[test]
fn request_that_starts_at_deadline_is_never_fetched() {
    let endpoint = endpoint("https://deadline.invalid/feed");
    let candidates = [ua("UA-deadline")];
    let mut samples = [Duration::ZERO, Duration::from_secs(30)].into_iter();
    let fetch_calls = Cell::new(0usize);
    let result = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &candidates,
        &[],
        None,
        &NegotiationPolicy::default(),
        || samples.next().unwrap_or(Duration::MAX),
        epoch(70_000),
        |_| {
            fetch_calls.set(fetch_calls.get() + 1);
            Ok(HttpResponse::new(200, b"valid".to_vec()))
        },
        |_| Ok(()),
    );
    assert_eq!(result.err(), Some(NegotiationError::DeadlineExceeded));
    assert_eq!(fetch_calls.get(), 0);
}

#[test]
fn oversize_and_invalid_policy_fail_before_classifier_or_fetch() {
    let endpoint = endpoint("https://limits.invalid/feed");
    let candidates = [ua("UA-only")];
    let other_candidate = [ua("UA-second")];
    let mut policy = NegotiationPolicy::default();
    policy.max_body_bytes = 8;
    let classifier_calls = Cell::new(0usize);
    let oversize_fetch_calls = Cell::new(0usize);
    let oversize = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &candidates,
        &other_candidate,
        None,
        &policy,
        || Duration::ZERO,
        epoch(9_000),
        |_| {
            oversize_fetch_calls.set(oversize_fetch_calls.get() + 1);
            Ok(HttpResponse::new(200, b"123456789".to_vec()))
        },
        |_| {
            classifier_calls.set(classifier_calls.get() + 1);
            Ok(())
        },
    );
    assert_eq!(oversize.err(), Some(NegotiationError::BodyTooLarge));
    assert_eq!(oversize_fetch_calls.get(), 1);
    assert_eq!(classifier_calls.get(), 0);

    let mut invalid_policy = NegotiationPolicy::default();
    invalid_policy.max_requests = 0;
    let fetch_calls = Cell::new(0usize);
    let invalid = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &candidates,
        &[],
        None,
        &invalid_policy,
        || Duration::ZERO,
        epoch(9_000),
        |_| {
            fetch_calls.set(fetch_calls.get() + 1);
            Ok(HttpResponse::new(200, b"body".to_vec()))
        },
        |_| Ok(()),
    );
    assert_eq!(invalid.err(), Some(NegotiationError::InvalidPolicy));
    assert_eq!(fetch_calls.get(), 0);

    let too_many: Vec<UserAgent> = (0..9)
        .map(|index| UserAgent::new(format!("UA-{index}")).unwrap())
        .collect();
    let candidate_fetches = Cell::new(0usize);
    let overflow = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &too_many,
        &[],
        None,
        &NegotiationPolicy::default(),
        || Duration::ZERO,
        epoch(9_000),
        |_| {
            candidate_fetches.set(candidate_fetches.get() + 1);
            Ok(HttpResponse::new(200, b"body".to_vec()))
        },
        |_| Ok(()),
    );
    assert_eq!(overflow.err(), Some(NegotiationError::TooManyCandidates));
    assert_eq!(candidate_fetches.get(), 0);

    assert_eq!(
        UserAgent::new("bad\nagent").err(),
        Some(NegotiationError::InvalidUserAgent)
    );
    assert!(UserAgent::new("x".repeat(512)).is_ok());
    assert_eq!(
        UserAgent::new("x".repeat(513)).err(),
        Some(NegotiationError::InvalidUserAgent)
    );
    assert_eq!(
        ConfiguredEndpoint::new("https://example.invalid/feed#fragment", false).err(),
        Some(NegotiationError::InvalidEndpoint)
    );
    assert_eq!(
        ConfiguredEndpoint::new("https://example.invalid/a\nb", false).err(),
        Some(NegotiationError::InvalidEndpoint)
    );
    assert_eq!(
        ConfiguredEndpoint::new("http://example.invalid/feed", false).err(),
        Some(NegotiationError::InvalidEndpoint)
    );
    assert!(ConfiguredEndpoint::new("http://example.invalid/feed", true).is_ok());
    let long_url = format!("https://example.invalid/{}", "x".repeat(8 * 1024));
    assert_eq!(
        ConfiguredEndpoint::new(long_url, false).err(),
        Some(NegotiationError::InvalidEndpoint)
    );

    let negative_time_fetches = Cell::new(0usize);
    let negative_time = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &candidates,
        &[],
        None,
        &NegotiationPolicy::default(),
        || Duration::ZERO,
        epoch(-1),
        |_| {
            negative_time_fetches.set(negative_time_fetches.get() + 1);
            Ok(HttpResponse::new(200, b"body".to_vec()))
        },
        |_| Ok(()),
    );
    assert_eq!(negative_time.err(), Some(NegotiationError::ClockFailure));
    assert_eq!(negative_time_fetches.get(), 0);

    let unsupported_fetches = Cell::new(0usize);
    let unsupported = negotiate_with_clock(
        &endpoint,
        "1.20.0",
        &candidates,
        &[],
        None,
        &NegotiationPolicy::default(),
        || Duration::ZERO,
        epoch(9_000),
        |_| {
            unsupported_fetches.set(unsupported_fetches.get() + 1);
            Ok(HttpResponse::new(200, b"body".to_vec()))
        },
        |_| Ok(()),
    );
    assert_eq!(
        unsupported.err(),
        Some(NegotiationError::UnsupportedCoreVersion)
    );
    assert_eq!(unsupported_fetches.get(), 0);
}

#[test]
fn private_values_stay_out_of_debug_output() {
    let endpoint = endpoint("https://secret.invalid/private?token=endpoint-secret");
    let preferred = [ua("Agent-secret/1")];
    let body = b"body-secret";
    let result = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &preferred,
        &[],
        None,
        &NegotiationPolicy::default(),
        || Duration::ZERO,
        epoch(10_000),
        fetch_ok(body),
        |_| Ok("parsed-secret".to_owned()),
    )
    .expect("synthetic response accepted");
    let response = HttpResponse::new(200, body.to_vec());
    let cache = result.make_cache_record();
    let formatted = format!(
        "{endpoint:?} {:?} {response:?} {result:?} {cache:?}",
        preferred[0]
    );
    for secret in [
        "endpoint-secret",
        "Agent-secret",
        "body-secret",
        "parsed-secret",
    ] {
        assert!(!formatted.contains(secret));
    }
    assert!(formatted.contains("[REDACTED]"));
}
