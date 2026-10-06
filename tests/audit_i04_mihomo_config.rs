//! I04.T04.a / V.03 / I04.T04.b: mihomo profile generator and `mihomo -t` codes.
//!
//! Value checks are stricter than `mihomo -t`. Without `CM_TEST_MIHOMO` the core
//! half prints SKIPPED and is not a pass.

use cm::common::contract_fixtures::TempDirGuard;
use cm::core::mihomo::{
    ConfigError, CoreValidationCode, attach_worker_listeners, generate_config, generate_from_store,
    profile_document, validate_file,
};
use cm::core::worker::generate as generate_worker;
use cm::profiles::{DnsPolicy, FakeIpPolicy, Id, Ipv6Policy, NodeSelection, SourceNodeRef, Store};
use cm::sources::pipeline::create_accepted_source;
use cm::sources::{
    ConfiguredEndpoint, HttpResponse, ImportFormat, NegotiationPolicy, UserAgent,
    negotiate_with_clock, parse_native,
};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::Duration;

fn dns(pass_ipv6: bool, fake_ip: bool) -> DnsPolicy {
    DnsPolicy {
        ipv6: if pass_ipv6 {
            Ipv6Policy::Pass
        } else {
            Ipv6Policy::Block
        },
        fake_ip: if fake_ip {
            FakeIpPolicy::Allowed
        } else {
            FakeIpPolicy::Forbidden
        },
    }
}

fn ss(name: &str) -> Value {
    json!({
        "name": name,
        "type": "ss",
        "server": "edge.example.invalid",
        "port": 8388,
        "cipher": "aes-128-gcm",
        "password": "p"
    })
}

fn assert_closed(error: &impl std::fmt::Display, marker: &str) {
    let shown = error.to_string();
    assert!(!shown.contains(marker), "error display leaked a marker");
}

#[test]
fn generator_rejects_port_zero_and_bad_uuid_that_mihomo_test_accepts() {
    let proxy = json!({
        "name": "vless-bad",
        "type": "vless",
        "server": "edge.example.invalid",
        "port": 0,
        "uuid": "bad"
    });
    let error = generate_config(
        std::slice::from_ref(&proxy),
        &dns(false, false),
        18080,
        &[],
        false,
    )
    .unwrap_err();
    assert!(matches!(error, ConfigError::InvalidNode));
    assert_closed(&error, "bad");
    assert_closed(&error, "edge.example.invalid");
    assert!(!format!("{error:?}").contains("bad"));

    let string_port = json!({
        "name": "vless-str",
        "type": "vless",
        "server": "edge.example.invalid",
        "port": "443",
        "uuid": "123e4567-e89b-12d3-a456-426614174000"
    });
    assert!(matches!(
        generate_config(
            std::slice::from_ref(&string_port),
            &dns(false, false),
            18080,
            &[],
            false
        ),
        Err(ConfigError::InvalidNode)
    ));

    let Some(binary) = std::env::var_os("CM_TEST_MIHOMO") else {
        eprintln!("I04.T04.a SKIPPED: CM_TEST_MIHOMO not set");
        return;
    };
    let sandbox = TempDirGuard::new("cm-i04-strict").unwrap();
    let raw = json!({
        "mode": "rule",
        "proxies": [proxy],
        "rules": ["MATCH,DIRECT"]
    });
    let file = sandbox.path().join("loose.json");
    std::fs::write(&file, serde_json::to_vec(&raw).unwrap()).unwrap();
    let output = Command::new(binary)
        .arg("-t")
        .arg("-d")
        .arg(sandbox.path())
        .arg("-f")
        .arg(&file)
        .output()
        .expect("spawn mihomo -t");
    assert!(
        output.status.success(),
        "mihomo -t rejected the loose document; exit {}",
        output.status
    );
}

