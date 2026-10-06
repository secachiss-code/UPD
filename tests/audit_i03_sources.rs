//! I03.T04.v: independent sources for profiles, built through the import path.

use cm::profiles::*;
use cm::sources::manual::{manual_node, manual_source_input, parse_add_server_args};
use cm::sources::{create_source, update_source};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);

fn id(text: &str) -> Id {
    Id::new(text).unwrap()
}

fn input(server: &str, at: i64) -> cm::sources::SourceImportInput {
    let args: Vec<String> = format!("--name n --protocol trojan --server {server} --port 443 --secret-stdin")
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let args = parse_add_server_args(&args).unwrap();
    manual_source_input("1.19.32", manual_node(&args, "synthetic-secret").unwrap(), at).unwrap()
}

fn add_profile(store: &Store, profile: &str, source: &Id, node: &Id, generation: u64) -> GraphSnapshot {
    let mut graph = store.read_snapshot().unwrap();
    let revision = graph.revision;
    let profile_id = id(profile);
    graph.issued_ids.insert(profile_id.clone());
    graph.issued_id_kinds.insert(profile_id.clone(), EntityIdKind::ConnectionProfile);
    graph.connection_profiles.insert(
        profile_id.clone(),
        ConnectionProfile {
            schema_version: SCHEMA_VERSION,
            id: profile_id,
            source_id: source.clone(),
            node_selection: NodeSelection::Pinned {
                node: SourceNodeRef { source_id: source.clone(), source_generation: generation, node_id: node.clone() },
            },
            kernel: KernelKind::Mihomo,
            dns: DnsPolicy { ipv6: Ipv6Policy::Block, fake_ip: FakeIpPolicy::Forbidden },
        },
    );
    store.commit(revision, graph, Vec::new()).unwrap()
}

#[test]
fn two_sources_are_independent_and_a_used_source_is_not_removed() {
    let root = std::env::temp_dir().join(format!("cm-i03-sources-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let store = Store::initialize(&root).unwrap();
    let a = create_source(&store, store.read_snapshot().unwrap().revision, input("a.example.invalid", 1_000)).unwrap();
    let b = create_source(&store, store.read_snapshot().unwrap().revision, input("b.example.invalid", 1_000)).unwrap();
    add_profile(&store, "profile-a", &a.source_id, &a.node_ids[0], a.source_generation);
    let before = add_profile(&store, "profile-b", &b.source_id, &b.node_ids[0], b.source_generation);

    // Refreshing A leaves B, its nodes and its profile untouched.
    let refreshed = update_source(&store, &a.source_id, before.revision, a.source_generation, input("a2.example.invalid", 2_000), 2_000).unwrap();
    let after = store.read_snapshot().unwrap();
    assert_eq!(refreshed.source_generation, a.source_generation + 1);
    assert_eq!(after.sources[&b.source_id], before.sources[&b.source_id]);
    assert_eq!(after.connection_profiles[&id("profile-b")], before.connection_profiles[&id("profile-b")]);
    assert!(after.nodes.contains_key(&b.node_ids[0]));

    // B is used by profile-b: removal is refused until the profile is detached.
    let plan = id("remove-b");
    assert!(store.begin_removal(after.revision, plan.clone(), b.source_id.clone()).is_err());
    let mut detached = store.read_snapshot().unwrap();
    let revision = detached.revision;
    detached.connection_profiles.remove(&id("profile-b"));
    let detached = store.commit(revision, detached, Vec::new()).unwrap();
    let done = store.remove_source(detached.revision, plan, b.source_id.clone()).unwrap();
    assert!(!done.sources.contains_key(&b.source_id));
    assert!(done.sources.contains_key(&a.source_id));
    assert!(done.connection_profiles.contains_key(&id("profile-a")));
    let _ = std::fs::remove_dir_all(root);
}
