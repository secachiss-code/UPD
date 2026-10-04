//! Synthetic provenance/private-artifact regressions. Deliberately NOT_RUN here.

use cm::profiles::*;
use cm::sources::artifact::{
    ArtifactError, GlobalDefaults, NodeDefinitionInput, SourceImportInput, create_source,
    read_node_definition, read_source_artifact, update_source,
};
use cm::sources::{
    BodyRejection, ConfiguredEndpoint, HttpResponse, ImportFormat, NegotiationPolicy, Transport,
    UserAgent, negotiate_with_clock,
};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);

fn id(text: &str) -> Id {
    Id::new(text).expect("synthetic id")
}

fn issue(graph: &mut GraphSnapshot, text: &str, kind: EntityIdKind) {
    let value = id(text);
    assert!(graph.issued_ids.insert(value.clone()));
    assert!(graph.issued_id_kinds.insert(value, kind).is_none());
}

fn root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "cm-i03-artifact-{label}-{}-{}",
        std::process::id(),
        NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn defaults(level: &str) -> GlobalDefaults {
    GlobalDefaults::new(vec![("log-level".to_owned(), json!(level))])
        .expect("synthetic constrained default")
}

fn definition(name: &str, password: &str) -> NodeDefinitionInput {
    NodeDefinitionInput::from_trusted_classifier(
        "1.19.32",
        NodeProtocol::Vless,
        Transport::Tcp,
        json!({
            "name": name,
            "server": "edge.synthetic.invalid",
            "port": 443,
            "uuid": "00000000-0000-4000-8000-000000000001",
            "password": password,
        }),
    )
    .expect("synthetic typed definition")
}

fn accepted_input(
    body: &[u8],
    actual_ua: &str,
    accepted_at: i64,
    format: ImportFormat,
    node_name: &str,
    password: &str,
    log_level: &str,
) -> SourceImportInput {
    let endpoint = ConfiguredEndpoint::new(
        "https://feed.synthetic.invalid/list?token=endpoint-secret",
        false,
    )
    .expect("synthetic endpoint");
    let preferred = [UserAgent::new("UA-first-candidate").unwrap()];
    let configured = [UserAgent::new(actual_ua).unwrap()];
    let accepted = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &preferred,
        &configured,
        None,
        &NegotiationPolicy::default(),
        || Duration::ZERO,
        || Ok(accepted_at),
        |request| {
            if request.user_agent().expose_value() == "UA-first-candidate" {
                Ok(HttpResponse::new(
                    200,
                    b"<html>login-secret</html>".to_vec(),
                ))
            } else {
                Ok(HttpResponse::new(200, body.to_vec()))
            }
        },
        |bytes| {
            if bytes.starts_with(b"<html>") {
                Err(BodyRejection::RetryableUnusableBody)
            } else {
                Ok(())
            }
        },
    )
    .expect("second candidate returns accepted synthetic body");
    assert_eq!(accepted.actual_user_agent().expose_value(), actual_ua);
    SourceImportInput::from_negotiated(
        format,
        &accepted,
        vec![definition(node_name, password)],
        defaults(log_level),
    )
    .expect("bounded typed import input")
}

