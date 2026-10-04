//! Synthetic model regressions. Prepared for installation-time execution.
use cm::profiles::*;

#[path = "support/profiles.rs"]
mod profile_fixture;
use profile_fixture::*;

#[test]
fn two_sources_three_apps_and_group_roundtrip_without_secret_material() {
    let graph = fixture();
    graph.validate().unwrap();
    let restored: GraphSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&graph).unwrap()).unwrap();
    assert_eq!(graph, restored);
    assert_eq!(
        graph.application_groups[&id("group")]
            .application_ids(&graph.applications)
            .len(),
        2
    );
    let public =
        serde_json::to_string(&graph.public_source_status(&id("source-a")).unwrap()).unwrap();
    assert!(!public.contains("synthetic-private-a"));
    assert!(!public.contains("executable"));
    let session = PublicSessionStatus::from_graph(&graph, &id("session-1")).unwrap();
    assert_eq!(session.verification, VerificationAxes::default());
}

#[test]
fn unchanged_definition_with_new_ids_preserves_sessions_without_net_block() {
    let old = fixture();
    let new = source_update(&old, true);
    GraphSnapshot::validate_transition(&old, &new).unwrap();
    assert_eq!(old.sessions, new.sessions);
    assert_eq!(old.verifications, new.verifications);
}

#[test]
fn disappeared_definition_requires_net_evidence_for_each_live_session() {
    let old = fixture();
    let mut new = source_update(&old, false);
    assert!(GraphSnapshot::validate_transition(&old, &new).is_err());
    net_evidence(&mut new, VerificationValue::Blocked, 2000);
    GraphSnapshot::validate_transition(&old, &new).unwrap();
    for session in new.sessions.keys() {
        let status = PublicSessionStatus::from_graph(&new, session).unwrap();
        assert_eq!(status.verification.net, VerificationValue::Blocked);
        assert_eq!(status.verification.region, VerificationValue::Unknown);
        assert_eq!(status.verification.state, VerificationValue::Unknown);
        assert_eq!(status.verification.app, VerificationValue::Unknown);
    }
}

#[test]
fn already_blocked_pin_accepts_fresh_evidence_when_definition_disappears() {
    let mut old = fixture();
    net_evidence(&mut old, VerificationValue::Blocked, 1500);
    let mut new = source_update(&old, false);
    net_evidence(&mut new, VerificationValue::Blocked, 2000);
    GraphSnapshot::validate_transition(&old, &new).unwrap();
    net_evidence(&mut new, VerificationValue::Blocked, 1500);
    assert!(GraphSnapshot::validate_transition(&old, &new).is_err());
}

#[test]
fn existing_session_remains_pinned_after_current_tunnel_and_profile_change() {
    let old = fixture();
    let mut new = source_update(&old, true);
    new.tunnel_instances
        .get_mut(&id("tunnel-group"))
        .unwrap()
        .generation = 2;
    new.connection_profiles
        .get_mut(&id("profile-a"))
        .unwrap()
        .node_selection = NodeSelection::Pinned {
        node: SourceNodeRef {
            source_id: id("source-a"),
            source_generation: 2,
            node_id: id("node-a-2"),
        },
    };
    GraphSnapshot::validate_transition(&old, &new).unwrap();
    assert_eq!(old.sessions, new.sessions);
}

#[test]
fn nodes_environment_browser_and_active_pins_cannot_be_rewritten() {
    let old = fixture();
    for mutate in [0, 1, 2, 3] {
        let mut new = old.clone();
        new.revision += 1;
        match mutate {
            0 => {
                new.nodes
                    .get_mut(&id("node-a-1"))
                    .unwrap()
                    .definition_digest_sha256 = digest(b"replacement")
            }
            1 => {
                new.environment_profiles
                    .get_mut(&id("environment-1"))
                    .unwrap()
                    .timezone = "Europe/Moscow".into()
            }
            2 => {
                new.browser_identity_profiles
                    .get_mut(&id("browser-1"))
                    .unwrap()
                    .label = Some("changed".into())
            }
            _ => {
                new.sessions
                    .get_mut(&id("session-1"))
                    .unwrap()
                    .created_at_unix_ms = 1200
            }
        }
        assert!(
            GraphSnapshot::validate_transition(&old, &new).is_err(),
            "mutation {mutate}"
        );
    }
}

#[test]
fn tombstone_id_cannot_be_reused_for_same_or_different_entity_kind() {
    let old = fixture();
    let mut removed = old.clone();
    removed.revision += 1;
    removed.browser_identity_profiles.remove(&id("browser-3"));
    GraphSnapshot::validate_transition(&old, &removed).unwrap();
    let mut resurrected = removed.clone();
    resurrected.revision += 1;
    resurrected.browser_identity_profiles.insert(
        id("browser-3"),
        old.browser_identity_profiles[&id("browser-3")].clone(),
    );
    assert!(GraphSnapshot::validate_transition(&removed, &resurrected).is_err());
    let mut reused = removed.clone();
    reused.revision += 1;
    reused.data_profiles.insert(
        id("browser-3"),
        DataProfile {
            schema_version: SCHEMA_VERSION,
            id: id("browser-3"),
        },
    );
    reused
        .issued_id_kinds
        .insert(id("browser-3"), EntityIdKind::DataProfile);
    assert!(GraphSnapshot::validate_transition(&removed, &reused).is_err());
}

