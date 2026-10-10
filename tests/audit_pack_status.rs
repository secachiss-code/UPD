//! M15: a tunnel view is built from the checks of the current generation and never folds
//! the axes into one green status.

use std::collections::BTreeMap;

use cm::profiles::{
    HostMode, HostPolicy, Id, SCHEMA_VERSION, Session, SessionLifecycle, TunnelInstance,
    TunnelLifecycle, TunnelOwner, Verification, VerificationAxis, VerificationValue,
};
use cm::status::tunnels::{TunnelView, build};

const NOW: i64 = 10_000_000;

fn id(value: &str) -> Id {
    Id::new(value).unwrap()
}

fn host(mode: HostMode) -> HostPolicy {
    HostPolicy {
        schema_version: SCHEMA_VERSION,
        id: id("host"),
        mode,
        connection_profile_id: None,
        tunnel_instance_id: None,
        autostart: false,
    }
}

fn tunnel(owner: TunnelOwner) -> TunnelInstance {
    TunnelInstance {
        schema_version: SCHEMA_VERSION,
        id: id("t1"),
        connection_profile_id: id("profile-1"),
        generation: 4,
        owner,
        lifecycle: TunnelLifecycle::Running,
    }
}

fn own() -> TunnelOwner {
    TunnelOwner::Application {
        application_id: id("browser"),
    }
}

fn session(name: &str, tunnel: &str, lifecycle: SessionLifecycle) -> Session {
    Session {
        schema_version: SCHEMA_VERSION,
        id: id(name),
        lifecycle,
        application_id: id("browser"),
        data_profile_id: id("data-1"),
        environment_profile_id: id("env-1"),
        tunnel_instance_id: id(tunnel),
        tunnel_generation: 4,
        source_id: id("source-1"),
        source_generation: 1,
        node_id: id("node-1"),
        created_at_unix_ms: 1,
        ended_at_unix_ms: None,
    }
}

fn check(
    axis: VerificationAxis,
    value: VerificationValue,
    generation: u64,
    age_s: i64,
) -> Verification {
    Verification {
        schema_version: SCHEMA_VERSION,
        id: id("v1"),
        session_id: id("s1"),
        tunnel_instance_id: id("t1"),
        tunnel_generation: generation,
        axis,
        value,
        evidence_at_unix_ms: NOW - age_s * 1000,
    }
}

fn view(checks: &[Verification]) -> TunnelView {
    build(
        &host(HostMode::Proxy),
        &tunnel(own()),
        &[],
        checks,
        None,
        NOW,
    )
}

fn map<T: Clone>(items: &[(&str, T)]) -> BTreeMap<String, T> {
    items
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

use VerificationAxis::{App, Net, Region, State};
use VerificationValue::{Blocked, Error, Partial, Verified};

#[test]
fn m15_view_of_the_current_generation() {
    let sessions = [
        session("s1", "t1", SessionLifecycle::Active),
        session("s2", "t1", SessionLifecycle::Ended),
        session("s3", "t2", SessionLifecycle::Active),
    ];
    let checks = [check(Net, Verified, 4, 40), check(Region, Partial, 4, 300)];
    let got = build(
        &host(HostMode::Proxy),
        &tunnel(own()),
        &sessions,
        &checks,
        None,
        NOW,
    );
    assert_eq!(
        got,
        TunnelView {
            name: "t1".to_owned(),
            host: "proxy".to_owned(),
            apps: "separate".to_owned(),
            condition: "degraded".to_owned(),
            axes: map(&[
                ("net", "verified"),
                ("region", "partial"),
                ("state", "unknown"),
                ("app", "unknown")
            ])
            .into_iter()
            .map(|(key, value)| (key, value.to_owned()))
            .collect(),
            age_s: map(&[
                ("net", Some(40)),
                ("region", Some(300)),
                ("state", None),
                ("app", None)
            ]),
            failure: None,
            sessions: 1,
        }
    );
    let shared = build(
        &host(HostMode::Off),
        &tunnel(TunnelOwner::Group { group_id: id("g") }),
        &sessions,
        &[],
        None,
        NOW,
    );
    assert_eq!(
        (shared.host.as_str(), shared.apps.as_str()),
        ("off", "shared")
    );
    assert_eq!(
        build(
            &host(HostMode::Tunnel),
            &tunnel(TunnelOwner::Host),
            &[],
            &[],
            None,
            NOW
        )
        .host,
        "tunnel"
    );
}

#[test]
fn m15_stale_generation_latest_evidence_and_future() {
    // A check of generation 3 says nothing about generation 4.
    assert_eq!(view(&[check(Net, Verified, 3, 10)]).axes["net"], "unknown");
    let mut other_tunnel = check(Net, Verified, 4, 10);
    other_tunnel.tunnel_instance_id = id("t2");
    assert_eq!(view(&[other_tunnel]).axes["net"], "unknown");
    let latest = view(&[check(Net, Verified, 4, 100), check(Net, Error, 4, 10)]);
    assert_eq!(latest.axes["net"], "error");
    assert_eq!(latest.age_s["net"], Some(10));
    assert_eq!(view(&[check(Net, Verified, 4, -50)]).age_s["net"], Some(0));
}

#[test]
fn m15_condition_and_failure() {
    let all_verified = [
        check(Net, Verified, 4, 1),
        check(Region, Verified, 4, 1),
        check(State, Verified, 4, 1),
        check(App, Verified, 4, 1),
    ];
    // There is no "ok": four verified axes stay four axes.
    assert_eq!(view(&all_verified).condition, "unknown");
    assert_eq!(view(&[]).condition, "unknown");
    for axis in [Net, Region, State, App] {
        assert_eq!(view(&[check(axis, Blocked, 4, 1)]).condition, "blocked");
    }
    assert_eq!(view(&[check(Net, Error, 4, 1)]).condition, "degraded");
    assert_eq!(
        view(&[check(Net, Error, 4, 1), check(App, Blocked, 4, 1)]).condition,
        "blocked"
    );
    assert_eq!(
        view(&[check(Net, Error, 4, 1)]).failure.as_deref(),
        Some("api-down")
    );
    assert_eq!(
        view(&[check(Region, Blocked, 4, 1)]).failure.as_deref(),
        Some("region-mismatch")
    );
    assert_eq!(
        view(&[check(Net, Error, 4, 1), check(Region, Blocked, 4, 1)])
            .failure
            .as_deref(),
        Some("api-down")
    );
    assert_eq!(view(&all_verified).failure, None);
}

#[test]
fn m15_fixtures_and_snapshots_use_the_same_type() {
    let fixtures: Vec<TunnelView> =
        serde_json::from_str(include_str!("fixtures/i17/states.json")).unwrap();
    assert!(!fixtures.is_empty());
    let snapshots = std::fs::read_dir("tests/fixtures/i17/snapshots")
        .unwrap()
        .count();
    assert_eq!(snapshots, 24);
    assert!(include_str!("../src/tui/mock_tunnels.rs").contains("TunnelView as Scenario"));
}
