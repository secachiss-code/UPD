//! V.04: `cm vpn use source:ID` behind `vpn_store_source`.
//!
//! Rule order is CM user rules, then the legacy generator's geodata rules,
//! then subscription rules, then publisher rules, then one final MATCH.

use cm::common::contract_fixtures::{EnvGuard, TempDirGuard, isolation_lock};
use cm::common::{Config, conf_path};
use cm::profiles::Store;
use cm::sources::pipeline::create_accepted_source;
use cm::sources::{
    ConfiguredEndpoint, HttpResponse, ImportFormat, NegotiationPolicy, UserAgent,
    negotiate_with_clock, parse_native,
};
use cm::vpn::{self, Subs};
use serde::Deserialize;
use serde_json::{Value, json};
use serde_yaml::Value as Yaml;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

const MARKER: &str = "secret-marker-v04";

struct VpnDirs {
    _lock: std::sync::MutexGuard<'static, ()>,
    _env: EnvGuard,
    _dir: TempDirGuard,
}

impl VpnDirs {
    fn new() -> Self {
        let dir = TempDirGuard::new("cm-v04").unwrap();
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
        fs::create_dir_all(dir.path().join("home")).unwrap();
        fs::create_dir_all(dir.path().join("home/profiles")).unwrap();
        Self {
            _lock: lock,
            _env: env,
            _dir: dir,
        }
    }

    fn path(&self) -> &Path {
        self._dir.path()
    }
}

fn profile() -> String {
    "proxies:\n  - {name: n1, type: ss, server: 203.0.113.8, port: 443, cipher: aes-128-gcm, password: x}\nproxy-groups:\n  - {name: Proxy, type: select, proxies: [n1]}\nrules:\n  - MATCH,Proxy\n".to_owned()
}

fn write_profile() {
    fs::write(format!("{}/profiles/p1.yaml", vpn::home()), profile()).unwrap();
    let subs = json!({
        "active": "p1",
        "list": [{
            "id": "p1",
            "name": "p",
            "url": format!("https://{MARKER}.example/sub"),
            "kind": "clash"
        }]
    });
    fs::write(format!("{}/subs.json", vpn::etc()), subs.to_string()).unwrap();
}

fn config() -> Config {
    let mut c = Config::defaults(vec![]);
    c.vpn_direct_lan = false;
    c.vpn_direct_ru = false;
    c.vpn_auto_select = false;
    c.vpn_dns = false;
    c
}

fn rule_list(doc: &str) -> Vec<String> {
    let value: Yaml = serde_yaml::from_str(doc).unwrap();
    value["rules"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|rule| rule.as_str().unwrap().to_owned())
        .collect()
}

fn publish(body: Value) -> String {
    let store = Store::initialize(std::env::var("CM_PROFILE_STORE").unwrap()).unwrap();
    let bytes = serde_json::to_vec(&body).unwrap();
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
    create_accepted_source(&store, 0, accepted)
        .unwrap()
        .source_id
        .to_string()
}

fn node(groups: Value, rules: Value) -> Value {
    json!({
        "proxies": [{
            "name": "n1",
            "type": "ss",
            "server": "203.0.113.8",
            "port": 443,
            "cipher": "aes-128-gcm",
            "password": "x"
        }],
        "proxy-groups": groups,
        "rules": rules
    })
}

fn select_source(id: &str, c: &mut Config) {
    c.vpn_store_source = true;
    let mut subs: Subs =
        serde_json::from_str(&fs::read_to_string(format!("{}/subs.json", vpn::etc())).unwrap())
            .unwrap();
    subs.store_source = id.to_owned();
    fs::write(
        format!("{}/subs.json", vpn::etc()),
        serde_json::to_vec(&subs).unwrap(),
    )
    .unwrap();
}

/// A legacy clash profile of a different feed: other node, group, rules and sub-rules.
fn legacy_other_feed() -> String {
    "proxies:\n  - {name: legacy-node, type: ss, server: 203.0.113.9, port: 443, cipher: aes-128-gcm, password: x}\nproxy-groups:\n  - {name: Legacy, type: select, proxies: [legacy-node]}\nsub-rules:\n  own:\n    - MATCH,DIRECT\nrules:\n  - DOMAIN-SUFFIX,legacy.example,Legacy\n  - MATCH,Legacy\n".to_owned()
}

