//! L2 AB stand: `tests/ab_stand.py` with real controller and mihomo.

use std::process::Command;

const EDGES: [&str; 16] = [
    "H12", "D04", "F03", "T07", "T10", "T11", "T13", "T14", "R03", "X02", "Z01", "Z02", "Z03",
    "Z05", "Z06", "Z07",
];

#[test]
fn ab_l2_stand_checks() {
    let Some(mihomo) = std::env::var_os("CM_TEST_MIHOMO") else {
        eprintln!("AB L2 SKIPPED: CM_TEST_MIHOMO not set");
        return;
    };
    let output = Command::new("timeout")
        .args([
            "600",
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
            "mount -t tmpfs tmpfs /run && ip link set lo up && exec python3 tests/ab_stand.py",
        ])
        .env("CM_BIN", env!("CARGO_BIN_EXE_cm"))
        .env("CM_TEST_MIHOMO", mihomo)
        .env("CM_HELPER_ALLOW", "1")
        .output()
        .expect("unshare");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let ok_lines: Vec<&str> = text.lines().filter(|l| l.starts_with("ok ")).collect();
    let fail_lines: Vec<&str> = text.lines().filter(|l| l.starts_with("FAIL ")).collect();
    assert!(fail_lines.is_empty(), "{fail_lines:#?}\n{text}");
    assert!(output.status.success(), "{text}");
    assert!(
        ok_lines.len() >= 90,
        "only {} checks:\n{text}",
        ok_lines.len()
    );
    for edge in EDGES {
        assert!(
            ok_lines.iter().any(|line| line.contains(edge)),
            "no stand check for {edge}"
        );
    }
}