#[test]
fn ending_one_group_session_preserves_other_session_and_host_policy() {
    let old = fixture();
    let mut new = old.clone();
    new.revision += 1;
    let session = new.sessions.get_mut(&id("session-1")).unwrap();
    session.lifecycle = SessionLifecycle::Ended;
    session.ended_at_unix_ms = Some(2000);
    GraphSnapshot::validate_transition(&old, &new).unwrap();
    assert_eq!(
        old.sessions[&id("session-2")],
        new.sessions[&id("session-2")]
    );
    assert_eq!(old.host_policy, new.host_policy);
}

#[test]
fn unknown_schema_fields_and_path_ids_are_rejected_on_deserialize() {
    let graph = fixture();
    let mut value = serde_json::to_value(&graph).unwrap();
    value["schema_version"] = 999.into();
    assert!(serde_json::from_value::<GraphSnapshot>(value).is_err());
    let mut source = serde_json::to_value(&graph.sources[&id("source-a")]).unwrap();
    source["schema_version"] = 999.into();
    assert!(serde_json::from_value::<Source>(source).is_err());
    let mut value = serde_json::to_value(&graph).unwrap();
    value["foreign"] = true.into();
    assert!(serde_json::from_value::<GraphSnapshot>(value).is_err());
    for unsafe_id in ["", "../outside", "/absolute", "https://endpoint", "a.b"] {
        assert!(Id::new(unsafe_id).is_err());
        assert!(serde_json::from_value::<Id>(unsafe_id.into()).is_err());
    }
}

#[test]
fn source_removal_requires_detached_references_and_durable_blob_plan() {
    let old = fixture();
    let mut detached = old.clone();
    detached.revision += 1;
    detached.applications.remove(&id("app-3"));
    detached.tunnel_instances.remove(&id("tunnel-own"));
    detached.connection_profiles.remove(&id("profile-b"));
    GraphSnapshot::validate_transition(&old, &detached).unwrap();
    let mut pending = detached.clone();
    pending.revision += 1;
    pending.sources.remove(&id("source-b"));
    pending.nodes.remove(&id("node-b-1"));
    assert!(GraphSnapshot::validate_transition(&detached, &pending).is_err());
    let metadata = pending.credentials[&id("credential-b")].clone();
    issue(&mut pending, "remove-b", EntityIdKind::RemovalPlan);
    pending.staged_removals.insert(
        id("remove-b"),
        StagedRemoval {
            schema_version: SCHEMA_VERSION,
            plan_id: id("remove-b"),
            source_id: id("source-b"),
            credential_metadata: vec![metadata],
            removed_credential_refs: Default::default(),
            stage: RemovalStage::BlobPending,
        },
    );
    GraphSnapshot::validate_transition(&detached, &pending).unwrap();
    assert!(pending.credentials.contains_key(&id("credential-b")));
    let mut reused_pending = pending.clone();
    reused_pending
        .nodes
        .get_mut(&id("node-a-1"))
        .unwrap()
        .credential_refs
        .push(id("credential-b"));
    assert!(reused_pending.validate().is_err());
    let mut completed = pending.clone();
    completed.revision += 1;
    completed.credentials.remove(&id("credential-b"));
    let removal = completed.staged_removals.get_mut(&id("remove-b")).unwrap();
    removal.stage = RemovalStage::Complete;
    removal.removed_credential_refs.insert(id("credential-b"));
    GraphSnapshot::validate_transition(&pending, &completed).unwrap();
    assert!(completed.issued_ids.contains(&id("source-b")));
    assert!(completed.issued_ids.contains(&id("credential-b")));
    let mut resurrected = completed.clone();
    resurrected.revision += 1;
    let mut metadata = old.credentials[&id("credential-b")].clone();
    metadata.owner_source_id = id("source-a");
    resurrected.credentials.insert(id("credential-b"), metadata);
    assert!(GraphSnapshot::validate_transition(&completed, &resurrected).is_err());
}