fn group_names(doc: &str) -> Vec<String> {
    let value: Yaml = serde_yaml::from_str(doc).unwrap();
    value["proxy-groups"]
        .as_sequence()
        .unwrap()
        .iter()
        .filter_map(|group| group["name"].as_str().map(str::to_owned))
        .collect()
}

fn assert_closed(text: &str) {
    assert!(!text.contains(MARKER), "output leaked a marker");
}

#[test]
fn flag_off_matches_the_legacy_config() {
    let _dirs = VpnDirs::new();
    write_profile();
    let off = config();
    let mut on_empty = config();
    on_empty.vpn_store_source = true;
    let legacy = vpn::build_config(&off).unwrap();
    let empty = vpn::build_config(&on_empty).unwrap();
    assert_eq!(legacy, empty);
    assert_closed(&legacy);
}

#[test]
fn flag_off_with_a_selected_source_is_an_error() {
    let _dirs = VpnDirs::new();
    write_profile();
    let mut subs: Subs =
        serde_json::from_str(&fs::read_to_string(format!("{}/subs.json", vpn::etc())).unwrap())
            .unwrap();
    subs.store_source = "src".into();
    fs::write(
        format!("{}/subs.json", vpn::etc()),
        serde_json::to_vec(&subs).unwrap(),
    )
    .unwrap();
    let error = vpn::build_config(&config()).unwrap_err();
    assert!(error.contains("выключенном флаге"), "{error}");
    assert_closed(&error);
}

#[test]
fn publisher_rules_sit_before_the_final_match() {
    let _dirs = VpnDirs::new();
    write_profile();
    fs::write(
        format!("{}/rules.txt", vpn::etc()),
        "DOMAIN-SUFFIX,cm-user.example,DIRECT\n",
    )
    .unwrap();
    let id = publish(node(
        json!([{"name": "Extra", "type": "select", "proxies": ["n1"]}]),
        json!(["DOMAIN-SUFFIX,publisher.example,DIRECT", "MATCH,n1"]),
    ));
    let mut c = config();
    select_source(&id, &mut c);
    let built = vpn::build_config(&c).unwrap();
    assert_closed(&built);
    let rules = rule_list(&built);
    let user = rules
        .iter()
        .position(|rule| rule.contains("cm-user.example"))
        .unwrap();
    let publisher = rules
        .iter()
        .position(|rule| rule.contains("publisher.example"))
        .unwrap();
    let matcher = rules
        .iter()
        .rposition(|rule| rule.starts_with("MATCH,"))
        .unwrap();
    assert!(user < publisher && publisher < matcher, "{rules:?}");
    assert_eq!(matcher, rules.len() - 1);
    assert_eq!(rules[matcher], "MATCH,n1");
    // Groups are the publisher's only: the legacy profile's Proxy is not mixed in.
    assert_eq!(group_names(&built), ["Extra"]);
}

#[test]
fn an_auto_group_name_rejects_the_build() {
    let _dirs = VpnDirs::new();
    write_profile();
    let id = publish(node(
        json!([{"name": vpn::AUTO_GROUP, "type": "select", "proxies": ["n1"]}]),
        json!(["MATCH,n1"]),
    ));
    let mut c = config();
    select_source(&id, &mut c);
    let before = fs::read(format!("{}/config.yaml", vpn::home())).unwrap_or_default();
    let error = vpn::build_config(&c).unwrap_err();
    assert!(error.contains("совпало"), "{error}");
    assert_closed(&error);
    assert_eq!(
        fs::read(format!("{}/config.yaml", vpn::home())).unwrap_or_default(),
        before
    );
}

/// The legacy profile and the Store source are the same feed in the user's A/B/A
/// check, so their group names coincide. That must not be a clash.
#[test]
fn a_publisher_group_named_like_the_legacy_one_is_accepted() {
    let _dirs = VpnDirs::new();
    write_profile();
    let id = publish(node(
        json!([{"name": "Proxy", "type": "select", "proxies": ["n1"]}]),
        json!(["MATCH,Proxy"]),
    ));
    let mut c = config();
    select_source(&id, &mut c);
    let built = vpn::build_config(&c).unwrap();
    assert_eq!(group_names(&built), ["Proxy"]);
    assert_eq!(rule_list(&built).last().unwrap(), "MATCH,Proxy");
}

