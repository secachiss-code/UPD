//! H.11 (D6): rootless network harness. L2: namespaces, veth, nftables.
//! Without user namespaces (some CI runners) the test prints SKIPPED and must be recorded
//! as SKIPPED in evidence, never as PASS.

use std::process::Command;

fn unshare_script(script: &str, marker: &str) {
    let probe = Command::new("unshare").args(["-rn", "true"]).output();
    if !probe.as_ref().is_ok_and(|output| output.status.success()) {
        eprintln!("H.11 SKIPPED: unprivileged user/network namespaces unavailable");
        return;
    }
    let output = Command::new("timeout")
        .args(["60", "unshare", "-rn", "sh", script])
        .output()
        .expect("run harness");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains(marker),
        "harness failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn two_namespace_harness_reaches_blocks_and_reopens() {
    unshare_script(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/netharness/smoke.sh"),
        "SMOKE_OK",
    );
}

#[test]
fn client_reaches_echo_through_worker_and_capture_sees_it() {
    unshare_script(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/netharness/worker.sh"),
        "WORKER_OK",
    );
}
