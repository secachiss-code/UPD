//! I04.T01 lifecycle of the fake adapter, and I04.T02.a instance directories.
use cm::common::contract_fixtures::TempDirGuard;
use cm::core::{
    ApiState, CoreAdapter, CoreConfig, CoreError, CoreReadiness, CoreStatistics, FakeAdapter,
    FakeOp, InstanceId, InstanceRoot, LeaseRegistry, NetPrivileges, RemoteState, ResourceKind,
    RouteState, legacy_hardening_directives, render_core_unit,
};
use cm::profiles::NodeProtocol;
use cm::sources::{supported_import_formats, supported_transports, supported_uri_schemes};
use std::os::unix::fs::PermissionsExt;

const SECRET: &str = "core-secret-marker";

fn config() -> CoreConfig {
    CoreConfig::from_bytes(format!("proxies: [{{password: {SECRET}}}]")).unwrap()
}

#[test]
fn fake_lifecycle_keeps_readiness_axes_independent() {
    let mut core = FakeAdapter::new();
    assert_eq!(core.health().unwrap(), CoreReadiness::DOWN);
    let ready = core.start(&config()).unwrap();
    assert_eq!(ready.api, ApiState::ApiReady);
    assert_eq!(ready.route, RouteState::Down);
    assert_eq!(ready.remote, RemoteState::Unknown);
    assert_eq!(core.generation(), 1);

    core.set_route(RouteState::RouteReady).unwrap();
    assert_eq!(core.health().unwrap().remote, RemoteState::Unknown);
    core.set_remote(RemoteState::RemoteReachable).unwrap();
    let health = core.health().unwrap();
    assert_eq!(health.api, ApiState::ApiReady);
    assert_eq!(health.route, RouteState::RouteReady);
    assert_eq!(health.remote, RemoteState::RemoteReachable);

    let reloaded = core.reload(&config()).unwrap();
    assert_eq!(reloaded.route, RouteState::RouteReady);
    assert_eq!(core.generation(), 2);
    core.set_statistics(CoreStatistics {
        upload_bytes: 3,
        download_bytes: 4,
        connections: 1,
    });
    assert_eq!(core.statistics().unwrap().download_bytes, 4);

    core.fail_next(FakeOp::Reload, CoreError::Failed);
    let error = core.reload(&config()).unwrap_err();
    assert_eq!(error, CoreError::Failed);
    assert!(!error.to_string().contains(SECRET));
    assert!(!format!("{error:?}").contains(SECRET));
    assert!(!format!("{:?}", config()).contains(SECRET));
    assert_eq!(core.generation(), 2, "failed reload does not publish");

    core.stop().unwrap();
    assert_eq!(core.health().unwrap(), CoreReadiness::DOWN);
    assert_eq!(core.statistics().unwrap(), CoreStatistics::default());
    assert_eq!(core.reload(&config()).unwrap_err(), CoreError::NotRunning);
}

#[test]
fn failed_start_leaves_the_adapter_stopped() {
    let mut core = FakeAdapter::new();
    core.start(&config()).unwrap();
    core.stop().unwrap();
    core.fail_next(FakeOp::Start, CoreError::Timeout);
    assert_eq!(core.start(&config()).unwrap_err(), CoreError::Timeout);
    assert_eq!(core.health().unwrap(), CoreReadiness::DOWN);
    assert_eq!(core.start(&config()).unwrap().api, ApiState::ApiReady);
    assert_eq!(core.start(&config()).unwrap_err(), CoreError::Conflict);
}

#[test]
fn mihomo_descriptor_is_the_i03_tables() {
    let caps = cm::core::mihomo::descriptor();
    assert_eq!(caps.version, cm::sources::PINNED_CORE_VERSION);
    assert_eq!(caps.commit, cm::sources::PINNED_CORE_COMMIT);
    assert!(std::ptr::eq(
        cm::core::mihomo::import_formats(),
        supported_import_formats()
    ));
    assert!(std::ptr::eq(
        cm::core::mihomo::uri_schemes(),
        supported_uri_schemes()
    ));
    assert!(std::ptr::eq(
        cm::core::mihomo::transports(NodeProtocol::Vless).unwrap(),
        supported_transports(NodeProtocol::Vless).unwrap()
    ));
}

