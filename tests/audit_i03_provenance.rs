#[path = "support/profiles.rs"]
#[allow(dead_code)] // Shared fixture also provides helpers for other audit targets.
mod support;

use cm::profiles::*;
use cm::sources::{PINNED_CORE_COMMIT, PINNED_CORE_VERSION};
use support::{digest, fixture, id};

fn graph_with_provenance() -> GraphSnapshot {
    let mut graph = fixture();
    let source = graph.sources.get_mut(&id("source-a")).unwrap();
    source.provenance = Some(SourceProvenance {
        schema_version: SCHEMA_VERSION,
        format: SourceFormat::MihomoYaml,
        accepted_at_unix_ms: 1_000,
        raw_body_digest_sha256: source.content_digest_sha256.clone(),
        core_version: PINNED_CORE_VERSION.into(),
        core_commit: PINNED_CORE_COMMIT.into(),
        origin: SourceOrigin::Negotiated,
        actual_user_agent_sha256: Some(digest(b"synthetic-private-accepted-agent")),
        omissions: ImportOmissions::default(),
        confirmed_omissions_digest_sha256: None,
        accepted_omissions_bound: ImportOmissions::default(),
    });
    graph.validate().unwrap();
    graph
}

#[test]
fn provenance_has_strict_schema_and_consistent_origin_time_and_body_digest() {
    let graph = graph_with_provenance();
    let encoded = serde_json::to_value(&graph).unwrap();
    for (field, value) in [
        ("accepted_at_unix_ms", serde_json::json!(-1)),
        (
            "raw_body_digest_sha256",
            serde_json::json!(digest(b"other body")),
        ),
        ("actual_user_agent_sha256", serde_json::Value::Null),
        ("origin", serde_json::json!("local")),
    ] {
        let mut modified = encoded.clone();
        modified["sources"]["source-a"]["provenance"][field] = value;
        let candidate: GraphSnapshot = serde_json::from_value(modified).unwrap();
        assert_eq!(candidate.validate(), Err(ModelError::InvalidProvenance));
    }
    for (field, value) in [
        (
            "actual_user_agent",
            serde_json::json!("synthetic-private-accepted-agent"),
        ),
        ("schema_version", serde_json::json!(999)),
        ("format", serde_json::json!("unknown-format")),
    ] {
        let mut modified = encoded.clone();
        modified["sources"]["source-a"]["provenance"][field] = value;
        assert!(serde_json::from_value::<GraphSnapshot>(modified).is_err());
    }
    let public =
        serde_json::to_value(graph.public_source_status(&id("source-a")).unwrap()).unwrap();
    assert!(public.get("provenance").is_none());
    assert!(public.get("actual_user_agent_sha256").is_none());
}

#[test]
fn refreshing_metadata_preserves_generation_and_rejects_provenance_loss_or_reinterpretation() {
    let old = graph_with_provenance();
    let mut refreshed = old.clone();
    refreshed.revision += 1;
    refreshed
        .sources
        .get_mut(&id("source-a"))
        .unwrap()
        .provenance
        .as_mut()
        .unwrap()
        .accepted_at_unix_ms = 2_000;
    GraphSnapshot::validate_transition(&old, &refreshed).unwrap();
    assert_eq!(refreshed.nodes, old.nodes);
    assert_eq!(refreshed.sessions, old.sessions);
    assert_eq!(
        refreshed.sources[&id("source-a")].generation,
        old.sources[&id("source-a")].generation
    );

    let mut missing = refreshed.clone();
    missing.sources.get_mut(&id("source-a")).unwrap().provenance = None;
    assert_eq!(
        GraphSnapshot::validate_transition(&old, &missing),
        Err(ModelError::InvalidProvenance)
    );
    let mut backwards = refreshed.clone();
    backwards
        .sources
        .get_mut(&id("source-a"))
        .unwrap()
        .provenance
        .as_mut()
        .unwrap()
        .accepted_at_unix_ms = 999;
    assert_eq!(
        GraphSnapshot::validate_transition(&old, &backwards),
        Err(ModelError::InvalidProvenance)
    );

    for change in 0..3 {
        let mut reinterpreted = refreshed.clone();
        let provenance = reinterpreted
            .sources
            .get_mut(&id("source-a"))
            .unwrap()
            .provenance
            .as_mut()
            .unwrap();
        match change {
            0 => provenance.format = SourceFormat::MihomoJson,
            1 => provenance.core_version = "9.8.7".into(),
            _ => provenance.core_commit = "a".repeat(40),
        }
        assert_eq!(
            GraphSnapshot::validate_transition(&old, &reinterpreted),
            Err(ModelError::InvalidProvenance)
        );
    }
}

#[test]
fn graph_retains_historical_core_pins_while_refusing_malformed_pin_metadata() {
    let mut graph = graph_with_provenance();
    let provenance = graph
        .sources
        .get_mut(&id("source-a"))
        .unwrap()
        .provenance
        .as_mut()
        .unwrap();
    provenance.core_version = "0.9.8-historical".into();
    provenance.core_commit = "b".repeat(40);
    graph.validate().unwrap();

    let mut malformed = graph.clone();
    malformed
        .sources
        .get_mut(&id("source-a"))
        .unwrap()
        .provenance
        .as_mut()
        .unwrap()
        .core_version = "0.9.8\nprivate".into();
    assert_eq!(malformed.validate(), Err(ModelError::InvalidProvenance));
    let mut malformed = graph;
    malformed
        .sources
        .get_mut(&id("source-a"))
        .unwrap()
        .provenance
        .as_mut()
        .unwrap()
        .core_commit = "short".into();
    assert_eq!(malformed.validate(), Err(ModelError::InvalidProvenance));
}
