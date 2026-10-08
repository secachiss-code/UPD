//! I04.T04.f: publisher groups and rules, D1 omissions, and `mihomo -t`.
//!
//! GEOIP and GEOSITE stay on the host path. A worker config rejects them.
//! Without `CM_TEST_MIHOMO` only the core check prints SKIPPED.

use cm::common::Config;
use cm::common::contract_fixtures::{EnvGuard, TempDirGuard, isolation_lock};
use cm::core::mihomo::{ConfigError, generate_from_store, validate_file};
use cm::profiles::{DnsPolicy, FakeIpPolicy, Ipv6Policy, NodeSelection, Store};
use cm::sources::pipeline::create_accepted_source;
use cm::sources::{
    ConfiguredEndpoint, HttpResponse, ImportFormat, NegotiationPolicy, ParserError, UserAgent,
    negotiate_with_clock, parse_native,
};
use cm::vpn;
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

const MARKER: &str = "secret-marker-g";

fn dns() -> DnsPolicy {
    DnsPolicy {
        ipv6: Ipv6Policy::Block,
        fake_ip: FakeIpPolicy::Forbidden,
    }
}

fn ss() -> Value {
    json!({
        "name": "n1",
        "type": "ss",
        "server": "203.0.113.8",
        "port": 443,
        "cipher": "aes-128-gcm",
        "password": "x"
    })
}

fn parse(body: &Value) -> Result<cm::sources::ParsedSource, ParserError> {
    let bytes = serde_json::to_vec(body).unwrap();
    parse_native("1.19.32", ImportFormat::MihomoJson, &bytes)
}

fn publish(root: &std::path::Path, body: &Value) -> String {
    let store = Store::initialize(root.join("store")).unwrap();
    let bytes = serde_json::to_vec(body).unwrap();
    let endpoint = ConfiguredEndpoint::new("https://feed.example.invalid/sub", false).unwrap();
    let agents = [UserAgent::new("cm-test").unwrap()];
    let parsed = parse_native("1.19.32", ImportFormat::MihomoJson, &bytes).unwrap();
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
        |_| Ok(HttpResponse::new(200, bytes.clone())),
        |_| Ok(parsed.take().unwrap()),
    )
    .unwrap();
    let receipt = create_accepted_source(&store, 0, accepted).unwrap();
    let snapshot = store.read_snapshot().unwrap();
    let source = snapshot.sources.get(&receipt.source_id).unwrap();
    let sections = &source.provenance.as_ref().unwrap().omissions.section_names;
    assert!(
        !sections.iter().any(|name| {
            matches!(
                name.as_str(),
                "proxy-groups" | "rules" | "sub-rules" | "rule-providers"
            )
        }),
        "D1 sections still omitted: {sections:?}"
    );
    receipt.source_id.to_string()
}

#[test]
fn relay_ghost_cycle_and_provider_are_explicit() {
    let relay = parse(&json!({
        "proxies": [ss()],
        "proxy-groups": [{"name": "Relay", "type": "relay", "proxies": ["n1"]}],
        "rules": ["MATCH,n1"]
    }));
    assert_eq!(relay.unwrap_err(), ParserError::PublisherRelay);

    let ghost = parse(&json!({
        "proxies": [ss()],
        "rules": [format!("DOMAIN-SUFFIX,example.invalid,{MARKER}")]
    }));
    let ghost = ghost.unwrap_err();
    assert_eq!(ghost, ParserError::PublisherGhostTarget);
    assert!(!ghost.to_string().contains(MARKER));

    let cycle = parse(&json!({
        "proxies": [ss()],
        "sub-rules": {
            "a": ["SUB-RULE,(NETWORK,TCP),b"],
            "b": ["SUB-RULE,(NETWORK,TCP),a"]
        },
        "rules": ["MATCH,n1"]
    }));
    assert_eq!(cycle.unwrap_err(), ParserError::PublisherCycle);

    let provider = parse(&json!({
        "proxies": [ss()],
        "rule-providers": {
            "remote": {"type": "http", "behavior": "domain", "url": format!("https://{MARKER}.example/rules")}
        },
        "rules": ["RULE-SET,remote,DIRECT"]
    }));
    let provider = provider.unwrap_err();
    assert_eq!(provider, ParserError::PublisherProvider);
    assert!(!provider.to_string().contains(MARKER));
}

