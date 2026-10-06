#[path = "support/profiles.rs"]
mod support;

use cm::profiles::{
    ImportOmissions, SkippedLineClass, SkippedLines, import_omissions_digest, omissions_within_bound,
};
use cm::sources::artifact::ArtifactError;
use cm::sources::import_confirmation::{apply_confirmation, evaluate_auto_refresh, pending_import_summary};
use cm::sources::artifact::{GlobalDefaults, NodeDefinitionInput, SourceImportInput};
use cm::sources::{PINNED_CORE_VERSION, capabilities::Transport};
use serde_json::json;
use support::{fixture, id};

fn sample_omissions() -> ImportOmissions {
    ImportOmissions {
        section_names: vec!["proxy-groups".into(), "rules".into()],
        skipped_lines: vec![SkippedLines {
            class: SkippedLineClass::UnsupportedFeature,
            line_numbers: vec![12, 44],
        }],
        tls_verification_disabled_count: 1,
    }
}

#[test]
fn omissions_digest_is_stable_and_safe() {
    let omissions = sample_omissions();
    let digest = import_omissions_digest(&omissions).unwrap();
    assert_eq!(digest.len(), 64);
    let text = format!("{omissions:?}");
    assert!(!text.contains("secret"));
}

#[test]
fn pending_summary_requires_confirmation() {
    let summary = pending_import_summary(sample_omissions()).unwrap();
    assert_eq!(
        summary.omissions_digest_sha256,
        import_omissions_digest(&summary.omissions).unwrap()
    );
}

#[test]
fn wrong_confirmation_digest_is_rejected() {
    let omissions = sample_omissions();
    let body = br#"{"proxies":[{"name":"n","type":"ss","server":"edge.example","port":443,"cipher":"aes-128-gcm","password":"x"}]}"#;
    let err = apply_confirmation(
        SourceImportInput::local(
            body.to_vec(),
            1_000,
            vec![NodeDefinitionInput::from_trusted_classifier(
                PINNED_CORE_VERSION,
                cm::profiles::NodeProtocol::Shadowsocks,
                Transport::Tcp,
                json!({
                    "name": "n",
                    "type": "ss",
                    "server": "edge.example",
                    "port": 443,
                    "cipher": "aes-128-gcm",
                    "password": "x"
                }),
            )
            .unwrap()],
            GlobalDefaults::default(),
        )
        .unwrap(),
        omissions,
        "deadbeef",
    )
    .unwrap_err();
    assert!(matches!(err, ArtifactError::OmissionsDigestMismatch));
}

#[test]
fn auto_refresh_requires_source_provenance() {
    let graph = fixture();
    let source = graph.sources.get(&id("source-a")).unwrap();
    assert!(source.provenance.is_none());
    let body = br#"{"proxies":[{"name":"n","type":"ss","server":"edge.example","port":443,"cipher":"aes-128-gcm","password":"x"}]}"#;
    let input = SourceImportInput::local(
        body.to_vec(),
        2_000,
        vec![NodeDefinitionInput::from_trusted_classifier(
            PINNED_CORE_VERSION,
            cm::profiles::NodeProtocol::Shadowsocks,
            Transport::Tcp,
            json!({
                "name": "n",
                "type": "ss",
                "server": "edge.example",
                "port": 443,
                "cipher": "aes-128-gcm",
                "password": "x"
            }),
        )
        .unwrap()],
        GlobalDefaults::default(),
    )
    .unwrap();
    assert!(matches!(
        evaluate_auto_refresh(source, &input).unwrap_err(),
        ArtifactError::SourceArtifactMissing
    ));
}

#[test]
fn omissions_bound_enforces_section_set_equality() {
    let bound = sample_omissions();
    let narrower = ImportOmissions {
        section_names: vec!["proxy-groups".into()],
        skipped_lines: bound.skipped_lines.clone(),
        tls_verification_disabled_count: 0,
    };
    assert!(!omissions_within_bound(&narrower, &bound));
    assert!(omissions_within_bound(&bound, &bound));
}

// ---- Coordinator review 2026-10-06: regressions for defects fixed in I03.T04.y ----