#[test]
fn generated_listener_is_mixed_and_host_keys_are_forbidden() {
    let proxy = ss("ss-marker");
    let policy = dns(true, true);
    let document = profile_document(
        std::slice::from_ref(&proxy),
        &policy,
        &["DOMAIN-SUFFIX,example.invalid,DIRECT"],
        true,
    )
    .unwrap();
    let worker = generate_worker(&document, 18081).unwrap();
    let worker_json: Value = serde_json::from_str(&worker).unwrap();
    assert_eq!(worker_json["listeners"][0]["listen"], "127.0.0.1");
    assert_eq!(worker_json["listeners"][0]["port"], 18081);
    assert!(worker_json["listeners"][0].get("type").is_none());

    let bytes = generate_config(
        std::slice::from_ref(&proxy),
        &policy,
        18081,
        &["DOMAIN-SUFFIX,example.invalid,DIRECT"],
        true,
    )
    .unwrap();
    let parsed: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed["listeners"][0]["name"], "cm");
    assert_eq!(parsed["listeners"][0]["type"], "mixed");
    assert_eq!(parsed["listeners"][0]["listen"], "127.0.0.1");
    assert_eq!(parsed["listeners"][0]["port"], 18081);
    assert_eq!(parsed["ipv6"], true);
    assert_eq!(parsed["dns"]["enhanced-mode"], "fake-ip");
    assert_eq!(parsed["dns"]["fake-ip-range"], "198.18.0.1/16");
    assert!(parsed["dns"].get("listen").is_none());
    assert_eq!(parsed["dns"]["default-nameserver"][0], "1.1.1.1");
    assert_eq!(parsed["proxy-groups"][0]["type"], "url-test");
    assert_eq!(parsed["proxy-groups"][0]["proxies"][0], "ss-marker");
    assert_eq!(parsed["rules"][1], "MATCH,⚡ Auto");
    let text = String::from_utf8(bytes).unwrap();
    for banned in [
        "auto-route",
        "auto-redirect",
        "dns-hijack",
        "mixed-port",
        "external-controller",
        "tun",
        "dns.listen",
        "\"system\"",
    ] {
        assert!(!text.contains(banned), "banned substring {banned}");
    }

    let direct = generate_config(
        std::slice::from_ref(&proxy),
        &dns(false, false),
        18081,
        &[],
        false,
    )
    .unwrap();
    let direct: Value = serde_json::from_slice(&direct).unwrap();
    assert_eq!(direct["proxy-groups"][0]["type"], "select");
    assert_eq!(direct["rules"][0], "MATCH,DIRECT");
    assert_eq!(direct["ipv6"], false);
    assert_eq!(direct["dns"]["enhanced-mode"], "redir-host");
    assert!(direct["dns"].get("fake-ip-range").is_none());
    assert!(direct.get("auto-route").is_none());

    let mut nested = ss("ss-marker");
    nested["auto-route"] = json!(true);
    assert!(matches!(
        generate_config(std::slice::from_ref(&nested), &policy, 18081, &[], false),
        Err(ConfigError::Forbidden)
    ));
    let mut with_tun = document;
    with_tun["tun"] = json!({"enable": true});
    let forbidden = attach_worker_listeners(&with_tun, 18081).unwrap_err();
    assert!(matches!(forbidden, ConfigError::Forbidden));
    assert_closed(&forbidden, "tun");

    assert!(matches!(
        generate_config(&[], &policy, 18081, &[], false),
        Err(ConfigError::Empty)
    ));
    assert!(matches!(
        generate_config(
            std::slice::from_ref(&proxy),
            &policy,
            18081,
            &["MATCH,DIRECT\nDROP"],
            false
        ),
        Err(ConfigError::InvalidRule)
    ));
}

#[test]
fn generate_from_store_emits_published_node_without_banned_keys() {
    let root = TempDirGuard::new("cm-i04-store").unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = Store::initialize(root.path().join("store")).unwrap();
    let body = serde_json::to_vec(&json!({
        "log-level": "warning",
        "proxies": [{
            "name": "node-marker-b",
            "type": "ss",
            "server": "edge.example.invalid",
            "port": 443,
            "cipher": "aes-128-gcm",
            "password": "p"
        }]
    }))
    .unwrap();
    let endpoint = ConfiguredEndpoint::new("https://feed.example.invalid/sub", false).unwrap();
    let agents = [UserAgent::new("cm-test").unwrap()];
    let parsed = parse_native("1.19.32", ImportFormat::MihomoJson, &body).unwrap();
    let mut parsed = Some(parsed);
    let accepted = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &agents,
        &[],
        None,
        &NegotiationPolicy::default(),
        || Duration::ZERO,
        || Ok(1_000_i64),
        |_| Ok(HttpResponse::new(200, body.clone())),
        |_| Ok(parsed.take().unwrap()),
    )
    .unwrap();
    let receipt = create_accepted_source(&store, 0, accepted).unwrap();
    let bytes = generate_from_store(
        &store,
        &receipt.source_id,
        NodeSelection::Policy {
            name: "default".to_owned(),
        },
        &dns(false, true),
        18082,
        &[],
        false,
    )
    .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(
        text.contains("node-marker-b"),
        "published node name missing"
    );
    assert!(
        !text.contains("feed.example.invalid"),
        "source url leaked into config"
    );
    for banned in [
        "auto-route",
        "auto-redirect",
        "dns-hijack",
        "mixed-port",
        "external-controller",
        "tun",
        "dns.listen",
    ] {
        assert!(!text.contains(banned), "banned substring {banned}");
    }
    let wrong = Id::new("other-source").unwrap();
    let pinned = NodeSelection::Pinned {
        node: SourceNodeRef {
            source_id: wrong,
            source_generation: receipt.source_generation,
            node_id: receipt.node_ids[0].clone(),
        },
    };
    assert!(matches!(
        generate_from_store(
            &store,
            &receipt.source_id,
            pinned,
            &dns(false, false),
            18082,
            &[],
            false
        ),
        Err(ConfigError::InvalidNode)
    ));
}