#[test]
fn stored_body_is_parsed_again_without_a_download() {
    let body = json!({
        "proxies": [ss()],
        "proxy-groups": [
            {"name": "Extra", "type": "select", "proxies": ["n1"]},
            {"name": "Fast", "type": "url-test", "proxies": ["n1"], "url": "https://www.gstatic.com/generate_204", "interval": 300},
            {"name": "Fall", "type": "fallback", "proxies": ["n1"], "url": "https://www.gstatic.com/generate_204", "interval": 300},
            {"name": "Bal", "type": "load-balance", "proxies": ["n1"]}
        ],
        "sub-rules": {"via": ["DOMAIN-SUFFIX,sub.example,DIRECT"]},
        "rule-providers": {"inline-set": {"type": "inline", "behavior": "domain", "payload": ["example.com"]}},
        "rules": ["RULE-SET,inline-set,DIRECT", "SUB-RULE,(NETWORK,TCP),via", "DOMAIN-SUFFIX,publisher.example,Extra", "MATCH,n1"]
    });
    let parsed = parse(&body).unwrap();
    assert!(
        parsed.omissions().section_names.is_empty(),
        "accepted groups and rules were recorded as D1: {:?}",
        parsed.omissions().section_names
    );
    let bytes = serde_json::to_vec(&body).unwrap();
    let names = std::collections::BTreeSet::from(["n1".to_owned()]);
    let policy = cm::core::mihomo::policy::from_raw(&bytes, &names).unwrap();
    assert!(policy.groups.iter().any(|group| group["name"] == "Extra"));
    assert!(
        policy
            .rules
            .iter()
            .any(|rule| rule.contains("publisher.example"))
    );
    assert!(!policy.sub_rules.is_empty());
}

#[test]
fn geo_rules_stay_on_the_host_and_are_rejected_for_a_worker() {
    let dir = TempDirGuard::new("cm-i04-geo").unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let lock = isolation_lock();
    let mut env = EnvGuard::new();
    env.set("CM_STATE_DIR", dir.path().join("state"));
    env.set("CM_VPN_ETC", dir.path().join("etc"));
    env.set("CM_VPN_HOME", dir.path().join("home"));
    env.set("CM_CONF", dir.path().join("cm.conf"));
    env.set("CM_PROFILE_STORE", dir.path().join("store"));
    fs::create_dir_all(dir.path().join("state")).unwrap();
    fs::create_dir_all(dir.path().join("etc")).unwrap();
    fs::create_dir_all(dir.path().join("home/profiles")).unwrap();
    fs::write(
        format!("{}/profiles/p1.yaml", vpn::home()),
        "proxies:\n  - {name: n1, type: ss, server: 203.0.113.8, port: 443, cipher: aes-128-gcm, password: x}\nproxy-groups:\n  - {name: Proxy, type: select, proxies: [n1]}\nrules:\n  - MATCH,Proxy\n",
    )
    .unwrap();
    fs::write(
        format!("{}/subs.json", vpn::etc()),
        r#"{"active":"p1","list":[{"id":"p1","name":"p","url":"https://feed.example.invalid/sub","kind":"clash"}]}"#,
    )
    .unwrap();
    let body = json!({
        "proxies": [ss()],
        "rules": ["GEOIP,RU,DIRECT", "GEOSITE,private,DIRECT", "MATCH,n1"]
    });
    let id = publish(dir.path(), &body);
    let mut subs: vpn::Subs =
        serde_json::from_str(&fs::read_to_string(format!("{}/subs.json", vpn::etc())).unwrap())
            .unwrap();
    subs.store_source = id.clone();
    fs::write(
        format!("{}/subs.json", vpn::etc()),
        serde_json::to_vec(&subs).unwrap(),
    )
    .unwrap();
    let mut config = Config::defaults(vec![]);
    config.vpn_store_source = true;
    config.vpn_direct_lan = false;
    config.vpn_direct_ru = false;
    config.vpn_auto_select = false;
    config.vpn_dns = false;
    let built = vpn::build_config(&config).unwrap();
    assert!(built.contains("GEOIP,RU,DIRECT"), "{built}");
    assert!(built.contains("GEOSITE,private,DIRECT"), "{built}");
    let store = Store::open(dir.path().join("store")).unwrap();
    let source = cm::profiles::Id::new(&id).unwrap();
    let error = generate_from_store(
        &store,
        &source,
        NodeSelection::Policy {
            name: "default".to_owned(),
        },
        &dns(),
        18083,
        &[],
        false,
    )
    .unwrap_err();
    assert_eq!(error, ConfigError::InvalidRule);
    assert!(!error.to_string().contains("GEOIP"));
    assert!(!error.to_string().contains("203.0.113.8"));
    drop(env);
    drop(lock);
}

