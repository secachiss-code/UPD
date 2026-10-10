//! K05, K06: mihomo nodes to an Xray config, and the Xray process lifecycle.
//! The process test runs inside `unshare -rn`. Without `CM_TEST_XRAY` it prints SKIPPED.

mod pack_support;

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use cm::common::contract_fixtures::TempDirGuard;
use cm::core::adapter::{ApiState, CoreAdapter, CoreConfig, CoreError, CoreReadiness};
use cm::core::xray::config::{XrayConfigError, generate_config, outbound};
use cm::core::xray::lifecycle::XrayWorker;
use cm::core::{InstanceId, InstanceRoot, LeaseRegistry};
use serde_json::{Value, json};

const UUID: &str = "11111111-2222-4333-8444-555555555555";

fn ss() -> Value {
    json!({"name":"n","type":"ss","server":"203.0.113.8","port":443,"cipher":"aes-128-gcm","password":"x"})
}

fn vless_ws() -> Value {
    json!({"name":"n","type":"vless","server":"203.0.113.8","port":443,"uuid":UUID,"tls":true,
        "servername":"example.invalid","network":"ws","ws-opts":{"path":"/p","headers":{"Host":"h.example"}}})
}

fn vless_reality() -> Value {
    json!({"name":"n","type":"vless","server":"203.0.113.8","port":443,"uuid":UUID,"tls":true,
        "servername":"example.invalid","flow":"xtls-rprx-vision",
        "reality-opts":{"public-key":"7xhH4b_VkliBxGulljcyPOH-bYUA2dl-XAdZAsfhk04","short-id":"6ba85179e30d4fc2"}})
}

fn trojan() -> Value {
    json!({"name":"n","type":"trojan","server":"203.0.113.8","port":443,"password":"x","sni":"example.invalid"})
}

fn vmess() -> Value {
    json!({"name":"n","type":"vmess","server":"203.0.113.8","port":443,"uuid":UUID})
}

#[test]
fn k05_supported_nodes() {
    assert_eq!(
        outbound(&ss(), "node").unwrap(),
        json!({"tag":"node","protocol":"shadowsocks","settings":{"servers":[
            {"address":"203.0.113.8","port":443,"method":"aes-128-gcm","password":"x"}]}})
    );
    let vless = outbound(&vless_ws(), "node").unwrap();
    assert_eq!(vless["protocol"], "vless");
    assert_eq!(
        vless["settings"]["vnext"][0]["users"][0],
        json!({"id":UUID,"encryption":"none"})
    );
    assert_eq!(
        vless["streamSettings"],
        json!({"network":"ws","wsSettings":{"path":"/p","headers":{"Host":"h.example"}},
            "security":"tls","tlsSettings":{"serverName":"example.invalid"}})
    );
    let reality = outbound(&vless_reality(), "node").unwrap();
    assert_eq!(reality["streamSettings"]["security"], "reality");
    assert_eq!(
        reality["streamSettings"]["realitySettings"],
        json!({"serverName":"example.invalid","publicKey":"7xhH4b_VkliBxGulljcyPOH-bYUA2dl-XAdZAsfhk04",
            "shortId":"6ba85179e30d4fc2","fingerprint":"chrome"})
    );
    assert_eq!(
        reality["settings"]["vnext"][0]["users"][0]["flow"],
        "xtls-rprx-vision"
    );
    let trojan = outbound(&trojan(), "node").unwrap();
    assert_eq!(trojan["streamSettings"]["security"], "tls");
    assert_eq!(
        trojan["streamSettings"]["tlsSettings"]["serverName"],
        "example.invalid"
    );
    let vmess = outbound(&vmess(), "node").unwrap();
    assert_eq!(
        vmess["settings"]["vnext"][0]["users"][0],
        json!({"id":UUID,"alterId":0,"security":"auto"})
    );
    assert!(vmess.get("streamSettings").is_none());
}

fn with(mut node: Value, key: &str, value: Value) -> Value {
    node[key] = value;
    node
}

#[test]
fn k05_unknown_fields_are_refused_not_dropped() {
    let field = Err(XrayConfigError::Unsupported("field"));
    assert_eq!(outbound(&with(ss(), "plugin", json!("obfs")), "n"), field);
    assert_eq!(
        outbound(&with(vless_ws(), "smux", json!({"enabled":true})), "n"),
        field
    );
    let mut early = vless_ws();
    early["ws-opts"]["max-early-data"] = json!(2048);
    assert_eq!(outbound(&early, "n"), field);
    let mut header = vless_ws();
    header["ws-opts"]["headers"]["X-Other"] = json!("1");
    assert_eq!(outbound(&header, "n"), field);
    assert_eq!(outbound(&with(ss(), "alpn", json!(["h2"])), "n"), field);
    // Hints about UDP and the socket change nothing in the result.
    let hinted = with(with(ss(), "udp", json!(true)), "tfo", json!(true));
    assert_eq!(outbound(&hinted, "node"), outbound(&ss(), "node"));
    let alpn = outbound(&with(trojan(), "alpn", json!(["h2"])), "node").unwrap();
    assert_eq!(alpn["streamSettings"]["tlsSettings"]["alpn"], json!(["h2"]));
}