#[test]
fn shared_credential_survives_origin_removal_then_belongs_to_last_users_plan() {
    let mut old = fixture();
    old.sessions.clear();
    old.environment_profiles.clear();
    old.tunnel_instances
        .get_mut(&id("tunnel-group"))
        .unwrap()
        .connection_profile_id = id("profile-b");
    old.nodes.get_mut(&id("node-b-1")).unwrap().credential_refs = vec![id("credential-a")];
    old.sources.get_mut(&id("source-b")).unwrap().credential =
        Some(old.credentials[&id("credential-a")].clone());
    old.connection_profiles.remove(&id("profile-a"));
    old.validate().unwrap();
    let mut pending_a = old.clone();
    pending_a.revision += 1;
    pending_a.sources.remove(&id("source-a"));
    pending_a.nodes.remove(&id("node-a-1"));
    issue(&mut pending_a, "remove-a", EntityIdKind::RemovalPlan);
    pending_a.staged_removals.insert(
        id("remove-a"),
        StagedRemoval {
            schema_version: SCHEMA_VERSION,
            plan_id: id("remove-a"),
            source_id: id("source-a"),
            credential_metadata: vec![],
            removed_credential_refs: Default::default(),
            stage: RemovalStage::BlobPending,
        },
    );
    GraphSnapshot::validate_transition(&old, &pending_a).unwrap();
    let mut complete_a = pending_a.clone();
    complete_a.revision += 1;
    complete_a
        .staged_removals
        .get_mut(&id("remove-a"))
        .unwrap()
        .stage = RemovalStage::Complete;
    GraphSnapshot::validate_transition(&pending_a, &complete_a).unwrap();
    assert!(complete_a.credentials.contains_key(&id("credential-a")));
    let mut detached_b = complete_a.clone();
    detached_b.revision += 1;
    detached_b.applications.clear();
    detached_b.application_groups.clear();
    detached_b.tunnel_instances.clear();
    detached_b.connection_profiles.clear();
    GraphSnapshot::validate_transition(&complete_a, &detached_b).unwrap();
    let mut pending_b = detached_b.clone();
    pending_b.revision += 1;
    pending_b.sources.remove(&id("source-b"));
    pending_b.nodes.remove(&id("node-b-1"));
    issue(&mut pending_b, "remove-b", EntityIdKind::RemovalPlan);
    let metadata = pending_b.credentials.values().cloned().collect();
    pending_b.staged_removals.insert(
        id("remove-b"),
        StagedRemoval {
            schema_version: SCHEMA_VERSION,
            plan_id: id("remove-b"),
            source_id: id("source-b"),
            credential_metadata: metadata,
            removed_credential_refs: Default::default(),
            stage: RemovalStage::BlobPending,
        },
    );
    GraphSnapshot::validate_transition(&detached_b, &pending_b).unwrap();
    let mut complete_b = pending_b.clone();
    complete_b.revision += 1;
    complete_b.credentials.clear();
    let plan = complete_b.staged_removals.get_mut(&id("remove-b")).unwrap();
    plan.stage = RemovalStage::Complete;
    plan.removed_credential_refs = plan
        .credential_metadata
        .iter()
        .map(|value| value.credential_ref.clone())
        .collect();
    GraphSnapshot::validate_transition(&pending_b, &complete_b).unwrap();
}

#[test]
fn duplicate_record_fields_dictionary_ids_and_registry_ids_are_ambiguous() {
    let graph = fixture();
    let source = serde_json::to_string(&graph.sources[&id("source-a")]).unwrap();
    let duplicate_schema = source.replacen(
        "\"schema_version\":1",
        "\"schema_version\":999,\"schema_version\":1",
        1,
    );
    assert!(serde_json::from_str::<Source>(&duplicate_schema).is_err());
    let snapshot = serde_json::to_string(&graph).unwrap();
    let mut value = serde_json::to_value(&graph).unwrap();
    value["sources"] = serde_json::json!({});
    let base = serde_json::to_string(&value).unwrap();
    let duplicate_sources = base.replacen(
        "\"sources\":{}",
        &format!("\"sources\":{{\"source-a\":{source},\"source-a\":{source}}}"),
        1,
    );
    assert!(serde_json::from_str::<GraphSnapshot>(&duplicate_sources).is_err());
    let duplicated_registry =
        snapshot.replacen("\"issued_ids\":[", "\"issued_ids\":[\"source-a\",", 1);
    assert!(serde_json::from_str::<GraphSnapshot>(&duplicated_registry).is_err());
}

#[test]
fn net_verification_never_promotes_region_state_or_app_axes() {
    let old = fixture();
    let mut candidate = old.clone();
    candidate.revision += 1;
    net_evidence(&mut candidate, VerificationValue::Verified, 2_000);
    GraphSnapshot::validate_transition(&old, &candidate).unwrap();
    for number in [1, 2] {
        let status =
            PublicSessionStatus::from_graph(&candidate, &id(&format!("session-{number}"))).unwrap();
        assert_eq!(status.verification.net, VerificationValue::Verified);
        assert_eq!(status.verification.region, VerificationValue::Unknown);
        assert_eq!(status.verification.state, VerificationValue::Unknown);
        assert_eq!(status.verification.app, VerificationValue::Unknown);
    }
    assert_eq!(candidate.sessions, old.sessions);
    assert_eq!(candidate.application_groups, old.application_groups);
    assert_eq!(candidate.host_policy, old.host_policy);
}
