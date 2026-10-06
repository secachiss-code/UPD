//! Shared synthetic profile fixture; never reads the host configuration.
use cm::profiles::*;
use sha2::{Digest, Sha256};

pub(crate) fn id(value: &str) -> Id {
    Id::new(value).unwrap()
}
pub(crate) fn digest(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
pub(crate) fn issue(graph: &mut GraphSnapshot, value: &str, kind: EntityIdKind) {
    graph.issued_ids.insert(id(value));
    assert!(graph.issued_id_kinds.insert(id(value), kind).is_none());
}

pub(crate) fn fixture() -> GraphSnapshot {
    let mut graph = GraphSnapshot::new();
    for suffix in ["a", "b"] {
        let source_id = format!("source-{suffix}");
        let node_id = format!("node-{suffix}-1");
        let credential_id = format!("credential-{suffix}");
        let profile_id = format!("profile-{suffix}");
        let material = format!("synthetic-private-{suffix}");
        let credential = CredentialMetadata {
            schema_version: SCHEMA_VERSION,
            credential_ref: id(&credential_id),
            owner_source_id: id(&source_id),
            digest_sha256: digest(material.as_bytes()),
            size_bytes: material.len() as u64,
        };
        graph
            .credentials
            .insert(id(&credential_id), credential.clone());
        graph.sources.insert(
            id(&source_id),
            Source {
                schema_version: SCHEMA_VERSION,
                id: id(&source_id),
                kind: SourceKind::Subscription,
                generation: 1,
                content_digest_sha256: digest(format!("input-{suffix}-1").as_bytes()),
                credential: Some(credential),
                provenance: None,
                current_node_ids: vec![id(&node_id)],
            },
        );
        graph.nodes.insert(
            id(&node_id),
            Node {
                schema_version: SCHEMA_VERSION,
                id: id(&node_id),
                source_id: id(&source_id),
                source_generation: 1,
                definition_digest_sha256: digest(format!("definition-{suffix}").as_bytes()),
                protocol: NodeProtocol::Vless,
                tls_verification: TlsVerification::NotApplicable,
                credential_refs: vec![id(&credential_id)],
            },
        );
        graph.connection_profiles.insert(
            id(&profile_id),
            ConnectionProfile {
                schema_version: SCHEMA_VERSION,
                id: id(&profile_id),
                source_id: id(&source_id),
                node_selection: NodeSelection::Pinned {
                    node: SourceNodeRef {
                        source_id: id(&source_id),
                        source_generation: 1,
                        node_id: id(&node_id),
                    },
                },
                kernel: KernelKind::Mihomo,
                dns: DnsPolicy {
                    ipv6: Ipv6Policy::Block,
                    fake_ip: FakeIpPolicy::Forbidden,
                },
            },
        );
        for (value, kind) in [
            (&source_id, EntityIdKind::Source),
            (&node_id, EntityIdKind::Node),
            (&credential_id, EntityIdKind::CredentialRef),
            (&profile_id, EntityIdKind::ConnectionProfile),
        ] {
            issue(&mut graph, value, kind);
        }
    }
    graph.environment_presets.insert(
        id("preset"),
        EnvironmentPreset {
            schema_version: SCHEMA_VERSION,
            id: id("preset"),
            timezone: "UTC".into(),
            locale: "en_US.UTF-8".into(),
            languages: vec!["en".into()],
        },
    );
    issue(&mut graph, "preset", EntityIdKind::EnvironmentPreset);
    graph.application_groups.insert(
        id("group"),
        ApplicationGroup {
            schema_version: SCHEMA_VERSION,
            id: id("group"),
            tunnel_instance_id: id("tunnel-group"),
            mutual_network_access: false,
            autostart: false,
        },
    );
    issue(&mut graph, "group", EntityIdKind::ApplicationGroup);
    for (number, grouped) in [(1, true), (2, true), (3, false)] {
        let app_id = format!("app-{number}");
        let data_id = format!("data-{number}");
        let browser_id = format!("browser-{number}");
        graph.data_profiles.insert(
            id(&data_id),
            DataProfile {
                schema_version: SCHEMA_VERSION,
                id: id(&data_id),
            },
        );
        graph.browser_identity_profiles.insert(
            id(&browser_id),
            BrowserIdentityProfile {
                schema_version: SCHEMA_VERSION,
                id: id(&browser_id),
                data_profile_id: id(&data_id),
                label: None,
            },
        );
        graph.applications.insert(
            id(&app_id),
            ApplicationDefinition {
                schema_version: SCHEMA_VERSION,
                id: id(&app_id),
                executable: format!("/usr/bin/example-{number}"),
                argv: vec![],
                cwd: None,
                data_profile_id: id(&data_id),
                environment: vec![],
                environment_preset_id: id("preset"),
                assignment: if grouped {
                    ApplicationAssignment::Group {
                        group_id: id("group"),
                    }
                } else {
                    ApplicationAssignment::OwnTunnel {
                        tunnel_instance_id: id("tunnel-own"),
                    }
                },
                autostart: false,
            },
        );
        issue(&mut graph, &data_id, EntityIdKind::DataProfile);
        issue(
            &mut graph,
            &browser_id,
            EntityIdKind::BrowserIdentityProfile,
        );
        issue(&mut graph, &app_id, EntityIdKind::Application);
        if grouped {
            let session_id = format!("session-{number}");
            let environment_id = format!("environment-{number}");
            graph.environment_profiles.insert(
                id(&environment_id),
                EnvironmentProfile {
                    schema_version: SCHEMA_VERSION,
                    id: id(&environment_id),
                    session_id: id(&session_id),
                    preset_id: id("preset"),
                    timezone: "UTC".into(),
                    locale: "en_US.UTF-8".into(),
                    languages: vec!["en".into()],
                },
            );
            graph.sessions.insert(
                id(&session_id),
                Session {
                    schema_version: SCHEMA_VERSION,
                    id: id(&session_id),
                    lifecycle: SessionLifecycle::Active,
                    application_id: id(&app_id),
                    data_profile_id: id(&data_id),
                    environment_profile_id: id(&environment_id),
                    tunnel_instance_id: id("tunnel-group"),
                    tunnel_generation: 1,
                    source_id: id("source-a"),
                    source_generation: 1,
                    node_id: id("node-a-1"),
                    created_at_unix_ms: 1000,
                    ended_at_unix_ms: None,
                },
            );
            issue(&mut graph, &session_id, EntityIdKind::Session);
            issue(
                &mut graph,
                &environment_id,
                EntityIdKind::EnvironmentProfile,
            );
        }
    }
    for (value, profile, owner) in [
        (
            "tunnel-group",
            "profile-a",
            TunnelOwner::Group {
                group_id: id("group"),
            },
        ),
        (
            "tunnel-own",
            "profile-b",
            TunnelOwner::Application {
                application_id: id("app-3"),
            },
        ),
    ] {
        graph.tunnel_instances.insert(
            id(value),
            TunnelInstance {
                schema_version: SCHEMA_VERSION,
                id: id(value),
                connection_profile_id: id(profile),
                generation: 1,
                owner,
                lifecycle: TunnelLifecycle::Running,
            },
        );
        issue(&mut graph, value, EntityIdKind::TunnelInstance);
    }
    graph.host_policy = Some(HostPolicy {
        schema_version: SCHEMA_VERSION,
        id: id("host"),
        mode: HostMode::Off,
        connection_profile_id: None,
        tunnel_instance_id: None,
        autostart: false,
    });
    issue(&mut graph, "host", EntityIdKind::HostPolicy);
    graph
}

pub(crate) fn source_update(old: &GraphSnapshot, same_definition: bool) -> GraphSnapshot {
    let mut new = old.clone();
    new.revision += 1;
    let mut node = old.nodes[&id("node-a-1")].clone();
    node.id = id("node-a-2");
    node.source_generation = 2;
    if !same_definition {
        node.definition_digest_sha256 = digest(b"different-definition");
    }
    new.nodes.insert(node.id.clone(), node);
    issue(&mut new, "node-a-2", EntityIdKind::Node);
    let source = new.sources.get_mut(&id("source-a")).unwrap();
    source.generation = 2;
    source.content_digest_sha256 = digest(b"input-a-2");
    source.current_node_ids = vec![id("node-a-2")];
    new
}

pub(crate) fn net_evidence(graph: &mut GraphSnapshot, value: VerificationValue, timestamp: i64) {
    for number in [1, 2] {
        let key = format!("net-{number}");
        if !graph.verifications.contains_key(&id(&key)) {
            issue(graph, &key, EntityIdKind::Verification);
        }
        graph.verifications.insert(
            id(&key),
            Verification {
                schema_version: SCHEMA_VERSION,
                id: id(&key),
                session_id: id(&format!("session-{number}")),
                tunnel_instance_id: id("tunnel-group"),
                tunnel_generation: 1,
                axis: VerificationAxis::Net,
                value,
                evidence_at_unix_ms: timestamp,
            },
        );
    }
}