#[test]
fn corpus_parses_with_empty_omissions() {
    let corpus = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/i03_core_corpus.json"
    ))
    .unwrap();
    let parsed = parse_native("1.19.32", ImportFormat::MihomoJson, &corpus).expect("corpus");
    assert!(parsed.omissions().is_empty());
    assert!(parsed.node_count() >= 13);
}

#[test]
fn validate_file_drops_config_text() {
    let Some(binary) = std::env::var_os("CM_TEST_MIHOMO") else {
        eprintln!("I04.T04.b SKIPPED: CM_TEST_MIHOMO not set");
        return;
    };
    let sandbox = TempDirGuard::new("cm-i04-validate").unwrap();
    let broken = br#"{"secret-marker-b":1,"proxies":[{"name":"secret-host.example""#;
    let error = validate_file(std::path::Path::new(&binary), sandbox.path(), broken).unwrap_err();
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains("secret-marker-b"));
    assert!(!shown.contains("secret-host.example"));
    assert!(matches!(
        error,
        CoreValidationCode::Syntax
            | CoreValidationCode::MissingField
            | CoreValidationCode::UnknownField
            | CoreValidationCode::InvalidValue
            | CoreValidationCode::Rejected
    ));

    let proxy = ss("ss-marker");
    let ok = generate_config(
        std::slice::from_ref(&proxy),
        &dns(false, false),
        18083,
        &[],
        false,
    )
    .unwrap();
    let sandbox = TempDirGuard::new("cm-i04-validate-ok").unwrap();
    validate_file(std::path::Path::new(&binary), sandbox.path(), &ok).expect("generator config");
}

#[test]
fn nodes_that_bypass_cm_routing_are_forbidden() {
    for key in ["interface-name", "routing-mark", "dialer-proxy"] {
        let mut node = ss("ss-marker");
        node[key] = json!("eth0");
        let error = generate_config(
            std::slice::from_ref(&node),
            &dns(false, false),
            18084,
            &[],
            false,
        )
        .unwrap_err();
        assert!(matches!(error, ConfigError::Forbidden), "{key}");
        assert_closed(&error, "eth0");
    }
}

#[test]
fn validate_file_does_not_follow_a_planted_symlink() {
    let sandbox = TempDirGuard::new("cm-i04-validate-link").unwrap();
    let outside = TempDirGuard::new("cm-i04-validate-outside").unwrap();
    let target = outside.path().join("victim");
    std::fs::write(&target, b"keep").unwrap();
    std::os::unix::fs::symlink(&target, sandbox.path().join("config.json")).unwrap();
    // The binary is never reached: opening the planted link fails first.
    let error =
        validate_file(std::path::Path::new("/nonexistent"), sandbox.path(), b"{}").unwrap_err();
    assert!(matches!(error, CoreValidationCode::Unavailable));
    assert_eq!(std::fs::read(&target).unwrap(), b"keep");
}

/// I04.T04.a done_when: the whole I03 corpus goes parser → generator → `mihomo -t`.
#[test]
fn whole_corpus_passes_generator_and_pinned_core() {
    let corpus = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/i03_core_corpus.json"
    ))
    .unwrap();
    let parsed = parse_native("1.19.32", ImportFormat::MihomoJson, &corpus).expect("corpus");
    let (_, nodes, _) = parsed.into_parts();
    let proxies: Vec<Value> = nodes
        .iter()
        .map(|node| node.full_definition().clone())
        .collect();
    let mut types: Vec<&str> = proxies
        .iter()
        .filter_map(|proxy| proxy["type"].as_str())
        .collect();
    types.sort_unstable();
    types.dedup();
    for expected in [
        "http",
        "hysteria2",
        "socks5",
        "ss",
        "trojan",
        "tuic",
        "vless",
        "vmess",
        "wireguard",
    ] {
        assert!(types.contains(&expected), "corpus lacks {expected}");
    }
    let config = generate_config(&proxies, &dns(true, true), 18085, &[], true)
        .expect("generator accepts every parser-accepted node");
    let Some(binary) = std::env::var_os("CM_TEST_MIHOMO") else {
        eprintln!("I04.T04.a core half SKIPPED: CM_TEST_MIHOMO not set");
        return;
    };
    let sandbox = TempDirGuard::new("cm-i04-corpus").unwrap();
    validate_file(std::path::Path::new(&binary), sandbox.path(), &config)
        .expect("pinned core accepts the generated corpus config");
}