fn add_active_session(store: &Store, source_id: &Id, node_id: &Id) -> GraphSnapshot {
    let mut graph = store
        .read_snapshot()
        .expect("snapshot before session setup");
    let previous = graph.clone();
    let profile_id = id("i03-profile");
    let tunnel_id = id("i03-tunnel");
    let app_id = id("i03-app");
    let data_id = id("i03-data");
    let preset_id = id("i03-preset");
    let session_id = id("i03-session");
    let environment_id = id("i03-environment");

    graph.connection_profiles.insert(
        profile_id.clone(),
        ConnectionProfile {
            schema_version: SCHEMA_VERSION,
            id: profile_id.clone(),
            source_id: source_id.clone(),
            node_selection: NodeSelection::Pinned {
                node: SourceNodeRef {
                    source_id: source_id.clone(),
                    source_generation: 1,
                    node_id: node_id.clone(),
                },
            },
            kernel: KernelKind::Mihomo,
            dns: DnsPolicy {
                ipv6: Ipv6Policy::Block,
                fake_ip: FakeIpPolicy::Forbidden,
            },
        },
    );
    graph.tunnel_instances.insert(
        tunnel_id.clone(),
        TunnelInstance {
            schema_version: SCHEMA_VERSION,
            id: tunnel_id.clone(),
            connection_profile_id: profile_id,
            generation: 1,
            owner: TunnelOwner::Application {
                application_id: app_id.clone(),
            },
            lifecycle: TunnelLifecycle::Running,
        },
    );
    graph.data_profiles.insert(
        data_id.clone(),
        DataProfile {
            schema_version: SCHEMA_VERSION,
            id: data_id.clone(),
        },
    );
    graph.environment_presets.insert(
        preset_id.clone(),
        EnvironmentPreset {
            schema_version: SCHEMA_VERSION,
            id: preset_id.clone(),
            timezone: "UTC".into(),
            locale: "en_US.UTF-8".into(),
            languages: vec!["en".into()],
        },
    );
    graph.applications.insert(
        app_id.clone(),
        ApplicationDefinition {
            schema_version: SCHEMA_VERSION,
            id: app_id.clone(),
            executable: "/usr/bin/synthetic-app".into(),
            argv: Vec::new(),
            cwd: None,
            data_profile_id: data_id.clone(),
            environment: Vec::new(),
            environment_preset_id: preset_id.clone(),
            assignment: ApplicationAssignment::OwnTunnel {
                tunnel_instance_id: tunnel_id.clone(),
            },
            autostart: false,
        },
    );
    graph.sessions.insert(
        session_id.clone(),
        Session {
            schema_version: SCHEMA_VERSION,
            id: session_id.clone(),
            lifecycle: SessionLifecycle::Active,
            application_id: app_id.clone(),
            data_profile_id: data_id,
            environment_profile_id: environment_id.clone(),
            tunnel_instance_id: tunnel_id,
            tunnel_generation: 1,
            source_id: source_id.clone(),
            source_generation: 1,
            node_id: node_id.clone(),
            created_at_unix_ms: 10_000,
            ended_at_unix_ms: None,
        },
    );
    graph.environment_profiles.insert(
        environment_id.clone(),
        EnvironmentProfile {
            schema_version: SCHEMA_VERSION,
            id: environment_id.clone(),
            session_id: session_id.clone(),
            preset_id,
            timezone: "UTC".into(),
            locale: "en_US.UTF-8".into(),
            languages: vec!["en".into()],
        },
    );
    for (text, kind) in [
        ("i03-profile", EntityIdKind::ConnectionProfile),
        ("i03-tunnel", EntityIdKind::TunnelInstance),
        ("i03-app", EntityIdKind::Application),
        ("i03-data", EntityIdKind::DataProfile),
        ("i03-preset", EntityIdKind::EnvironmentPreset),
        ("i03-session", EntityIdKind::Session),
        ("i03-environment", EntityIdKind::EnvironmentProfile),
    ] {
        issue(&mut graph, text, kind);
    }
    store
        .commit(previous.revision, graph, Vec::new())
        .expect("commit synthetic active session")
}

