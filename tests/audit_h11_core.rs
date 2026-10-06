//! H.11 core half: worker mihomo, remote mihomo inbound, nft counter.
//!
//! Without `CM_TEST_MIHOMO` or user namespaces the test prints SKIPPED and returns.
//! SKIPPED is not `CORE_OK`.

use cm::common::contract_fixtures::TempDirGuard;
use cm::core::mihomo::generate_config;
use cm::profiles::{DnsPolicy, FakeIpPolicy, Ipv6Policy};
use serde_json::json;
use std::os::unix::fs::OpenOptionsExt;
use std::process::Command;

const REMOTE_MIXED_PORT: u16 = 17890;
const WORKER_PORT: u16 = 18080;

#[test]
fn core_harness_reaches_remote_through_generated_worker() {
    let Some(mihomo) = std::env::var_os("CM_TEST_MIHOMO") else {
        eprintln!("H.11 SKIPPED: CM_TEST_MIHOMO not set");
        return;
    };
    let probe = Command::new("unshare").args(["-rn", "true"]).output();
    if !probe.as_ref().is_ok_and(|output| output.status.success()) {
        eprintln!("H.11 SKIPPED: unprivileged user/network namespaces unavailable");
        return;
    }
    let sandbox = TempDirGuard::new("cm-h11-core").unwrap();
    let proxy = json!({
        "name": "h11-marker",
        "type": "http",
        "server": "10.98.2.2",
        "port": REMOTE_MIXED_PORT
    });
    let dns = DnsPolicy {
        ipv6: Ipv6Policy::Block,
        fake_ip: FakeIpPolicy::Forbidden,
    };
    let bytes = generate_config(
        std::slice::from_ref(&proxy),
        &dns,
        WORKER_PORT,
        &["MATCH,h11-marker"],
        false,
    )
    .unwrap();
    let config = sandbox.path().join("worker.json");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&config)
        .unwrap();
    std::io::Write::write_all(&mut file, &bytes).unwrap();
    drop(file);
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/netharness/core.sh");
    let output = Command::new("timeout")
        .args(["140", "unshare", "-rn", "sh", script])
        .env("CM_TEST_MIHOMO", &mihomo)
        .env("CM_TEST_WORKER_CONFIG", &config)
        .output()
        .expect("run core harness");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("CORE_OK"),
        "core harness failed: status {} stdout_has_core_ok {}",
        output.status,
        stdout.contains("CORE_OK")
    );
}
