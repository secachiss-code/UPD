//! I03.T04.w: every node the parser accepts must also be accepted by the pinned core.
//!
//! Set `CM_TEST_MIHOMO=/path/to/mihomo` (v1.19.32). Without it the check prints SKIPPED and
//! must be recorded as SKIPPED in evidence, never as PASS. No network is used: `-t` only
//! parses the configuration.

use cm::sources::{ImportFormat, parse_native};
use std::path::PathBuf;
use std::process::Command;

#[test]
fn accepted_corpus_is_accepted_by_pinned_core() {
    let corpus = std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/i03_core_corpus.json")).unwrap();
    // Our side first: the corpus must be fully accepted with no omissions.
    let parsed = parse_native("1.19.32", ImportFormat::MihomoJson, &corpus).expect("corpus accepted by parser");
    assert!(parsed.omissions().is_empty());
    let Some(core) = std::env::var_os("CM_TEST_MIHOMO") else {
        eprintln!("I03.T04.w SKIPPED: CM_TEST_MIHOMO not set ({} nodes parsed)", parsed.node_count());
        return;
    };
    let dir = std::env::temp_dir().join(format!("cm-i03-core-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut config: serde_json::Value = serde_json::from_slice(&corpus).unwrap();
    config["mode"] = "rule".into();
    config["rules"] = serde_json::json!(["MATCH,DIRECT"]);
    let file = dir.join("config.yaml");
    std::fs::write(&file, serde_json::to_vec(&config).unwrap()).unwrap();
    let output = Command::new(core).arg("-t").arg("-d").arg(&dir).arg("-f").arg(&file).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        output.status.success(),
        "pinned core rejected a parser-accepted node:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