#[test]
fn legacy_sections_do_not_leak_into_a_store_build() {
    let _dirs = VpnDirs::new();
    write_profile();
    fs::write(
        format!("{}/profiles/p1.yaml", vpn::home()),
        legacy_other_feed(),
    )
    .unwrap();
    let mut document = node(json!([]), json!(["MATCH,n1"]));
    document["sub-rules"] = json!({"extra": ["MATCH,DIRECT"]});
    let id = publish(document);
    let mut c = config();
    select_source(&id, &mut c);
    let built = vpn::build_config(&c).unwrap();
    assert_closed(&built);
    assert!(!built.contains("legacy-node"), "{built}");
    assert!(!built.contains("legacy.example"), "{built}");
    assert_eq!(group_names(&built), ["Proxy"]);
    let value: Yaml = serde_yaml::from_str(&built).unwrap();
    assert!(value["sub-rules"]["extra"].is_sequence(), "{built}");
    assert!(value["sub-rules"]["own"].is_null(), "{built}");
    assert_eq!(rule_list(&built).last().unwrap(), "MATCH,Proxy");
}

#[test]
fn a_store_source_needs_no_legacy_subscription() {
    let _dirs = VpnDirs::new();
    let id = publish(node(json!([]), json!([])));
    fs::write(
        format!("{}/subs.json", vpn::etc()),
        json!({"active": "", "list": [], "store_source": id}).to_string(),
    )
    .unwrap();
    let mut c = config();
    c.vpn_store_source = true;
    let built = vpn::build_config(&c).unwrap();
    assert_eq!(group_names(&built), ["Proxy"]);
    assert_eq!(rule_list(&built), ["MATCH,Proxy"]);
}

#[test]
fn host_build_from_another_feed_is_accepted_by_mihomo() {
    let Some(binary) = std::env::var_os("CM_TEST_MIHOMO") else {
        eprintln!("V.04 SKIPPED: CM_TEST_MIHOMO not set");
        return;
    };
    let dirs = VpnDirs::new();
    write_profile();
    fs::write(
        format!("{}/profiles/p1.yaml", vpn::home()),
        legacy_other_feed(),
    )
    .unwrap();
    let mut document = node(
        json!([
            {"name": "Proxy", "type": "select", "proxies": ["Fast", "n1"]},
            {"name": "Fast", "type": "url-test", "proxies": ["n1"], "url": "https://www.gstatic.com/generate_204", "interval": 300}
        ]),
        json!([
            "RULE-SET,inline-set,DIRECT",
            "DOMAIN-SUFFIX,publisher.example,Fast",
            "MATCH,Proxy"
        ]),
    );
    document["rule-providers"] =
        json!({"inline-set": {"type": "inline", "behavior": "domain", "payload": ["example.com"]}});
    let id = publish(document);
    let mut c = config();
    c.vpn_auto_select = true;
    select_source(&id, &mut c);
    let built = vpn::build_config(&c).unwrap();
    assert!(!built.contains("legacy-node"), "{built}");
    let sandbox = dirs.path().join("sandbox");
    fs::create_dir(&sandbox).unwrap();
    cm::core::mihomo::validate_file(Path::new(&binary), &sandbox, built.as_bytes())
        .expect("mihomo -t");
}