mod review {
    use cm::profiles::{
        ImportOmissions, NodeProtocol, SkippedLineClass, SkippedLines, Store, TlsVerification,
        import_omissions_digest,
    };
    use cm::sources::PINNED_CORE_VERSION;
    use cm::sources::artifact::{
        ArtifactError, GlobalDefaults, NodeDefinitionInput, SourceImportInput, create_source,
        update_source,
    };
    use cm::sources::capabilities::Transport;
    use cm::sources::import_confirmation::apply_confirmation;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);

    fn store(label: &str) -> (Store, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "cm-i03-omissions-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        (Store::initialize(&root).expect("synthetic store"), root)
    }

    fn node(name: &str, tls: TlsVerification) -> NodeDefinitionInput {
        NodeDefinitionInput::from_trusted_classifier(
            PINNED_CORE_VERSION,
            NodeProtocol::Shadowsocks,
            Transport::Tcp,
            json!({"name": name, "type": "ss", "server": "edge.example.invalid", "port": 443,
                   "cipher": "aes-128-gcm", "password": "x"}),
        )
        .unwrap()
        .with_tls_verification(tls)
    }

    fn input(body: &str, at: i64, nodes: Vec<NodeDefinitionInput>) -> SourceImportInput {
        SourceImportInput::local(body.as_bytes().to_vec(), at, nodes, GlobalDefaults::default())
            .unwrap()
    }

    fn groups_and_rules(lines: SkippedLineClass) -> ImportOmissions {
        ImportOmissions {
            section_names: vec!["proxy-groups".into(), "rules".into()],
            skipped_lines: vec![SkippedLines { class: lines, line_numbers: vec![3] }],
            tls_verification_disabled_count: 0,
        }
    }

    fn confirmed(input: SourceImportInput, omissions: ImportOmissions) -> SourceImportInput {
        let probe = input.with_omissions(omissions);
        let digest = import_omissions_digest(probe.omissions()).unwrap();
        let omissions = probe.omissions().clone();
        apply_confirmation(probe, omissions, &digest).unwrap()
    }

    /// The disabled-TLS count is derived from nodes: a caller passing 0 cannot skip confirmation.
    #[test]
    fn disabled_tls_node_cannot_hide_behind_zero_count() {
        let (store, root) = store("tls-count");
        let rev = store.read_snapshot().unwrap().revision;
        let forged = input("a", 1_000, vec![node("n1", TlsVerification::Disabled)])
            .with_omissions(ImportOmissions::default());
        assert_eq!(forged.omissions().tls_verification_disabled_count, 1);
        assert!(matches!(
            create_source(&store, rev, forged).unwrap_err(),
            ArtifactError::PendingConfirmationRequired
        ));
        // Even without with_omissions the count is synced at publication.
        let bare = input("b", 1_000, vec![node("n1", TlsVerification::Disabled)]);
        assert!(matches!(
            create_source(&store, rev, bare).unwrap_err(),
            ArtifactError::PendingConfirmationRequired
        ));
        let ok = confirmed(
            input("c", 1_000, vec![node("n1", TlsVerification::Disabled)]),
            ImportOmissions::default(),
        );
        create_source(&store, rev, ok).expect("confirmed insecure node publishes");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Unattended refresh within the confirmed bound publishes without a new confirmation,
    /// and carries the old bound forward.
    #[test]
    fn auto_refresh_within_confirmed_bound_publishes() {
        let (store, root) = store("auto-ok");
        let rev = store.read_snapshot().unwrap().revision;
        let first = confirmed(
            input("gen1", 1_000, vec![node("n1", TlsVerification::NotApplicable), node("n2", TlsVerification::NotApplicable)]),
            groups_and_rules(SkippedLineClass::UnsupportedScheme),
        );
        let receipt = create_source(&store, rev, first).unwrap();
        let refresh = input("gen2", 2_000, vec![node("n1", TlsVerification::NotApplicable), node("n3", TlsVerification::NotApplicable)])
            .with_omissions(groups_and_rules(SkippedLineClass::UnsupportedScheme))
            .as_auto_refresh();
        let next = update_source(
            &store, &receipt.source_id, receipt.graph_revision, receipt.source_generation, refresh, 2_000,
        )
        .expect("refresh within bound must publish");
        assert_eq!(next.source_generation, receipt.source_generation + 1);
        let graph = store.read_snapshot().unwrap();
        let provenance = graph.sources[&receipt.source_id].provenance.as_ref().unwrap();
        assert_eq!(provenance.accepted_omissions_bound, groups_and_rules(SkippedLineClass::UnsupportedScheme));
        let _ = std::fs::remove_dir_all(root);
    }

    /// A new omission class or a new insecure node in unattended refresh keeps the old generation.
    #[test]
    fn auto_refresh_outside_bound_is_blocked() {
        let (store, root) = store("auto-blocked");
        let rev = store.read_snapshot().unwrap().revision;
        let first = confirmed(
            input("gen1", 1_000, vec![node("n1", TlsVerification::NotApplicable)]),
            groups_and_rules(SkippedLineClass::UnsupportedScheme),
        );
        let receipt = create_source(&store, rev, first).unwrap();
        let wider = input("gen2", 2_000, vec![node("n1", TlsVerification::NotApplicable)])
            .with_omissions(groups_and_rules(SkippedLineClass::Malformed))
            .as_auto_refresh();
        assert!(matches!(
            update_source(&store, &receipt.source_id, receipt.graph_revision, receipt.source_generation, wider, 2_000)
                .unwrap_err(),
            ArtifactError::AutoUpdateBlocked
        ));
        let insecure = input("gen3", 2_000, vec![node("n1", TlsVerification::Disabled)])
            .with_omissions(groups_and_rules(SkippedLineClass::UnsupportedScheme))
            .as_auto_refresh();
        assert!(matches!(
            update_source(&store, &receipt.source_id, receipt.graph_revision, receipt.source_generation, insecure, 2_000)
                .unwrap_err(),
            ArtifactError::AutoUpdateBlocked
        ));
        assert_eq!(store.read_snapshot().unwrap().sources[&receipt.source_id].generation, 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn nodes_without_tls_are_not_reported_as_verified() {
        assert_eq!(TlsVerification::default(), TlsVerification::NotApplicable);
        assert_eq!(node("n", TlsVerification::default()).tls_verification(), TlsVerification::NotApplicable);
    }
}