#[test]
fn persisted_winner_refreshes_and_historical_node_keeps_its_artifact() {
    let root = root("lifecycle");
    let store = Store::initialize(&root).expect("private synthetic store");
    let create = create_source(
        &store,
        0,
        accepted_input(
            b"raw-feed-body-secret-v1",
            "UA-actual-winner-secret",
            10_000,
            ImportFormat::MihomoYaml,
            "edge-one",
            "node-password-secret-v1",
            "warning",
        ),
    )
    .expect("create source");
    assert_eq!(create.source_generation, 1);
    assert_eq!(create.graph_revision, 1);
    let graph1 = store.read_snapshot().unwrap();
    let source1 = &graph1.sources[&create.source_id];
    let old_artifact_ref = source1.credential.as_ref().unwrap().credential_ref.clone();
    let old_node_id = create.node_ids[0].clone();
    let public_graph = serde_json::to_string(&graph1).unwrap();
    for material in [
        "raw-feed-body-secret-v1",
        "UA-actual-winner-secret",
        "node-password-secret-v1",
        "feed.synthetic.invalid/list?token=endpoint-secret",
    ] {
        assert!(!public_graph.contains(material));
    }

    let debug_input = format!(
        "{:?}",
        accepted_input(
            b"raw-feed-body-secret-v1",
            "UA-actual-winner-secret",
            10_001,
            ImportFormat::MihomoYaml,
            "edge-one",
            "node-password-secret-v1",
            "warning",
        )
    );
    assert!(!debug_input.contains("node-password-secret-v1"));
    drop(store);

    let store = Store::open(&root).expect("reopen persisted synthetic store");
    let artifact = read_source_artifact(&store, &create.source_id).expect("read source artifact");
    assert_eq!(artifact.raw_body(), b"raw-feed-body-secret-v1");
    assert_eq!(
        artifact.actual_user_agent(),
        Some("UA-actual-winner-secret")
    );
    assert_eq!(artifact.node_count(), 1);
    assert_eq!(
        artifact.defaults().get("log-level").and_then(Value::as_str),
        Some("warning")
    );
    assert!(!format!("{artifact:?}").contains("UA-actual-winner-secret"));

    let same_ua = update_source(
        &store,
        &create.source_id,
        1,
        1,
        accepted_input(
            b"raw-feed-body-secret-v1",
            "UA-actual-winner-secret",
            10_010,
            ImportFormat::MihomoYaml,
            "edge-one",
            "node-password-secret-v1",
            "warning",
        ),
        10_010,
    )
    .expect("same body and winner reuse private artifact");
    assert!(same_ua.artifact_reused);
    assert_eq!(same_ua.source_generation, 1);
    assert_eq!(same_ua.node_ids, vec![old_node_id.clone()]);
    assert_eq!(same_ua.graph_revision, 2);

    let changed_ua = update_source(
        &store,
        &create.source_id,
        2,
        1,
        accepted_input(
            b"raw-feed-body-secret-v1",
            "UA-refreshed-secret",
            10_020,
            ImportFormat::MihomoYaml,
            "edge-one",
            "node-password-secret-v1",
            "warning",
        ),
        10_020,
    )
    .expect("same body with a different actual UA uses a new source artifact");
    assert!(!changed_ua.artifact_reused);
    assert_eq!(changed_ua.source_generation, 1);
    assert_eq!(changed_ua.node_ids, vec![old_node_id.clone()]);
    let graph2 = store.read_snapshot().unwrap();
    assert_ne!(
        graph2.sources[&create.source_id]
            .credential
            .as_ref()
            .unwrap()
            .credential_ref,
        old_artifact_ref
    );
    assert_eq!(
        graph2.nodes[&old_node_id].credential_refs,
        vec![old_artifact_ref.clone()]
    );
    assert_eq!(
        read_source_artifact(&store, &create.source_id)
            .unwrap()
            .actual_user_agent(),
        Some("UA-refreshed-secret")
    );

    let changed_body = update_source(
        &store,
        &create.source_id,
        3,
        1,
        accepted_input(
            b"raw-feed-body-secret-v2",
            "UA-v2-secret",
            10_030,
            ImportFormat::MihomoJson,
            "edge-two",
            "node-password-secret-v2",
            "debug",
        ),
        10_030,
    )
    .expect("changed body archives old immutable node");
    assert_eq!(changed_body.source_generation, 2);
    assert_ne!(changed_body.node_ids[0], old_node_id);
    let old_definition = read_node_definition(&store, &old_node_id)
        .expect("old node resolves through its archived artifact");
    assert_eq!(old_definition.full_definition()["name"], "edge-one");
    assert_eq!(old_definition.source_id(), &create.source_id);
    assert_eq!(old_definition.source_generation(), 1);
    assert_eq!(old_definition.format(), SourceFormat::MihomoYaml);
    assert_eq!(old_definition.origin(), SourceOrigin::Negotiated);
    assert_eq!(old_definition.core_version(), "1.19.32");
    assert_eq!(
        old_definition.core_commit(),
        "88dcbf7f1614a67c3b36b848ee3592dfa92ada36"
    );
    let current_definition = read_node_definition(&store, &changed_body.node_ids[0]).unwrap();
    assert_eq!(current_definition.source_generation(), 2);
    assert_eq!(current_definition.format(), SourceFormat::MihomoJson);
    assert_eq!(
        current_definition
            .defaults()
            .get("log-level")
            .and_then(Value::as_str),
        Some("debug")
    );
    assert_eq!(
        old_definition
            .defaults()
            .get("log-level")
            .and_then(Value::as_str),
        Some("warning")
    );
    assert!(!format!("{old_definition:?}").contains("node-password-secret-v1"));

    let unchanged = store.read_snapshot().unwrap();
    let refused = update_source(
        &store,
        &create.source_id,
        unchanged.revision,
        2,
        accepted_input(
            b"raw-feed-body-secret-v2",
            "UA-v2-secret",
            10_040,
            ImportFormat::MihomoJson,
            "edge-two",
            "node-password-secret-v2",
            "warning",
        ),
        10_040,
    );
    assert!(matches!(
        refused,
        Err(ArtifactError::SameBodyPayloadChanged)
    ));
    assert_eq!(store.read_snapshot().unwrap(), unchanged);
    let changed_format = update_source(
        &store,
        &create.source_id,
        unchanged.revision,
        2,
        accepted_input(
            b"raw-feed-body-secret-v2",
            "UA-v2-secret",
            10_041,
            ImportFormat::MihomoYaml,
            "edge-two",
            "node-password-secret-v2",
            "debug",
        ),
        10_041,
    );
    assert!(matches!(
        changed_format,
        Err(ArtifactError::SameBodyPayloadChanged)
    ));
    assert_eq!(store.read_snapshot().unwrap(), unchanged);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn changed_body_invalidates_only_net_for_lost_active_pins_and_isolates_other_sources() {
    let root = root("session-net");
    let store = Store::initialize(&root).expect("private synthetic store");
    let source_a = create_source(
        &store,
        0,
        accepted_input(
            b"source-a-body-v1",
            "UA-source-a",
            10_000,
            ImportFormat::MihomoYaml,
            "source-a-node-v1",
            "source-a-secret-v1",
            "warning",
        ),
    )
    .expect("create source A");
    let source_b = create_source(
        &store,
        1,
        accepted_input(
            b"source-b-body-v1",
            "UA-source-b",
            10_000,
            ImportFormat::MihomoYaml,
            "source-b-node-v1",
            "source-b-secret-v1",
            "warning",
        ),
    )
    .expect("create independent source B");
    let before_session = add_active_session(&store, &source_a.source_id, &source_a.node_ids[0]);
    let session_before = before_session.sessions[&id("i03-session")].clone();
    let source_b_before = before_session.sources[&source_b.source_id].clone();
    let source_b_node_before = before_session.nodes[&source_b.node_ids[0]].clone();

    let updated = update_source(
        &store,
        &source_a.source_id,
        before_session.revision,
        1,
        accepted_input(
            b"source-a-body-v2",
            "UA-source-a-v2",
            20_000,
            ImportFormat::MihomoYaml,
            "source-a-node-v2",
            "source-a-secret-v2",
            "warning",
        ),
        20_000,
    )
    .expect("update source A with a disappeared pinned node");
    assert_eq!(updated.source_generation, 2);
    let after = store.read_snapshot().unwrap();
    assert_eq!(after.sessions[&id("i03-session")], session_before);
    assert_eq!(after.sources[&source_b.source_id], source_b_before);
    assert_eq!(after.nodes[&source_b.node_ids[0]], source_b_node_before);
    assert!(after.nodes.contains_key(&source_a.node_ids[0]));
    assert_eq!(after.verifications.len(), 1);
    let net = after.verifications.values().next().unwrap();
    assert_eq!(net.session_id, id("i03-session"));
    assert_eq!(net.axis, VerificationAxis::Net);
    assert_eq!(net.value, VerificationValue::Blocked);
    assert_eq!(net.evidence_at_unix_ms, 20_000);
    let status = PublicSessionStatus::from_graph(&after, &id("i03-session")).unwrap();
    assert_eq!(status.verification.net, VerificationValue::Blocked);
    assert_eq!(status.verification.region, VerificationValue::Unknown);
    assert_eq!(status.verification.state, VerificationValue::Unknown);
    assert_eq!(status.verification.app, VerificationValue::Unknown);
    let archived = read_node_definition(&store, &source_a.node_ids[0]).unwrap();
    assert_eq!(archived.full_definition()["name"], "source-a-node-v1");
    let independent = read_source_artifact(&store, &source_b.source_id).unwrap();
    assert_eq!(independent.raw_body(), b"source-b-body-v1");
    assert_eq!(independent.actual_user_agent(), Some("UA-source-b"));
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_revision_generation_and_corrupt_blob_fail_closed() {
    let root = root("cas-corruption");
    let store = Store::initialize(&root).expect("private synthetic store");
    let stale = create_source(
        &store,
        1,
        accepted_input(
            b"stale-body",
            "UA-stale",
            10_000,
            ImportFormat::MihomoYaml,
            "stale-node",
            "stale-secret",
            "warning",
        ),
    );
    assert!(matches!(
        stale,
        Err(ArtifactError::Store(StoreError::Conflict {
            expected: 1,
            current: 0
        }))
    ));
    assert_eq!(fs::read_dir(root.join("credentials")).unwrap().count(), 0);

    let created = create_source(
        &store,
        0,
        accepted_input(
            b"bound-body",
            "UA-bound",
            10_000,
            ImportFormat::MihomoYaml,
            "bound-node",
            "bound-secret",
            "warning",
        ),
    )
    .unwrap();
    let stable = store.read_snapshot().unwrap();
    let wrong_generation = update_source(
        &store,
        &created.source_id,
        stable.revision,
        2,
        accepted_input(
            b"new-body",
            "UA-new",
            10_001,
            ImportFormat::MihomoYaml,
            "new-node",
            "new-secret",
            "warning",
        ),
        10_001,
    );
    assert!(matches!(
        wrong_generation,
        Err(ArtifactError::Store(StoreError::SourceGenerationConflict {
            expected: 2,
            current: 1
        }))
    ));
    assert_eq!(store.read_snapshot().unwrap(), stable);
    assert_eq!(fs::read_dir(root.join("credentials")).unwrap().count(), 1);

    let artifact_ref = stable.sources[&created.source_id]
        .credential
        .as_ref()
        .unwrap()
        .credential_ref
        .clone();
    let blob = root
        .join("credentials")
        .join(format!("{artifact_ref}.blob"));
    fs::write(&blob, b"foreign replacement marker").unwrap();
    let error = read_source_artifact(&store, &created.source_id)
        .expect_err("store rejects altered immutable private material");
    assert!(matches!(error, ArtifactError::Store(_)));
    assert_eq!(fs::read(&blob).unwrap(), b"foreign replacement marker");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_artifact_records_local_origin_without_user_agent() {
    let root = root("local-origin");
    let store = Store::initialize(&root).expect("private synthetic store");
    let input = SourceImportInput::local(
        b"local-definition-body".to_vec(),
        30_000,
        vec![definition("local-node", "local-password-secret")],
        defaults("warning"),
    )
    .expect("typed local source");
    let receipt = create_source(&store, 0, input).expect("publish local source");
    let graph = store.read_snapshot().unwrap();
    let source = &graph.sources[&receipt.source_id];
    assert_eq!(source.kind, SourceKind::ManualServer);
    assert_eq!(
        source.provenance.as_ref().unwrap().origin,
        SourceOrigin::Local
    );
    let artifact = read_source_artifact(&store, &receipt.source_id).unwrap();
    assert_eq!(artifact.actual_user_agent(), None);
    assert_eq!(artifact.raw_body(), b"local-definition-body");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn typed_import_rejects_restricted_fields_defaults_and_resource_overflow() {
    let restricted = NodeDefinitionInput::from_trusted_classifier(
        "1.19.32",
        NodeProtocol::Vless,
        Transport::Tcp,
        json!({"name":"synthetic", "routing-mark": 7}),
    );
    assert!(matches!(restricted, Err(ArtifactError::RestrictedField)));
    let advanced = NodeDefinitionInput::from_trusted_classifier(
        "1.19.32",
        NodeProtocol::Vless,
        Transport::Tcp,
        json!({"name":"synthetic", "plugin":"outside-subset"}),
    );
    assert!(matches!(advanced, Err(ArtifactError::UnsupportedFeature)));
    assert!(matches!(
        NodeDefinitionInput::from_trusted_classifier(
            "1.19.31",
            NodeProtocol::Vless,
            Transport::Tcp,
            json!({"name":"synthetic"}),
        ),
        Err(ArtifactError::UnsupportedCoreVersion)
    ));
    assert!(matches!(
        GlobalDefaults::new(vec![("log-level".into(), json!("warn"))]),
        Err(ArtifactError::InvalidDefault)
    ));
    assert!(matches!(
        GlobalDefaults::new(vec![
            ("mode".into(), json!("rule")),
            ("mode".into(), json!("direct")),
        ]),
        Err(ArtifactError::DuplicateDefault)
    ));

    let mut deeply_nested = json!(null);
    for _ in 0..MAX_DEPTH_OVERFLOW {
        deeply_nested = Value::Array(vec![deeply_nested]);
    }
    assert!(matches!(
        NodeDefinitionInput::from_trusted_classifier(
            "1.19.32",
            NodeProtocol::Vless,
            Transport::Tcp,
            json!({"name":"synthetic", "nested":deeply_nested}),
        ),
        Err(ArtifactError::TooDeep)
    ));

    let too_many = (0..4097)
        .map(|number| definition(&format!("edge-{number}"), "synthetic-password"))
        .collect();
    assert!(matches!(
        SourceImportInput::local(b"fixture".to_vec(), 1, too_many, GlobalDefaults::default()),
        Err(ArtifactError::TooManyNodes)
    ));
    assert!(matches!(
        SourceImportInput::local(
            vec![b'x'; 8 * 1024 * 1024 + 1],
            1,
            vec![definition("edge", "password")],
            GlobalDefaults::default(),
        ),
        Err(ArtifactError::RawBodyTooLarge)
    ));
    assert!(matches!(
        SourceImportInput::local(b"empty".to_vec(), 1, Vec::new(), GlobalDefaults::default()),
        Err(ArtifactError::InvalidInput)
    ));
    assert!(matches!(
        SourceImportInput::local(
            b"negative-time".to_vec(),
            -1,
            vec![definition("edge", "password")],
            GlobalDefaults::default(),
        ),
        Err(ArtifactError::InvalidInput)
    ));
}

const MAX_DEPTH_OVERFLOW: usize = 65;