#[test]
fn reload_failure_restores_the_previous_config_bytes() {
    let dirs = VpnDirs::new();
    write_profile();
    let bin = dirs.path().join("home/bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("mihomo"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(bin.join("mihomo"), fs::Permissions::from_mode(0o755)).unwrap();
    let id = publish(node(
        json!([{"name": "Extra", "type": "select", "proxies": ["n1"]}]),
        json!(["DOMAIN-SUFFIX,publisher.example,DIRECT"]),
    ));
    let previous = b"previous-config\n".to_vec();
    fs::write(format!("{}/config.yaml", vpn::home()), &previous).unwrap();
    let mut c = config();
    c.vpn_store_source = true;
    let mut calls = 0;
    let mut reload = || {
        calls += 1;
        if calls == 1 {
            Err("reload refused".to_owned())
        } else {
            Ok(())
        }
    };
    let error = match vpn::switch_store_source(&id, &c, true, &mut reload) {
        Ok(switched) => panic!("reload failure was committed for {}", switched.source_id),
        Err(error) => error,
    };
    assert_closed(&error);
    assert!(error.contains("restored"), "{error}");
    assert_eq!(
        fs::read(format!("{}/config.yaml", vpn::home())).unwrap(),
        previous
    );
}

#[test]
fn validation_failure_keeps_the_previous_config_bytes() {
    let dirs = VpnDirs::new();
    write_profile();
    let bin = dirs.path().join("home/bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("mihomo"), "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(bin.join("mihomo"), fs::Permissions::from_mode(0o755)).unwrap();
    let id = publish(node(
        json!([{"name": "Extra", "type": "select", "proxies": ["n1"]}]),
        json!(["MATCH,n1"]),
    ));
    let previous = b"kept-config\n".to_vec();
    fs::write(format!("{}/config.yaml", vpn::home()), &previous).unwrap();
    let mut c = config();
    c.vpn_store_source = true;
    let mut reload = || panic!("reload after a rejected config");
    let error = match vpn::switch_store_source(&id, &c, true, &mut reload) {
        Ok(switched) => panic!("rejected config was committed for {}", switched.source_id),
        Err(error) => error,
    };
    assert_closed(&error);
    assert_eq!(
        fs::read(format!("{}/config.yaml", vpn::home())).unwrap(),
        previous
    );
}

#[test]
fn store_off_refuses_while_a_source_is_selected() {
    let _dirs = VpnDirs::new();
    write_profile();
    let mut c = config();
    vpn::set_store_source_flag(true, &mut c).unwrap();
    assert!(
        fs::read_to_string(conf_path())
            .unwrap()
            .contains("vpn_store_source = 1")
    );
    assert!(c.vpn_store_source);
    let mut subs: Subs =
        serde_json::from_str(&fs::read_to_string(format!("{}/subs.json", vpn::etc())).unwrap())
            .unwrap();
    subs.store_source = "src".into();
    fs::write(
        format!("{}/subs.json", vpn::etc()),
        serde_json::to_vec(&subs).unwrap(),
    )
    .unwrap();
    let error = vpn::set_store_source_flag(false, &mut c).unwrap_err();
    assert!(error.contains("cm vpn use N"), "{error}");
    assert_closed(&error);
    assert!(c.vpn_store_source);
}

#[test]
fn store_on_off_requires_root_outside_the_test_sandbox() {
    if cm::common::is_root() {
        return;
    }
    let _lock = isolation_lock();
    let mut env = EnvGuard::new();
    env.remove("CM_STATE_DIR");
    env.remove("UPD_STATE_DIR");
    let mut c = Config::defaults(vec![]);
    let error = vpn::set_store_source_flag(true, &mut c).unwrap_err();
    assert!(error.contains("root"), "{error}");
}

#[derive(Deserialize)]
struct LegacySubs {
    #[serde(default)]
    active: String,
    #[serde(default)]
    list: Vec<Value>,
}

#[test]
fn an_older_subs_reader_ignores_store_source() {
    let with_field = r#"{"active":"a","list":[],"store_source":"src"}"#;
    let legacy: LegacySubs = serde_json::from_str(with_field).unwrap();
    assert_eq!(legacy.active, "a");
    assert!(legacy.list.is_empty());
    let current: Subs = serde_json::from_str(with_field).unwrap();
    assert_eq!(current.store_source, "src");
    let old = r#"{"active":"a","list":[]}"#;
    assert!(
        serde_json::from_str::<Subs>(old)
            .unwrap()
            .store_source
            .is_empty()
    );
    let extra = r#"{"active":"a","list":[],"store_source":"src","future":1}"#;
    assert_eq!(
        serde_json::from_str::<Subs>(extra).unwrap().store_source,
        "src"
    );
}
