//! Synthetic, compile-checked storage regressions. These tests are intentionally not run here.

use cm::profiles::*;
use std::fs::{self, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicU64, Ordering},
};
use std::thread;
use std::time::Duration;

#[path = "support/profiles.rs"]
mod profile_fixture;
use profile_fixture::{digest, fixture, id, issue, net_evidence, source_update};

static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);

#[test]
fn stored_sessions_survive_read_in_a_new_process() {
    const CHILD_ROOT: &str = "CM_I02_STORE_RESTART_FIXTURE";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let store = Store::open(PathBuf::from(root)).unwrap();
        let mut expected = fixture();
        expected.revision = 1;
        assert_eq!(store.read_snapshot().unwrap(), expected);
        for number in [1, 2] {
            let status =
                PublicSessionStatus::from_graph(&expected, &id(&format!("session-{number}")))
                    .unwrap();
            assert_eq!(status.tunnel_generation, 1);
        }
        return;
    }
    let (root, store) = initialized_store("new-process");
    seed_store(&store);
    drop(store);
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "stored_sessions_survive_read_in_a_new_process"])
        .env(CHILD_ROOT, &root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "independent snapshot reader failed"
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("1 passed")
    );
    fs::remove_dir_all(root).unwrap();
}

fn root_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "cm-i02-store-{label}-{}-{}",
        std::process::id(),
        NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn materials() -> Vec<CredentialMaterial> {
    vec![
        CredentialMaterial::new(id("credential-a"), b"synthetic-private-a".to_vec()).unwrap(),
        CredentialMaterial::new(id("credential-b"), b"synthetic-private-b".to_vec()).unwrap(),
    ]
}

fn initialized_store(label: &str) -> (PathBuf, Store) {
    let root = root_path(label);
    let store = Store::initialize(&root).unwrap();
    (root, store)
}

fn seed_store(store: &Store) -> GraphSnapshot {
    store.commit(0, fixture(), materials()).unwrap()
}

fn detach_source_b(store: &Store, graph: &GraphSnapshot) -> GraphSnapshot {
    let mut detached = graph.clone();
    detached.applications.remove(&id("app-3"));
    detached.tunnel_instances.remove(&id("tunnel-own"));
    detached.connection_profiles.remove(&id("profile-b"));
    store.commit(graph.revision, detached, Vec::new()).unwrap()
}

