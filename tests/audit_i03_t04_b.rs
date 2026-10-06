use cm::sources::ParserError;
use cm::sources::parser::{
    audit_fixture_opts_digest, audit_parse_strict_json, audit_validate_fixture_opts,
};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/i03_t04_b")
}

fn load(name: &str) -> Vec<u8> {
    fs::read(fixture_dir().join(name)).expect("fixture bytes")
}

fn assert_reject(bytes: &[u8], expected: ParserError) {
    let err = audit_validate_fixture_opts(bytes).unwrap_err();
    assert_eq!(err, expected);
    let text = format!("{err}");
    assert!(!text.contains("CMFIXB"));
    assert!(!text.contains("not-a-key"));
    assert!(!text.contains("token-name"));
}

#[test]
fn strict_json_rejects_duplicate_literal_keys() {
    let bytes = load("duplicate-key-literal.json");
    assert_eq!(
        audit_parse_strict_json(&bytes).unwrap_err(),
        ParserError::DuplicateKey
    );
}

#[test]
fn strict_json_rejects_excessive_nesting() {
    let bytes = load("depth-over-limit.json");
    assert_eq!(
        audit_parse_strict_json(&bytes).unwrap_err(),
        ParserError::TooDeep
    );
}

#[test]
fn rejects_unknown_nested_key_without_echo() {
    assert_reject(
        &load("unknown-nested-key.json"),
        ParserError::UnsupportedNodeField { node_index: 0 },
    );
}

#[test]
fn rejects_case_insensitive_duplicate_keys() {
    let bytes = load("duplicate-key-case.json");
    assert_eq!(
        audit_validate_fixture_opts(&bytes).unwrap_err(),
        ParserError::DuplicateKey
    );
}

#[test]
fn rejects_control_and_bidi_text() {
    for name in [
        "control-nul.json",
        "control-cr.json",
        "control-lf.json",
        "control-bel.json",
        "control-bidi.json",
    ] {
        assert_reject(&load(name), ParserError::InvalidNode { node_index: 0 });
    }
}

#[test]
fn accepts_and_rejects_string_length_bounds() {
    let accepted = audit_validate_fixture_opts(&load("string-at-limit.json"))
        .expect("256-byte path accepted");
    assert!(accepted.get("path").and_then(Value::as_str).is_some());
    assert_reject(
        &load("string-over-limit.json"),
        ParserError::InvalidNode { node_index: 0 },
    );
}

#[test]
fn accepts_and_rejects_list_bounds() {
    audit_validate_fixture_opts(&load("empty-list.json")).expect("empty items accepted");
    audit_validate_fixture_opts(&load("list-at-limit.json")).expect("128 items accepted");
    assert_reject(
        &load("list-over-limit.json"),
        ParserError::InvalidNode { node_index: 0 },
    );
}

#[test]
fn rejects_invalid_scalar_types() {
    for name in [
        "type-string-for-int.json",
        "type-map-for-list.json",
        "type-number-for-bool.json",
    ] {
        assert_reject(&load(name), ParserError::InvalidNode { node_index: 0 });
    }
}

#[test]
fn enforces_exclusive_and_paired_fields() {
    assert_reject(
        &load("exclusive-left-right.json"),
        ParserError::InvalidNode { node_index: 0 },
    );
    assert_reject(
        &load("exclusive-alpha-beta.json"),
        ParserError::InvalidNode { node_index: 0 },
    );
    audit_validate_fixture_opts(&load("exclusive-left-only.json")).expect("single left accepted");
    audit_validate_fixture_opts(&load("exclusive-alpha-only.json")).expect("single alpha accepted");
    assert_reject(
        &load("pair-token-only.json"),
        ParserError::InvalidNode { node_index: 0 },
    );
    assert_reject(
        &load("pair-name-only.json"),
        ParserError::InvalidNode { node_index: 0 },
    );
    audit_validate_fixture_opts(&load("pair-both.json")).expect("token pair accepted");
}

#[test]
fn canonical_digest_is_order_independent() {
    let a = audit_validate_fixture_opts(&load("order-a.json")).expect("order-a");
    let b = audit_validate_fixture_opts(&load("order-b.json")).expect("order-b");
    let digest_a = audit_fixture_opts_digest(&a);
    let digest_b = audit_fixture_opts_digest(&b);
    assert_eq!(digest_a, digest_b);

    let different = audit_validate_fixture_opts(&load("order-value-differs.json"))
        .expect("order-value-differs");
    assert_ne!(audit_fixture_opts_digest(&different), digest_b);
}
