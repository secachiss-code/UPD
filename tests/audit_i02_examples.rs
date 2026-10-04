//! Schema-1 JSON examples are synthetic and are validated through the public model API.

use cm::profiles::*;

fn example(name: &str) -> GraphSnapshot {
    let text = match name {
        "two_sources_three_apps.json" => {
            include_str!(
                "../docs/design/cm-network-manager/i02-examples/two_sources_three_apps.json"
            )
        }
        "host_and_group.json" => {
            include_str!("../docs/design/cm-network-manager/i02-examples/host_and_group.json")
        }
        "source_generation2_net_blocked.json" => include_str!(
            "../docs/design/cm-network-manager/i02-examples/source_generation2_net_blocked.json"
        ),
        _ => unreachable!("only checked-in example names are used"),
    };
    serde_json::from_str(text).unwrap()
}

#[test]
fn complete_baseline_and_generation_transition_examples_validate() {
    let baseline = example("two_sources_three_apps.json");
    let changed = example("source_generation2_net_blocked.json");
    baseline.validate().unwrap();
    changed.validate().unwrap();
    GraphSnapshot::validate_transition(&baseline, &changed).unwrap();

    assert_eq!(baseline.sources.len(), 2);
    assert_eq!(baseline.applications.len(), 3);
    assert_eq!(
        baseline.application_groups[&Id::new("group").unwrap()]
            .application_ids(&baseline.applications)
            .len(),
        2
    );
    assert!(changed.nodes.contains_key(&Id::new("node-a-1").unwrap()));
    assert!(changed.nodes.contains_key(&Id::new("node-a-2").unwrap()));
    assert_eq!(changed.sessions, baseline.sessions);
    for session_id in [Id::new("session-1").unwrap(), Id::new("session-2").unwrap()] {
        let status = PublicSessionStatus::from_graph(&changed, &session_id).unwrap();
        assert_eq!(status.node_id, Id::new("node-a-1").unwrap());
        assert_eq!(status.verification.net, VerificationValue::Blocked);
        assert_eq!(status.verification.region, VerificationValue::Unknown);
        assert_eq!(status.verification.state, VerificationValue::Unknown);
        assert_eq!(status.verification.app, VerificationValue::Unknown);
    }
}

#[test]
fn host_tunnel_uses_an_independent_source_from_the_application_group() {
    let graph = example("host_and_group.json");
    graph.validate().unwrap();
    let host = graph.host_policy.as_ref().unwrap();
    assert_eq!(host.mode, HostMode::Tunnel);
    let host_tunnel = &graph.tunnel_instances[host.tunnel_instance_id.as_ref().unwrap()];
    assert_eq!(host_tunnel.owner, TunnelOwner::Host);
    let host_profile = &graph.connection_profiles[&host_tunnel.connection_profile_id];
    assert_eq!(host_profile.source_id, Id::new("source-b").unwrap());
    let group_tunnel = &graph.tunnel_instances
        [&graph.application_groups[&Id::new("group").unwrap()].tunnel_instance_id];
    let group_profile = &graph.connection_profiles[&group_tunnel.connection_profile_id];
    assert_eq!(group_profile.source_id, Id::new("source-a").unwrap());
    assert_eq!(
        group_profile.source_id,
        graph.sessions[&Id::new("session-1").unwrap()].source_id
    );
}

#[test]
fn examples_contain_credential_metadata_but_no_synthetic_blob_payloads() {
    for text in [
        include_str!("../docs/design/cm-network-manager/i02-examples/two_sources_three_apps.json"),
        include_str!("../docs/design/cm-network-manager/i02-examples/host_and_group.json"),
        include_str!(
            "../docs/design/cm-network-manager/i02-examples/source_generation2_net_blocked.json"
        ),
    ] {
        assert!(text.contains("digest_sha256"));
        assert!(text.contains("size_bytes"));
        assert!(!text.contains("synthetic-private-a"));
        assert!(!text.contains("synthetic-private-b"));
    }
}
