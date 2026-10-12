//! T03, T04: tunnel resolution and application assignment.

#[path = "ab_support/mod.rs"]
mod ab_support;

use cm::app::assign::{self, AssignError};
use cm::profiles::*;

use ab_support::with_cm_conf;

fn id(value: &str) -> Id {
    Id::new(value).unwrap()
}

fn snapshot_tunnels() -> GraphSnapshot {
    let mut graph = GraphSnapshot::new();
    let source = id("s1");
    let profile = id("p1");
    graph.sources.insert(
        source.clone(),
        Source {
            schema_version: SCHEMA_VERSION,
            id: source.clone(),
            kind: SourceKind::Subscription,
            generation: 1,
            content_digest_sha256: "a".repeat(64),
            credential: None,
            provenance: None,
            current_node_ids: vec![id("n1")],
        },
    );
    graph.connection_profiles.insert(
        profile.clone(),
        ConnectionProfile {
            schema_version: SCHEMA_VERSION,
            id: profile.clone(),
            source_id: source.clone(),
            node_selection: NodeSelection::Pinned {
                node: SourceNodeRef {
                    source_id: source.clone(),
                    source_generation: 1,
                    node_id: id("n1"),
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
        id("t1"),
        TunnelInstance {
            schema_version: SCHEMA_VERSION,
            id: id("t1"),
            generation: 1,
            connection_profile_id: profile.clone(),
            owner: TunnelOwner::Application {
                application_id: id("a1"),
            },
            lifecycle: TunnelLifecycle::Stopped,
        },
    );
    graph.tunnel_instances.insert(
        id("t2"),
        TunnelInstance {
            schema_version: SCHEMA_VERSION,
            id: id("t2"),
            generation: 1,
            connection_profile_id: profile,
            owner: TunnelOwner::Application {
                application_id: id("a1"),
            },
            lifecycle: TunnelLifecycle::Stopped,
        },
    );
    graph.applications.insert(
        id("a1"),
        ApplicationDefinition {
            schema_version: SCHEMA_VERSION,
            id: id("a1"),
            executable: "/bin/sh".into(),
            argv: vec![],
            cwd: None,
            data_profile_id: id("d1"),
            environment: vec![],
            environment_preset_id: id("preset"),
            assignment: ApplicationAssignment::OwnTunnel {
                tunnel_instance_id: id("t1"),
            },
            autostart: false,
        },
    );
    graph.applications.insert(
        id("a2"),
        ApplicationDefinition {
            schema_version: SCHEMA_VERSION,
            id: id("a2"),
            executable: "/bin/sh".into(),
            argv: vec![],
            cwd: None,
            data_profile_id: id("d1"),
            environment: vec![],
            environment_preset_id: id("preset"),
            assignment: ApplicationAssignment::OwnTunnel {
                tunnel_instance_id: id("t1"),
            },
            autostart: false,
        },
    );
    graph.application_groups.insert(
        id("g1"),
        ApplicationGroup {
            schema_version: SCHEMA_VERSION,
            id: id("g1"),
            tunnel_instance_id: id("t1"),
            mutual_network_access: false,
            autostart: false,
        },
    );
    graph
}

#[test]
fn t03_t04_assign() {
    with_cm_conf(|| {
        let graph = snapshot_tunnels();
        let t = assign::tunnel(&graph, &id("t1")).unwrap();
        assert_eq!(t.instance, "t1");
        assert_eq!(t.source_id, id("s1"));
        assert_eq!(t.connection_profile_id, id("p1"));
        assert_eq!(
            assign::tunnel(&graph, &id("missing")),
            Err(AssignError::UnknownTunnel)
        );
        assert_eq!(
            assign::application(&graph, &id("a1")).unwrap().shared,
            false
        );
        assert_eq!(
            assign::application(&graph, &id("a2")).unwrap_err(),
            AssignError::OwnerMismatch
        );
        let mut g_bad = snapshot_tunnels();
        g_bad.tunnel_instances.insert(
            id("Work_VPN"),
            TunnelInstance {
                schema_version: SCHEMA_VERSION,
                id: id("Work_VPN"),
                generation: 1,
                connection_profile_id: id("p1"),
                owner: TunnelOwner::Application {
                    application_id: id("a1"),
                },
                lifecycle: TunnelLifecycle::Stopped,
            },
        );
        assert_eq!(
            assign::tunnel(&g_bad, &id("Work_VPN")),
            Err(AssignError::BadInstanceName)
        );
        let mut g_host = snapshot_tunnels();
        g_host.tunnel_instances.insert(
            id("host"),
            TunnelInstance {
                schema_version: SCHEMA_VERSION,
                id: id("host"),
                generation: 1,
                connection_profile_id: id("p1"),
                owner: TunnelOwner::Application {
                    application_id: id("a1"),
                },
                lifecycle: TunnelLifecycle::Stopped,
            },
        );
        assert_eq!(
            assign::tunnel(&g_host, &id("host")),
            Err(AssignError::BadInstanceName)
        );
        let long = "a".repeat(33);
        let mut g_long = snapshot_tunnels();
        g_long.tunnel_instances.insert(
            id(&long),
            TunnelInstance {
                schema_version: SCHEMA_VERSION,
                id: id(&long),
                generation: 1,
                connection_profile_id: id("p1"),
                owner: TunnelOwner::Application {
                    application_id: id("a1"),
                },
                lifecycle: TunnelLifecycle::Stopped,
            },
        );
        assert_eq!(
            assign::tunnel(&g_long, &id(&long)),
            Err(AssignError::BadInstanceName)
        );
        let mut g2 = snapshot_tunnels();
        g2.application_groups.insert(
            id("g1"),
            ApplicationGroup {
                schema_version: SCHEMA_VERSION,
                id: id("g1"),
                tunnel_instance_id: id("t1"),
                mutual_network_access: false,
                autostart: false,
            },
        );
        g2.tunnel_instances.get_mut(&id("t1")).unwrap().owner =
            TunnelOwner::Group { group_id: id("g1") };
        g2.applications.get_mut(&id("a2")).unwrap().assignment =
            ApplicationAssignment::Group { group_id: id("g1") };
        assert!(assign::application(&g2, &id("a2")).unwrap().shared);
        let mut g3 = snapshot_tunnels();
        g3.application_groups
            .get_mut(&id("g1"))
            .unwrap()
            .tunnel_instance_id = id("t2");
        g3.applications.get_mut(&id("a2")).unwrap().assignment =
            ApplicationAssignment::Group { group_id: id("g1") };
        assert_eq!(
            assign::application(&g3, &id("a2")).unwrap_err(),
            AssignError::OwnerMismatch
        );
        let mut g4 = snapshot_tunnels();
        g4.tunnel_instances.get_mut(&id("t1")).unwrap().owner = TunnelOwner::Host;
        assert_eq!(
            assign::application(&g4, &id("a1")).unwrap_err(),
            AssignError::HostTunnel
        );
    });
}
