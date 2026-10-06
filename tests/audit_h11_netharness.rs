//! H.11 (D6): rootless network harness smoke. L2 level: two namespaces, veth, nftables.
//! Without user namespaces (some CI runners) the test prints SKIPPED and must be recorded
//! as SKIPPED in evidence, never as PASS.

use std::process::Command;

#[test]
fn two_namespace_harness_reaches_blocks_and_reopens() {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/netharness/smoke.sh");
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
        output.status.success() && stdout.contains("SMOKE_OK"),
        "harness failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