#[test]
fn private_layout_and_credential_material_remain_redacted() {
    let (root, store) = initialized_store("private");
    let graph = seed_store(&store);
    let root_metadata = fs::metadata(&root).unwrap();
    let credentials_metadata = fs::metadata(root.join("credentials")).unwrap();
    let lock_metadata = fs::metadata(root.join(".lock")).unwrap();
    assert_eq!(root_metadata.mode() & 0o7777, 0o700);
    assert_eq!(credentials_metadata.mode() & 0o7777, 0o700);
    assert_eq!(lock_metadata.mode() & 0o7777, 0o600);
    assert_eq!(graph.revision, 1);

    let material = store.read_credential(&id("credential-a")).unwrap();
    assert_eq!(material.as_bytes(), b"synthetic-private-a");
    assert!(!format!("{material:?}").contains("synthetic-private-a"));
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn same_store_parallel_writers_have_one_cas_winner() {
    let (root, store) = initialized_store("same-store-cas");
    let store = Arc::new(store);
    let barrier = Arc::new(Barrier::new(3));
    let mut joins = Vec::new();
    for _ in 0..2 {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        joins.push(thread::spawn(move || {
            barrier.wait();
            store.commit(0, fixture(), materials())
        }));
    }
    barrier.wait();
    let results: Vec<_> = joins.into_iter().map(|join| join.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(StoreError::Conflict { .. })))
            .count(),
        1
    );
    assert_eq!(store.read_snapshot().unwrap().revision, 1);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn partial_blob_unlink_survives_store_reopen_and_finishes_exact_plan() {
    let (root, store) = initialized_store("removal-restart");
    let mut graph = seed_store(&store);
    graph = detach_source_b(&store, &graph);

    let plan_id = id("remove-source-b");
    let pending = store
        .begin_removal(graph.revision, plan_id.clone(), id("source-b"))
        .unwrap();
    assert_eq!(
        pending.staged_removals[&plan_id].stage,
        RemovalStage::BlobPending
    );
    drop(store);

    // Simulate a restart after one idempotent unlink in the durable pending stage.
    fs::remove_file(root.join("credentials/credential-b.blob")).unwrap();
    let reopened = Store::open(&root).unwrap();
    let complete = reopened.finish_removal(&plan_id).unwrap();
    assert_eq!(
        complete.staged_removals[&plan_id].stage,
        RemovalStage::Complete
    );
    assert!(!complete.credentials.contains_key(&id("credential-b")));
    assert!(root.join("credentials/credential-a.blob").exists());
    drop(reopened);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn nofollow_state_rejects_symlink_and_foreign_temporary_files_are_preserved() {
    let (root, store) = initialized_store("nofollow");
    let graph = seed_store(&store);
    let foreign_temp = root.join(".state.foreign.tmp");
    fs::write(&foreign_temp, b"foreign synthetic marker").unwrap();
    let mut candidate = graph.clone();
    candidate.revision += 1;
    store.commit(graph.revision, candidate, Vec::new()).unwrap();
    assert_eq!(
        fs::read(&foreign_temp).unwrap(),
        b"foreign synthetic marker"
    );
    drop(store);

    fs::remove_file(root.join("state.json")).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", root.join("state.json")).unwrap();
    assert!(matches!(
        Store::open(&root),
        Err(StoreError::UnsafeFilesystem)
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stale_revision_does_not_publish_credentials_or_overwrite_state() {
    let (root, store) = initialized_store("cas-conflict");
    let initial = seed_store(&store);
    let mut stale = initial.clone();
    stale.revision += 1;
    let error = store.commit(0, stale, Vec::new()).unwrap_err();
    assert!(matches!(
        error,
        StoreError::Conflict {
            expected: 0,
            current: 1
        }
    ));
    assert_eq!(store.read_snapshot().unwrap(), initial);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_update_archives_old_node_and_changes_only_net_evidence_for_live_pins() {
    let (root, store) = initialized_store("source-update");
    let old = seed_store(&store);
    let mut proposed = source_update(&old, false);
    net_evidence(&mut proposed, VerificationValue::Blocked, 2_000);
    GraphSnapshot::validate_transition(&old, &proposed).unwrap();

    let updated = store
        .update_source(
            old.revision,
            1,
            proposed.sources[&id("source-a")].clone(),
            vec![proposed.nodes[&id("node-a-2")].clone()],
            Vec::new(),
            2_000,
        )
        .unwrap();
    assert!(updated.nodes.contains_key(&id("node-a-1")));
    assert_eq!(updated.sessions, old.sessions);
    for session_id in [id("session-1"), id("session-2")] {
        let status = PublicSessionStatus::from_graph(&updated, &session_id).unwrap();
        assert_eq!(status.verification.net, VerificationValue::Blocked);
        assert_eq!(status.verification.region, VerificationValue::Unknown);
        assert_eq!(status.verification.state, VerificationValue::Unknown);
        assert_eq!(status.verification.app, VerificationValue::Unknown);
    }
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn separate_store_handles_observe_one_cas_winner_and_busy_lock_is_bounded() {
    let (root, store) = initialized_store("separate-handles");
    seed_store(&store);
    drop(store);
    let first = Arc::new(Store::open(&root).unwrap());
    let second = Arc::new(Store::open(&root).unwrap());
    let barrier = Arc::new(Barrier::new(3));
    let left = {
        let first = Arc::clone(&first);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            first.commit(1, fixture(), Vec::new())
        })
    };
    let right = {
        let second = Arc::clone(&second);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            second.commit(1, fixture(), Vec::new())
        })
    };
    barrier.wait();
    let results = [left.join().unwrap(), right.join().unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(StoreError::Conflict { .. })))
            .count(),
        1
    );

    let busy = Store::open(&root)
        .unwrap()
        .with_lock_timeout(Duration::from_millis(10));
    let external_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join(".lock"))
        .unwrap();
    assert_eq!(
        unsafe { libc::flock(external_lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    assert!(matches!(busy.read_snapshot(), Err(StoreError::Busy)));
    drop(busy);
    assert_eq!(
        unsafe { libc::flock(external_lock.as_raw_fd(), libc::LOCK_UN) },
        0
    );
    drop(external_lock);
    drop(first);
    drop(second);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_in_use_and_immutable_node_errors_include_safe_graph_context() {
    let (root, store) = initialized_store("typed-context");
    let graph = seed_store(&store);
    let source_error = store
        .begin_removal(1, id("plan-a"), id("source-a"))
        .unwrap_err();
    match source_error {
        StoreError::SourceInUse {
            source_id,
            references,
        } => {
            assert_eq!(source_id, id("source-a"));
            assert!(references.contains(&id("profile-a")));
            assert!(references.contains(&id("session-1")));
        }
        other => panic!("unexpected safe error variant: {other:?}"),
    }

    let mut altered = graph.clone();
    altered
        .nodes
        .get_mut(&id("node-a-1"))
        .unwrap()
        .definition_digest_sha256 = digest(b"changed");
    let node_error = store
        .commit(graph.revision, altered, Vec::new())
        .unwrap_err();
    match node_error {
        StoreError::ImmutableNode {
            node_id,
            references,
        } => {
            assert_eq!(node_id, id("node-a-1"));
            assert!(references.contains(&id("profile-a")));
            assert!(references.contains(&id("session-1")));
        }
        other => panic!("unexpected safe error variant: {other:?}"),
    }
    assert_eq!(store.read_snapshot().unwrap(), graph);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn corrupt_pending_blob_aborts_before_unlinking_any_planned_blob() {
    let (root, store) = initialized_store("corrupt-removal");
    let graph = seed_store(&store);
    let detached = detach_source_b(&store, &graph);
    let plan_id = id("remove-source-b-corrupt");
    store
        .begin_removal(detached.revision, plan_id.clone(), id("source-b"))
        .unwrap();
    fs::write(
        root.join("credentials/credential-b.blob"),
        b"synthetic tampered content",
    )
    .unwrap();
    assert!(matches!(
        store.finish_removal(&plan_id),
        Err(StoreError::CredentialMismatch)
    ));
    assert_eq!(
        fs::read(root.join("credentials/credential-a.blob")).unwrap(),
        b"synthetic-private-a"
    );
    assert_eq!(
        fs::read(root.join("credentials/credential-b.blob")).unwrap(),
        b"synthetic tampered content"
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn occupied_new_blob_is_preserved_and_old_snapshot_stays_published() {
    let (root, store) = initialized_store("blob-collision");
    let old = seed_store(&store);
    let secret = b"synthetic-private-c";
    let source_id = id("source-c");
    let node_id = id("node-c-1");
    let credential_ref = id("credential-c");
    let metadata = CredentialMetadata {
        schema_version: SCHEMA_VERSION,
        credential_ref: credential_ref.clone(),
        owner_source_id: source_id.clone(),
        digest_sha256: digest(secret),
        size_bytes: secret.len() as u64,
    };
    let mut candidate = old.clone();
    candidate.sources.insert(
        source_id.clone(),
        Source {
            schema_version: SCHEMA_VERSION,
            id: source_id.clone(),
            kind: SourceKind::ManualServer,
            generation: 1,
            content_digest_sha256: digest(b"source-c-input"),
            credential: Some(metadata.clone()),
            provenance: None,
            current_node_ids: vec![node_id.clone()],
        },
    );
    candidate.nodes.insert(
        node_id.clone(),
        Node {
            schema_version: SCHEMA_VERSION,
            id: node_id.clone(),
            source_id: source_id.clone(),
            source_generation: 1,
            definition_digest_sha256: digest(b"node-c-definition"),
            protocol: NodeProtocol::Vless,
            credential_refs: vec![credential_ref.clone()],
        },
    );
    candidate
        .credentials
        .insert(credential_ref.clone(), metadata);
    issue(&mut candidate, "source-c", EntityIdKind::Source);
    issue(&mut candidate, "node-c-1", EntityIdKind::Node);
    issue(&mut candidate, "credential-c", EntityIdKind::CredentialRef);
    let orphan_path = root.join("credentials/credential-c.blob");
    fs::write(&orphan_path, b"foreign orphan marker").unwrap();

    let error = store
        .commit(
            old.revision,
            candidate,
            vec![CredentialMaterial::new(credential_ref, secret.to_vec()).unwrap()],
        )
        .unwrap_err();
    assert!(matches!(error, StoreError::CredentialOccupied { .. }));
    assert_eq!(store.read_snapshot().unwrap(), old);
    assert_eq!(fs::read(orphan_path).unwrap(), b"foreign orphan marker");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn host_stop_changes_only_host_owned_tunnel_policy() {
    let (root, store) = initialized_store("host-stop");
    let mut graph = fixture();
    let host_tunnel_id = id("tunnel-host");
    graph.tunnel_instances.insert(
        host_tunnel_id.clone(),
        TunnelInstance {
            schema_version: SCHEMA_VERSION,
            id: host_tunnel_id.clone(),
            connection_profile_id: id("profile-a"),
            generation: 1,
            owner: TunnelOwner::Host,
            lifecycle: TunnelLifecycle::Running,
        },
    );
    issue(&mut graph, "tunnel-host", EntityIdKind::TunnelInstance);
    let host = graph.host_policy.as_mut().unwrap();
    host.mode = HostMode::Tunnel;
    host.connection_profile_id = Some(id("profile-a"));
    host.tunnel_instance_id = Some(host_tunnel_id.clone());
    let before = store.commit(0, graph, materials()).unwrap();
    let after = store.stop_host(before.revision).unwrap();
    assert_eq!(after.host_policy.as_ref().unwrap().mode, HostMode::Off);
    assert_eq!(
        after.host_policy.as_ref().unwrap().connection_profile_id,
        None
    );
    assert_eq!(
        after.tunnel_instances[&host_tunnel_id].lifecycle,
        TunnelLifecycle::Stopped
    );
    assert_eq!(after.application_groups, before.application_groups);
    assert_eq!(after.applications, before.applications);
    assert_eq!(after.sessions, before.sessions);
    assert_eq!(
        after.tunnel_instances[&id("tunnel-group")],
        before.tunnel_instances[&id("tunnel-group")]
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unsupported_unknown_and_duplicate_state_documents_fail_without_echoing_input() {
    let (root, store) = initialized_store("strict-state");
    seed_store(&store);
    drop(store);

    let state_path = root.join("state.json");
    let bytes = fs::read(&state_path).unwrap();
    let mut unknown: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    unknown["private_marker"] = "synthetic-secret-marker".into();
    fs::write(&state_path, serde_json::to_vec(&unknown).unwrap()).unwrap();
    let error = Store::open(&root)
        .err()
        .expect("unsupported state should be rejected");
    assert!(matches!(error, StoreError::CorruptState));
    assert!(!format!("{error:?}").contains("synthetic-secret-marker"));

    let mut unsupported: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    unsupported["schema_version"] = 999.into();
    fs::write(&state_path, serde_json::to_vec(&unsupported).unwrap()).unwrap();
    assert!(matches!(
        Store::open(&root),
        Err(StoreError::UnsupportedSchema)
    ));

    let text = String::from_utf8(bytes).unwrap();
    let duplicated = format!("{{\"schema_version\":999,{}", &text[1..]);
    fs::write(&state_path, duplicated.as_bytes()).unwrap();
    assert!(matches!(Store::open(&root), Err(StoreError::CorruptState)));
    fs::remove_dir_all(root).unwrap();
}