#[test]
fn two_instances_do_not_share_files_and_removal_is_local() {
    let root_dir = TempDirGuard::new("cm-i04-instance").unwrap();
    let root = InstanceRoot::new(root_dir.path());
    let left = InstanceId::new("host-a").unwrap();
    let right = InstanceId::new("host-b").unwrap();
    assert!(InstanceId::new("../etc").is_err());
    assert!(InstanceId::new("Host").is_err());
    let left_dirs = root.create(&left).unwrap();
    let right_dirs = root.create(&right).unwrap();
    std::fs::write(left_dirs.config.join("marker"), b"left").unwrap();
    assert!(!right_dirs.config.join("marker").exists());
    for dir in [
        &left_dirs.root,
        &left_dirs.config,
        &right_dirs.cache,
        &right_dirs.run,
    ] {
        let mode = std::fs::metadata(dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "{}", dir.display());
    }
    root.remove(&left).unwrap();
    assert!(!left_dirs.root.exists());
    assert!(right_dirs.config.is_dir());
    assert!(!right_dirs.config.join("marker").exists());
}

#[test]
fn concurrent_leases_are_unique_and_crash_reclaim_frees_the_absent_holder() {
    let dir = TempDirGuard::new("cm-i04-leases").unwrap();
    let registry = LeaseRegistry::create(dir.path()).unwrap();
    let mode = std::fs::metadata(dir.path().join("leases.json"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    let mut threads = Vec::new();
    for index in 0..8 {
        let registry = registry.clone();
        threads.push(std::thread::spawn(move || {
            let holder = format!("worker-{index}");
            let mut values = Vec::new();
            for _ in 0..20 {
                values.push(
                    registry
                        .allocate(&holder, ResourceKind::Port)
                        .unwrap()
                        .value,
                );
            }
            values
        }));
    }
    let mut values = Vec::new();
    for thread in threads {
        values.extend(thread.join().unwrap());
    }
    values.sort();
    let mut unique = values.clone();
    unique.dedup();
    assert_eq!(values.len(), 160);
    assert_eq!(unique.len(), 160);

    registry.allocate("kept", ResourceKind::TunName).unwrap();
    let dropped = registry.allocate("gone", ResourceKind::Port).unwrap();
    assert_eq!(registry.reclaim_absent(&["kept"]).unwrap(), 161);
    let reused = registry.allocate("kept", ResourceKind::Port).unwrap();
    assert_eq!(reused.value, "20000");
    assert_ne!(reused.value, dropped.value);
    assert!(
        registry
            .release("nope", ResourceKind::Port, &reused.value)
            .is_err()
    );
    registry
        .release("kept", ResourceKind::Port, &reused.value)
        .unwrap();
}

#[test]
fn core_unit_grants_capabilities_only_when_requested() {
    let id = InstanceId::new("host-a").unwrap();
    let proxy = render_core_unit(&id, NetPrivileges::default());
    assert!(proxy.contains("CapabilityBoundingSet=\n"));
    assert!(!proxy.contains("AmbientCapabilities="));
    assert!(!proxy.contains("CAP_NET_"));
    let tun = render_core_unit(
        &id,
        NetPrivileges {
            admin: true,
            ..NetPrivileges::default()
        },
    );
    assert!(tun.contains("CapabilityBoundingSet=CAP_NET_ADMIN\n"));
    assert!(tun.contains("AmbientCapabilities=CAP_NET_ADMIN\n"));
    assert!(!tun.contains("CAP_NET_RAW"));
    assert!(!tun.contains("CAP_NET_BIND_SERVICE"));
    assert!(tun.contains("ReadWritePaths=-/var/lib/cm/instances/host-a\n"));
    assert!(!tun.contains("ReadWritePaths=-/var/lib/cm/vpn"));
    assert!(tun.contains("RuntimeDirectory=cm-core-host-a\n"));
    assert!(tun.contains("NoNewPrivileges=yes\n"));
    for directive in legacy_hardening_directives() {
        assert!(tun.contains(directive), "{directive}");
    }
}

#[test]
fn worker_generator_rejects_host_routing_and_foreign_listeners() {
    let leased = 20_000u16;
    let forbidden = [
        r#"{"tun":{"auto-route":true}}"#,
        r#"{"auto-redirect":true}"#,
        r#"{"dns":{"dns-hijack":["any"]}}"#,
        r#"{"listeners":[{"listen":"0.0.0.0","port":20000}]}"#,
        r#"{"listen":"127.0.0.1","port":20001}"#,
    ];
    for raw in forbidden {
        let value: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert!(cm::core::generate_worker(&value, leased).is_err(), "{raw}");
    }
    let allowed: serde_json::Value =
        serde_json::from_str(r#"{"listen":"127.0.0.1","port":20000}"#).unwrap();
    let config = cm::core::generate_worker(&allowed, leased).unwrap();
    assert!(config.contains("127.0.0.1"));
    assert!(config.contains("20000"));
    assert!(!config.contains("auto-route"));
    assert!(!config.contains("auto-redirect"));
    assert!(!config.contains("dns-hijack"));
    assert!(!config.contains("0.0.0.0"));
}

// ---- Coordinator review 2026-10-06 ----

#[test]
fn worker_refuses_every_host_listener_key() {
    use cm::core::{WorkerError, generate_worker};
    use serde_json::json;
    for requested in [
        json!({"mixed-port": 7890}),
        json!({"socks-port": 7891}),
        json!({"external-controller": "0.0.0.0:9090"}),
        json!({"allow-lan": true}),
        json!({"tun": {"enable": true}}),
        json!({"dns": {"listen": "0.0.0.0:53"}}),
        json!({"listeners": [{"listen": "127.0.0.1", "port": "7890"}]}),
        json!({"listeners": [{"listen": "::", "port": 20000}]}),
    ] {
        let error = generate_worker(&requested, 20000).unwrap_err();
        assert!(matches!(error, WorkerError::ExternalListener | WorkerError::UnleasedListener), "{requested}");
    }
    generate_worker(&json!({"dns": {"nameserver": ["1.1.1.1"]}}), 20000).expect("resolver settings are not a listener");
}

#[test]
fn core_unit_runs_binary_outside_writable_paths() {
    use cm::core::{InstanceId, NetPrivileges, render_core_unit};
    let id = InstanceId::new("app-one").unwrap();
    let unit = render_core_unit(&id, NetPrivileges { admin: true, raw: false, bind_service: false });
    let exec = unit.lines().find(|line| line.starts_with("ExecStart=")).unwrap();
    let rw = unit.lines().find(|line| line.starts_with("ReadWritePaths=")).unwrap();
    let binary = exec.trim_start_matches("ExecStart=").split_whitespace().next().unwrap();
    assert!(!binary.starts_with(rw.trim_start_matches("ReadWritePaths=-")), "{exec} / {rw}");
    for directive in ["ProtectKernelTunables=yes", "RestrictNamespaces=yes", "DeviceAllow=/dev/net/tun rw", "SystemCallArchitectures=native"] {
        assert!(unit.contains(directive), "{directive}");
    }
    let plain = render_core_unit(&id, NetPrivileges::default());
    assert!(plain.contains("PrivateDevices=yes") && plain.contains("CapabilityBoundingSet=\n"));
}

#[test]
fn lease_lock_gives_up_instead_of_hanging() {
    use cm::core::{LeaseError, LeaseRegistry, ResourceKind};
    use std::os::fd::AsRawFd;
    let dir = std::env::temp_dir().join(format!("cm-i04-lease-lock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let registry = LeaseRegistry::create(&dir).unwrap();
    let holder = std::fs::File::open(dir.join("leases.lock")).unwrap();
    assert_eq!(unsafe { libc::flock(holder.as_raw_fd(), libc::LOCK_EX) }, 0);
    let started = std::time::Instant::now();
    assert_eq!(registry.allocate("app-one", ResourceKind::Port).unwrap_err(), LeaseError::Busy);
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    drop(holder);
    registry.allocate("app-one", ResourceKind::Port).unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