#[test]
fn k05_unsupported_and_invalid() {
    for kind in ["tuic", "hysteria2", "wireguard", "http", "socks5"] {
        let node = json!({"type":kind,"server":"203.0.113.8","port":443});
        assert_eq!(
            outbound(&node, "n"),
            Err(XrayConfigError::Unsupported(kind))
        );
    }
    assert_eq!(
        outbound(&with(vless_ws(), "network", json!("h2")), "n"),
        Err(XrayConfigError::Unsupported("network"))
    );
    for key in ["dialer-proxy", "interface-name", "routing-mark"] {
        assert_eq!(
            outbound(&with(ss(), key, json!("x")), "n"),
            Err(XrayConfigError::Unsupported("dialer"))
        );
    }
    let secret = "secret-password-marker";
    let invalid = [
        with(ss(), "port", json!(0)),
        with(ss(), "port", json!(70000)),
        with(vmess(), "uuid", json!("x")),
        json!({"type":"ss","port":443,"cipher":"aes-128-gcm","password":secret}),
    ];
    for node in invalid {
        let error = outbound(&node, "n").unwrap_err();
        assert_eq!(error, XrayConfigError::InvalidNode);
        let text = error.to_string();
        for leak in ["203.0.113.8", secret, UUID] {
            assert!(!text.contains(leak));
        }
    }
    assert_eq!(generate_config(&[], 20000), Err(XrayConfigError::Empty));
    assert_eq!(
        generate_config(&[ss()], 0),
        Err(XrayConfigError::InvalidPort)
    );
}

#[test]
fn k05_full_config() {
    let config: Value = serde_json::from_slice(&generate_config(&[ss()], 20000).unwrap()).unwrap();
    assert_eq!(
        config["inbounds"],
        json!([{"tag":"cm","listen":"127.0.0.1","port":20000,"protocol":"mixed","settings":{"udp":true}}])
    );
    let tags: Vec<&str> = config["outbounds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["tag"].as_str().unwrap())
        .collect();
    assert_eq!(tags, ["node", "direct", "block"]);
    assert_eq!(
        config["routing"]["rules"],
        json!([{"type":"field","inboundTag":["cm"],"outboundTag":"node"}])
    );
}

fn xray() -> Option<PathBuf> {
    let path = std::env::var_os("CM_TEST_XRAY").map(PathBuf::from);
    if path.is_none() {
        eprintln!("K05/K06 SKIPPED: CM_TEST_XRAY not set");
    }
    path
}

#[test]
fn k05_pinned_xray_accepts_every_supported_node() {
    let Some(binary) = xray() else { return };
    let dir = TempDirGuard::new("cm-pack-k05").unwrap();
    for (name, node) in [
        ("ss", ss()),
        ("vmess", vmess()),
        ("vless-ws-tls", vless_ws()),
        ("vless-reality", vless_reality()),
        ("trojan", trojan()),
    ] {
        let path = dir.path().join(format!("{name}.json"));
        fs::write(&path, generate_config(&[node], 20000).unwrap()).unwrap();
        let status = Command::new(&binary)
            .args(["run", "-test", "-c"])
            .arg(&path)
            .env_clear()
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "{name}");
    }
}

fn config(nodes: &[Value]) -> CoreConfig {
    CoreConfig::from_bytes(generate_config(nodes, 20000).unwrap()).unwrap()
}

fn port_open(port: u16) -> bool {
    std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
}

