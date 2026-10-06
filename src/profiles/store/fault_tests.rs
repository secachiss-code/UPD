//! Synthetic failure boundaries; compile preparation does not constitute execution.

use super::*;
use crate::profiles::{NodeProtocol, SCHEMA_VERSION, SourceKind, TlsVerification};

fn id(value: &str) -> Id {
    Id::new(value).unwrap()
}

fn initialized(label: &str) -> (PathBuf, Store) {
    let root = std::env::temp_dir().join(format!(
        "cm-i02-fault-{label}-{}-{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let store = Store::initialize(&root).unwrap();
    (root, store)
}

fn candidate_with_blobs() -> (GraphSnapshot, Vec<CredentialMaterial>) {
    let mut graph = GraphSnapshot::new();
    let source_id = id("source-fault");
    reserve_id(&mut graph, source_id.clone(), EntityIdKind::Source).unwrap();
    let mut materials = Vec::new();
    let mut refs = Vec::new();
    for suffix in ["a", "b"] {
        let credential_ref = id(&format!("credential-{suffix}"));
        let bytes = format!("synthetic-fault-{suffix}").into_bytes();
        let metadata = make_metadata(credential_ref.clone(), source_id.clone(), &bytes);
        reserve_id(
            &mut graph,
            credential_ref.clone(),
            EntityIdKind::CredentialRef,
        )
        .unwrap();
        graph.credentials.insert(credential_ref.clone(), metadata);
        materials.push(CredentialMaterial::new(credential_ref.clone(), bytes).unwrap());
        refs.push(credential_ref);
    }
    let node_id = id("node-fault");
    reserve_id(&mut graph, node_id.clone(), EntityIdKind::Node).unwrap();
    graph.nodes.insert(
        node_id.clone(),
        Node {
            schema_version: SCHEMA_VERSION,
            id: node_id.clone(),
            source_id: source_id.clone(),
            source_generation: 1,
            definition_digest_sha256: hex_sha256(b"synthetic-node"),
            protocol: NodeProtocol::Socks5,
            tls_verification: TlsVerification::NotApplicable,
            credential_refs: refs,
        },
    );
    graph.sources.insert(
        source_id.clone(),
        Source {
            schema_version: SCHEMA_VERSION,
            id: source_id,
            kind: SourceKind::ManualServer,
            generation: 1,
            content_digest_sha256: hex_sha256(b"synthetic-source"),
            credential: None,
            provenance: None,
            current_node_ids: vec![node_id],
        },
    );
    graph.validate().unwrap();
    (graph, materials)
}

fn arm(store: &Store, fault: TestFault) {
    *store.test_fault.lock().unwrap() = Some(fault);
}

#[test]
fn blob_sync_failure_keeps_old_snapshot_and_never_overwrites_orphan() {
    let (root, store) = initialized("blob-sync");
    arm(&store, TestFault::BlobSync);
    let (candidate, materials) = candidate_with_blobs();
    assert!(matches!(
        store.commit(0, candidate, materials),
        Err(StoreError::Io {
            operation: StoreIoOperation::Sync,
            ..
        })
    ));
    assert_eq!(store.read_snapshot().unwrap(), GraphSnapshot::new());
    let orphan = fs::read(root.join("credentials/credential-a.blob")).unwrap();
    drop(store);
    let reopened = Store::open(&root).unwrap();
    let (candidate, materials) = candidate_with_blobs();
    assert!(matches!(
        reopened.commit(0, candidate, materials),
        Err(StoreError::CredentialOccupied { .. })
    ));
    assert_eq!(
        fs::read(root.join("credentials/credential-a.blob")).unwrap(),
        orphan
    );
    assert_eq!(reopened.read_snapshot().unwrap().revision, 0);
    drop(reopened);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn temp_sync_and_rename_failures_preserve_old_state_and_foreign_temp() {
    for point in [TestFault::StateTempSync, TestFault::StateRename] {
        let (root, store) = initialized("before-publish");
        let foreign = root.join(".state.foreign.tmp");
        fs::write(&foreign, b"synthetic-foreign").unwrap();
        let old_bytes = fs::read(root.join(STATE_FILE)).unwrap();
        arm(&store, point);
        assert!(matches!(
            store.commit(0, GraphSnapshot::new(), Vec::new()),
            Err(StoreError::Io { .. })
        ));
        assert_eq!(fs::read(root.join(STATE_FILE)).unwrap(), old_bytes);
        assert_eq!(fs::read(&foreign).unwrap(), b"synthetic-foreign");
        let temps: Vec<_> = fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".state."))
            .collect();
        assert_eq!(temps.len(), 1);
        drop(store);
        assert_eq!(
            Store::open(&root)
                .unwrap()
                .read_snapshot()
                .unwrap()
                .revision,
            0
        );
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn directory_sync_failure_after_rename_reports_indeterminate_and_preserves_new_state() {
    let (root, store) = initialized("after-rename");
    arm(&store, TestFault::StateDirectorySync);
    assert!(matches!(
        store.commit(0, GraphSnapshot::new(), Vec::new()),
        Err(StoreError::DurabilityIndeterminate)
    ));
    drop(store);
    let reopened = Store::open(&root).unwrap();
    assert_eq!(reopened.read_snapshot().unwrap().revision, 1);
    drop(reopened);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn interrupted_unlink_resumes_exact_durable_plan_after_reopen() {
    let (root, store) = initialized("partial-unlink");
    let (candidate, materials) = candidate_with_blobs();
    store.commit(0, candidate, materials).unwrap();
    let plan_id = id("plan-fault");
    store
        .begin_removal(1, plan_id.clone(), id("source-fault"))
        .unwrap();
    arm(&store, TestFault::AfterBlobUnlink);
    assert!(matches!(
        store.finish_removal(&plan_id),
        Err(StoreError::Io {
            operation: StoreIoOperation::Unlink,
            ..
        })
    ));
    assert!(!root.join("credentials/credential-a.blob").exists());
    assert!(root.join("credentials/credential-b.blob").exists());
    drop(store);
    let reopened = Store::open(&root).unwrap();
    assert_eq!(
        reopened.read_snapshot().unwrap().staged_removals[&plan_id].stage,
        RemovalStage::BlobPending
    );
    let complete = reopened.finish_removal(&plan_id).unwrap();
    assert_eq!(
        complete.staged_removals[&plan_id].stage,
        RemovalStage::Complete
    );
    assert!(complete.credentials.is_empty());
    assert_eq!(reopened.finish_removal(&plan_id).unwrap(), complete);
    assert!(complete.issued_ids.contains(&id("credential-a")));
    drop(reopened);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn corrupt_second_planned_blob_preserves_the_first_and_the_pending_snapshot() {
    let (root, store) = initialized("corrupt-second");
    let (candidate, materials) = candidate_with_blobs();
    store.commit(0, candidate, materials).unwrap();
    let plan_id = id("plan-fault");
    store
        .begin_removal(1, plan_id.clone(), id("source-fault"))
        .unwrap();
    let pending_bytes = fs::read(root.join(STATE_FILE)).unwrap();
    fs::write(
        root.join("credentials/credential-b.blob"),
        b"synthetic-tampered",
    )
    .unwrap();
    assert!(matches!(
        store.finish_removal(&plan_id),
        Err(StoreError::CredentialMismatch)
    ));
    assert_eq!(
        fs::read(root.join("credentials/credential-a.blob")).unwrap(),
        b"synthetic-fault-a"
    );
    assert_eq!(
        fs::read(root.join("credentials/credential-b.blob")).unwrap(),
        b"synthetic-tampered"
    );
    assert_eq!(fs::read(root.join(STATE_FILE)).unwrap(), pending_bytes);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn completion_rename_failure_keeps_pending_plan_with_missing_blobs_recoverable() {
    let (root, store) = initialized("completion-rename");
    let (candidate, materials) = candidate_with_blobs();
    store.commit(0, candidate, materials).unwrap();
    let plan_id = id("plan-fault");
    store
        .begin_removal(1, plan_id.clone(), id("source-fault"))
        .unwrap();
    arm(&store, TestFault::StateRename);
    assert!(matches!(
        store.finish_removal(&plan_id),
        Err(StoreError::Io {
            operation: StoreIoOperation::Rename,
            ..
        })
    ));
    assert!(
        fs::read_dir(root.join(CREDENTIAL_DIRECTORY))
            .unwrap()
            .next()
            .is_none()
    );
    drop(store);
    let reopened = Store::open(&root).unwrap();
    assert_eq!(reopened.read_snapshot().unwrap().revision, 2);
    assert_eq!(reopened.finish_removal(&plan_id).unwrap().revision, 3);
    drop(reopened);
    fs::remove_dir_all(root).unwrap();
}

/// v1 → v2: provenance gains empty omissions, nodes gain `not_applicable` TLS.
#[test]
fn v1_graph_migrates_to_v2_with_empty_omissions() {
    let v1 = serde_json::json!({
        "schema_version": 1,
        "sources": {"s": {"schema_version": 1, "provenance": {"schema_version": 1}}},
        "nodes": {"n": {"schema_version": 1}},
    });
    let migrated = migrate_graph_json(v1).unwrap();
    assert_eq!(migrated["schema_version"], SCHEMA_VERSION);
    let provenance = &migrated["sources"]["s"]["provenance"];
    assert_eq!(
        provenance["omissions"]["section_names"],
        serde_json::json!([])
    );
    assert_eq!(
        provenance["accepted_omissions_bound"]["tls_verification_disabled_count"],
        0
    );
    assert_eq!(migrated["nodes"]["n"]["tls_verification"], "not_applicable");
    assert!(!has_schema_other_than(&migrated, u64::from(SCHEMA_VERSION)));
}

/// A v1 file carrying a nested record of another version is refused, not relabelled as v2.
#[test]
fn v1_graph_with_foreign_nested_version_is_refused() {
    for nested in [0, 2, 3] {
        let v1 = serde_json::json!({
            "schema_version": 1,
            "sources": {"s": {"schema_version": 1, "provenance": {"schema_version": nested}}},
        });
        assert!(matches!(
            migrate_graph_json(v1),
            Err(StoreError::UnsupportedSchema)
        ));
    }
    let future = serde_json::json!({"schema_version": 3});
    assert!(matches!(
        migrate_graph_json(future),
        Err(StoreError::UnsupportedSchema)
    ));
}
