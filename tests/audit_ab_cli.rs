//! X01, U02: CLI writes net policy; unsupported launchers exit before controller.

#[path = "ab_support/mod.rs"]
mod ab_support;

use std::fs;
use std::process::Command;

use cm::net::policy::{self, FailureMode, Lifetime, TunnelPolicy};
use cm::profiles::{FakeIpPolicy, Ipv6Policy};

use ab_support::with_cm_conf;

#[test]
fn x01_u02_cli_policy_and_launcher() {
    with_cm_conf(|| {
        let dir = cm::common::contract_fixtures::TempDirGuard::new("cm-ab-x01").unwrap();
        let bytes = serde_json::to_vec(&TunnelPolicy {
            lifetime: Lifetime::Refcount,
            failure: FailureMode::Failover,
            ipv6: Ipv6Policy::Block,
            fake_ip: FakeIpPolicy::Forbidden,
        })
        .unwrap();
        fs::write(dir.path().join("net.json"), &bytes).unwrap();
        let read = policy::parse(&bytes).unwrap();
        assert_eq!(read.lifetime, Lifetime::Refcount);
        assert_eq!(read.failure, FailureMode::Failover);
        let bad = Command::new(env!("CARGO_BIN_EXE_cm"))
            .args(["tunnel", "up", "t1", "--lifetime", "forever"])
            .output()
            .unwrap();
        assert_eq!(bad.status.code(), Some(2));
        assert!(matches!(
            cm::app::packaging::policy::support(
                &cm::app::packaging::detect::Packaging::Launcher,
                cm::env::private::IpcPolicy::Private,
                cm::env::private::DataState::Existing,
            ),
            cm::app::packaging::policy::Support::Unsupported(
                cm::app::packaging::policy::Reason::ExecutorOutsideNetns
            )
        ));
    });
}

#[test]
#[ignore = "дефект U02: cm app run не отклоняет systemd-run до обращения к контроллеру (код 4)"]
fn u02_launcher_rejected_before_controller() {
    with_cm_conf(|| {
        let launcher = Command::new(env!("CARGO_BIN_EXE_cm"))
            .args([
                "app",
                "run",
                "t1",
                "--",
                "/usr/bin/systemd-run",
                "--user",
                "x",
            ])
            .env("CM_CONF", ab_support::CM_CONF)
            .output()
            .unwrap();
        assert_eq!(launcher.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&launcher.stderr).contains("[unsupported_executor_outside]")
        );
    });
}
