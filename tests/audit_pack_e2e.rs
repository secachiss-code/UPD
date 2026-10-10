//! Z02 and the L2 parts of C07, C09, C16, K14, M03, X04: the real `cm controller serve`
//! and the pinned mihomo in a private user, network and mount namespace.
//!
//! The scenario is `tests/pack_stand.py`; the controller runs as uid 0 of the namespace and
//! the client as another uid. Without `CM_TEST_MIHOMO` the test prints SKIPPED.

use std::process::Command;

const EDGES: [&str; 8] = ["C07", "C09", "C15", "C16", "K14", "M03", "X04", "Z02"];

#[test]
fn z02_application_reaches_the_net_only_through_its_tunnel() {
    let Some(mihomo) = std::env::var_os("CM_TEST_MIHOMO") else {
        eprintln!("Z02 SKIPPED: CM_TEST_MIHOMO not set");
        return;
    };
    let output = Command::new("timeout")
        .args([
            "300",
            "unshare",
            "-U",
            "--map-root-user",
            "--map-auto",
            "-n",
            "-m",
            "--",
        ])
        .args([
            "sh",
            "-c",
            "mount -t tmpfs tmpfs /run && ip link set lo up && exec python3 tests/pack_stand.py",
        ])
        .env("CM_BIN", env!("CARGO_BIN_EXE_cm"))
        .env("CM_TEST_MIHOMO", mihomo)
        .output()
        .expect("unshare");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let passed: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("PASS "))
        .collect();
    let failed: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("FAIL "))
        .collect();
    assert!(failed.is_empty(), "{failed:#?}\n{text}");
    assert!(output.status.success(), "{text}");
    assert!(
        passed.len() >= 70,
        "only {} checks ran:\n{text}",
        passed.len()
    );
    for edge in EDGES {
        assert!(
            passed
                .iter()
                .any(|line| line.starts_with(&format!("PASS {edge} "))),
            "no check of {edge}"
        );
    }
    println!("{} stand checks passed", passed.len());
}