#[test]
fn publisher_fixture_is_accepted_by_mihomo() {
    let Some(binary) = std::env::var_os("CM_TEST_MIHOMO") else {
        eprintln!("I04.T04.f SKIPPED: CM_TEST_MIHOMO not set");
        return;
    };
    let dir = TempDirGuard::new("cm-i04-policy").unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let body = json!({
        "proxies": [ss()],
        "proxy-groups": [
            {"name": "Extra", "type": "select", "proxies": ["n1"]},
            {"name": "Fast", "type": "url-test", "proxies": ["n1"], "url": "https://www.gstatic.com/generate_204", "interval": 300},
            {"name": "Fall", "type": "fallback", "proxies": ["n1"], "url": "https://www.gstatic.com/generate_204", "interval": 300},
            {"name": "Bal", "type": "load-balance", "proxies": ["n1"]}
        ],
        "sub-rules": {"via": ["DOMAIN-SUFFIX,sub.example,DIRECT"]},
        "rule-providers": {"inline-set": {"type": "inline", "behavior": "domain", "payload": ["example.com"]}},
        "rules": ["RULE-SET,inline-set,DIRECT", "SUB-RULE,(NETWORK,TCP),via", "DOMAIN-SUFFIX,publisher.example,Extra", "MATCH,n1"]
    });
    let parsed = parse(&body).unwrap();
    assert!(parsed.omissions().section_names.is_empty());
    let id = publish(dir.path(), &body);
    let store = Store::open(dir.path().join("store")).unwrap();
    let source = cm::profiles::Id::new(&id).unwrap();
    let bytes = generate_from_store(
        &store,
        &source,
        NodeSelection::Policy {
            name: "default".to_owned(),
        },
        &dns(),
        18084,
        &["DOMAIN-SUFFIX,cm-user.example,DIRECT"],
        false,
    )
    .unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.contains("\"name\":\"Extra\""), "{text}");
    assert!(text.contains("\"inline-set\""), "{text}");
    let document: Value = serde_json::from_slice(&bytes).unwrap();
    let rules: Vec<&str> = document["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|rule| rule.as_str().unwrap())
        .collect();
    assert_eq!(rules.first(), Some(&"DOMAIN-SUFFIX,cm-user.example,DIRECT"));
    assert_eq!(rules.last(), Some(&"MATCH,n1"));
    assert_eq!(
        rules
            .iter()
            .filter(|rule| rule.starts_with("MATCH,"))
            .count(),
        1
    );
    assert!(text.contains("publisher.example"), "{text}");
    assert!(!text.contains("feed.example.invalid"));
    let sandbox = dir.path().join("sandbox");
    fs::create_dir(&sandbox).unwrap();
    validate_file(std::path::Path::new(&binary), &sandbox, &bytes).expect("mihomo -t");
}

#[test]
fn provider_groups_and_name_clashes_are_explicit() {
    let uses = parse(&json!({
        "proxies": [ss()],
        "proxy-groups": [{"name": "Feed", "type": "select", "use": ["remote"]}],
        "rules": ["MATCH,Feed"]
    }));
    assert_eq!(uses.unwrap_err(), ParserError::PublisherProvider);

    let names = std::collections::BTreeSet::from(["n1".to_owned()]);
    let body = json!({
        "proxies": [ss()],
        "proxy-groups": [{"name": vpn::AUTO_GROUP, "type": "select", "proxies": ["n1"]}],
        "rules": ["MATCH,n1"]
    });
    let policy =
        cm::core::mihomo::policy::from_raw(&serde_json::to_vec(&body).unwrap(), &names).unwrap();
    let mut document = json!({
        "proxies": [ss()],
        "proxy-groups": [{"name": vpn::AUTO_GROUP, "type": "select", "proxies": ["n1"]}],
        "rules": [format!("MATCH,{}", vpn::AUTO_GROUP)]
    });
    assert_eq!(
        cm::core::mihomo::policy::merge_worker(&mut document, &policy).unwrap_err(),
        ConfigError::InvalidRule
    );
}