#[test]
fn k06_xray_lifecycle() {
    let Some(binary) = xray() else { return };
    if pack_support::rerun_in_netns("k06_xray_lifecycle", "CM_PACK_K06_INNER") {
        return;
    }
    assert!(
        Command::new("ip")
            .args(["link", "set", "lo", "up"])
            .status()
            .unwrap()
            .success()
    );
    let dir = TempDirGuard::new("cm-pack-k06").unwrap();
    let leases = dir.path().join("leases");
    let id = InstanceId::new("proxy").unwrap();
    let mut worker = XrayWorker::new(id, InstanceRoot::new(dir.path()), &leases, &binary);
    let capabilities = worker.capabilities();
    assert_eq!(
        (
            capabilities.core,
            capabilities.reload_without_restart,
            capabilities.delay_probe
        ),
        ("xray", false, false)
    );
    // The shape is refused before a port is leased.
    let mut two: Value = serde_json::from_slice(config(&[ss()]).as_bytes()).unwrap();
    let inbound = two["inbounds"][0].clone();
    two["inbounds"].as_array_mut().unwrap().push(inbound);
    let mut open: Value = serde_json::from_slice(config(&[ss()]).as_bytes()).unwrap();
    open["inbounds"][0]["listen"] = json!("0.0.0.0");
    for bad in [two, open] {
        let bad = CoreConfig::from_bytes(serde_json::to_vec(&bad).unwrap()).unwrap();
        assert_eq!(worker.start(&bad).err(), Some(CoreError::InvalidConfig));
    }
    assert!(
        LeaseRegistry::open(&leases)
            .map(|r| r.list().unwrap().is_empty())
            .unwrap_or(true)
    );

    let readiness = worker.start(&config(&[ss()])).unwrap();
    assert_eq!(readiness.api, ApiState::ApiReady);
    let lease = LeaseRegistry::open(&leases).unwrap().list().unwrap();
    assert_eq!(lease.len(), 1);
    let port: u16 = lease[0].value.parse().unwrap();
    assert!(port_open(port));
    let written = dir.path().join("instances/proxy/config/config.json");
    assert_eq!(fs::metadata(&written).unwrap().mode() & 0o777, 0o600);
    let document: Value = serde_json::from_slice(&fs::read(&written).unwrap()).unwrap();
    assert_eq!(document["inbounds"][0]["port"], port);
    let pid = worker.core_pid().unwrap();
    let uid = fs::metadata(format!("/proc/{pid}")).unwrap().uid();
    assert_eq!(uid, cm::common::sys::euid());

    assert_eq!(
        worker.reload(&config(&[ss()])).err(),
        Some(CoreError::Unsupported)
    );
    assert_eq!(worker.core_pid(), Some(pid));
    assert_eq!(worker.statistics().err(), Some(CoreError::Unsupported));

    // A config the core rejects leaves the running process alone.
    let mut broken: Value = serde_json::from_slice(config(&[ss()]).as_bytes()).unwrap();
    broken["outbounds"][0]["settings"]["servers"][0]["method"] = json!("no-such-cipher");
    let broken = CoreConfig::from_bytes(serde_json::to_vec(&broken).unwrap()).unwrap();
    assert_eq!(
        worker.restart(&broken).err(),
        Some(CoreError::InvalidConfig)
    );
    assert_eq!(worker.core_pid(), Some(pid));
    assert!(port_open(port));

    worker.restart(&config(&[trojan()])).unwrap();
    let second = worker.core_pid().unwrap();
    assert_ne!(second, pid);
    let port: u16 = LeaseRegistry::open(&leases).unwrap().list().unwrap()[0]
        .value
        .parse()
        .unwrap();
    assert!(port_open(port));

    // SAFETY: the pid is this test's own child; SIGKILL has no other effect.
    unsafe { libc::kill(second as i32, libc::SIGKILL) };
    for _ in 0..100 {
        if worker.health() == Ok(CoreReadiness::DOWN) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(worker.health(), Ok(CoreReadiness::DOWN));
    worker.stop().unwrap();
    assert!(
        LeaseRegistry::open(&leases)
            .unwrap()
            .list()
            .unwrap()
            .is_empty()
    );
    assert!(!port_open(port));
}

/// X01: Xray drops to the caller's uid by the same mechanism as mihomo.
#[test]
fn x01_xray_runs_as_the_caller() {
    let Some(binary) = xray() else { return };
    if pack_support::rerun_in_userns("x01_xray_runs_as_the_caller", "CM_PACK_X01_INNER") {
        return;
    }
    assert!(
        Command::new("ip")
            .args(["link", "set", "lo", "up"])
            .status()
            .unwrap()
            .success()
    );
    let dir = TempDirGuard::new("cm-pack-x01").unwrap();
    let binary = pack_support::shared_copy(dir.path(), &binary);
    let mut worker = XrayWorker::new(
        InstanceId::new("proxy").unwrap(),
        InstanceRoot::new(dir.path().join("u1")),
        dir.path().join("leases"),
        &binary,
    );
    worker.set_run_as(cm::controller::drop::RunAs {
        uid: 1,
        gid: 1,
        groups: vec![1],
    });
    worker.start(&config(&[ss()])).unwrap();
    let pid = worker.core_pid().unwrap();
    let status = fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    for line in [
        "Uid:\t1\t1\t1\t1",
        "CapEff:\t0000000000000000",
        "NoNewPrivs:\t1",
    ] {
        assert!(
            status.lines().any(|candidate| candidate == line),
            "{line}\n{status}"
        );
    }
    // The config the core reads is the caller's file in a directory the manager keeps.
    let rendered = dir.path().join("u1/instances/proxy/core/config.json");
    let meta = fs::metadata(&rendered).unwrap();
    assert_eq!((meta.uid(), meta.mode() & 0o777), (1, 0o600));
    assert_eq!(fs::metadata(rendered.parent().unwrap()).unwrap().uid(), 0);
    worker.stop().unwrap();
}
