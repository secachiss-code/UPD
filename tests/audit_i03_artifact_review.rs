//! Independent Sol regressions; execution is deferred until installation.

use cm::profiles::{NodeProtocol, Store};
use cm::sources::{
    ArtifactError, GlobalDefaults, NodeDefinitionInput, SourceImportInput, Transport,
    create_source, read_node_definition, update_source,
};
use serde_json::json;
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static ROOT_COUNTER: AtomicU64 = AtomicU64::new(1);

struct FixtureRoot(PathBuf);

impl FixtureRoot {
    fn new() -> Self {
        for _ in 0..16 {
            let parent = std::env::temp_dir().join(format!(
                "cm-i03-sol-{}-{}",
                std::process::id(),
                ROOT_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::DirBuilder::new().mode(0o700).create(&parent) {
                Ok(()) => return Self(parent.join("store")),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("fixture creation failed: {:?}", error.kind()),
            }
        }
        panic!("fixture directory collision limit reached");
    }
}

impl Drop for FixtureRoot {
    fn drop(&mut self) {
        // Only remove the private parent this fixture created exclusively.
        let _ = fs::remove_dir_all(self.0.parent().unwrap());
    }
}

fn input(body: &[u8], protocol: NodeProtocol, transport: Transport) -> SourceImportInput {
    // This trusted marker isolates the typed identity fields. Full core syntax is
    // the later parser's responsibility; no core is executed by this fixture.
    let definition = NodeDefinitionInput::from_trusted_classifier(
        "1.19.32",
        protocol,
        transport,
        json!({"name":"synthetic-typed-marker"}),
    )
    .unwrap();
    SourceImportInput::local(
        body.to_vec(),
        1_000,
        vec![definition],
        GlobalDefaults::default(),
    )
    .unwrap()
}

#[test]
fn typed_protocol_and_transport_are_part_of_immutable_definition_identity() {
    let root = FixtureRoot::new();
    let store = Store::initialize(&root.0).unwrap();
    let first = create_source(
        &store,
        0,
        input(b"one", NodeProtocol::Vless, Transport::Tcp),
    )
    .unwrap();
    let original = store.read_snapshot().unwrap();
    let refused = update_source(
        &store,
        &first.source_id,
        first.graph_revision,
        1,
        input(b"one", NodeProtocol::Vmess, Transport::Tcp),
        1_000,
    );
    assert!(matches!(
        refused,
        Err(ArtifactError::SameBodyPayloadChanged)
    ));
    assert_eq!(store.read_snapshot().unwrap(), original);

    let second = update_source(
        &store,
        &first.source_id,
        first.graph_revision,
        1,
        input(b"two", NodeProtocol::Vmess, Transport::Tcp),
        1_000,
    )
    .unwrap();
    let third = update_source(
        &store,
        &first.source_id,
        second.graph_revision,
        2,
        input(b"three", NodeProtocol::Vmess, Transport::Ws),
        1_000,
    )
    .unwrap();
    assert_eq!(third.source_generation, 3);
    let nodes = [&first, &second, &third]
        .map(|receipt| read_node_definition(&store, &receipt.node_ids[0]).unwrap());
    assert_ne!(
        nodes[0].definition_digest_sha256(),
        nodes[1].definition_digest_sha256()
    );
    assert_ne!(
        nodes[1].definition_digest_sha256(),
        nodes[2].definition_digest_sha256()
    );
    assert_eq!(nodes[0].protocol(), NodeProtocol::Vless);
    assert_eq!(nodes[2].transport(), Transport::Ws);
    assert_ne!(first.node_ids, second.node_ids);
    assert_ne!(second.node_ids, third.node_ids);
}

#[test]
fn aggregate_payload_overflow_preserves_the_published_source_and_private_files() {
    let root = FixtureRoot::new();
    let store = Store::initialize(&root.0).unwrap();
    create_source(
        &store,
        0,
        input(b"working", NodeProtocol::Vless, Transport::Tcp),
    )
    .unwrap();
    let before = store.read_snapshot().unwrap();
    let before_state = fs::read(root.0.join("state.json")).unwrap();
    let before_blob_count = fs::read_dir(root.0.join("credentials")).unwrap().count();
    let definitions =
        (0..2)
            .map(|index| {
                NodeDefinitionInput::from_trusted_classifier(
            "1.19.32", NodeProtocol::Vless, Transport::Tcp,
            json!({"name":format!("large-{index}"), "material":"x".repeat(17 * 1024 * 1024)}),
        ).unwrap()
            })
            .collect();
    let rejected = SourceImportInput::local(
        b"oversized normalized payload".to_vec(),
        2_000,
        definitions,
        GlobalDefaults::default(),
    );
    assert!(matches!(rejected, Err(ArtifactError::ArtifactTooLarge)));
    assert_eq!(store.read_snapshot().unwrap(), before);
    assert_eq!(fs::read(root.0.join("state.json")).unwrap(), before_state);
    assert_eq!(
        fs::read_dir(root.0.join("credentials")).unwrap().count(),
        before_blob_count
    );
}
