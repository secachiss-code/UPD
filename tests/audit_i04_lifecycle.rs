//! I04.T04.c: mihomo worker lifecycle on the unix API socket.
//!
//! The process test runs inside `unshare -rn`. Without `CM_TEST_MIHOMO` it
//! prints SKIPPED. A missing user or network namespace is a failure.

use cm::common::contract_fixtures::TempDirGuard;
use cm::common::sys::euid;
use cm::core::mihomo::api::request;
use cm::core::mihomo::{
    ConfigError, MihomoWorker, attach_instance_controller, reject_geo_document,
};
use cm::core::{
    ApiState, CoreAdapter, CoreConfig, CoreError, CoreReadiness, InstanceId, InstanceRoot,
    LeaseRegistry, ResourceKind,
};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn config(bytes: Vec<u8>) -> CoreConfig {
    CoreConfig::from_bytes(bytes).unwrap()
}

fn direct(level: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "mode": "direct",
        "log-level": level,
        "find-process-mode": "off",
        "ipv6": false
    }))
    .unwrap()
}

fn rejected_proxy() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "mode": "direct",
        "log-level": "warning",
        "find-process-mode": "off",
        "ipv6": false,
        "proxies": [{
            "name": "incomplete",
            "type": "ss",
            "server": "203.0.113.5",
            "port": 1
        }]
    }))
    .unwrap()
}

fn worker(root: &Path, binary: &Path) -> MihomoWorker {
    MihomoWorker::new(
        InstanceId::new("e1").unwrap(),
        InstanceRoot::new(root.join("state")),
        root.join("leases"),
        binary,
    )
}

fn group_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-s", "0", "--", &format!("-{pid}")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn peer_sites() -> Vec<String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    walk(&root, &root, &mut found);
    found.sort();
    found
}

fn walk(root: &Path, dir: &Path, found: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(root, &path, found);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        if text.contains("libc::SO_PEERCRED") {
            found.push(path.strip_prefix(root).unwrap().display().to_string());
        }
    }
}

#[test]
fn controller_path_is_not_taken_from_the_document() {
    let path = Path::new("/run/cm/instances/e1/run/sock0.sock");
    let poisoned = json!({
        "mode": "direct",
        "external-controller-unix": "/tmp/from-document.sock"
    });
    assert_eq!(
        attach_instance_controller(&poisoned, 20_000, path).unwrap_err(),
        ConfigError::Forbidden
    );

    let clean = json!({"mode": "direct", "ipv6": false});
    let bytes = attach_instance_controller(&clean, 20_000, path).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["external-controller-unix"], path.to_str().unwrap());
    assert_eq!(value["listeners"][0]["listen"], "127.0.0.1");
    assert_eq!(value["listeners"][0]["port"], 20_000);
    assert_eq!(value["listeners"][0]["type"], "mixed");
    assert!(!value.to_string().contains("from-document"));
}

#[test]
fn worker_rejects_geo_rules_and_malformed_rule_lists() {
    for document in [
        json!({"rules": ["GEOIP,RU,DIRECT"]}),
        json!({"rules": ["GEOSITE,private,DIRECT"]}),
        json!({"rules": ["geoip,cn,DIRECT"]}),
        json!({"sub-rules": {"nested": ["GEOSITE,cn,DIRECT"]}}),
        json!({"rules": [1]}),
        json!({"sub-rules": "nope"}),
        json!({"rules": "MATCH,DIRECT"}),
    ] {
        assert_eq!(
            reject_geo_document(&document).unwrap_err(),
            ConfigError::InvalidRule
        );
    }
    assert!(reject_geo_document(&json!({"rules": ["MATCH,DIRECT"]})).is_ok());

    let dir = TempDirGuard::new("cm-i04-geo-validate").unwrap();
    let mut core = worker(dir.path(), Path::new("/nonexistent/mihomo"));
    let bytes = serde_json::to_vec(&json!({"rules": ["GEOIP,RU,DIRECT"]})).unwrap();
    let error = core.validate(&config(bytes)).unwrap_err();
    assert_eq!(error, CoreError::InvalidConfig);
    assert!(!error.to_string().contains("GEOIP"));
    assert!(!format!("{error:?}").contains("GEOIP"));
}

#[test]
fn rejected_controller_key_does_not_keep_a_lease() {
    let dir = TempDirGuard::new("cm-i04-lease-reject").unwrap();
    let mut core = worker(dir.path(), Path::new("/nonexistent/mihomo"));
    let bytes = serde_json::to_vec(&json!({
        "mode": "direct",
        "external-controller-unix": "/tmp/from-document.sock"
    }))
    .unwrap();
    let error = core.start(&config(bytes)).unwrap_err();
    assert_eq!(error, CoreError::InvalidConfig);
    assert!(!error.to_string().contains("from-document"));
    assert!(core.leases().unwrap().is_empty());
    assert!(core.process_id().is_none());
    assert!(core.socket_path().is_none());
}

#[test]
fn exhausted_socket_pool_releases_the_port() {
    let dir = TempDirGuard::new("cm-i04-socket-exhausted").unwrap();
    let leases = dir.path().join("leases");
    fs::create_dir(&leases).unwrap();
    let registry = LeaseRegistry::create(&leases).unwrap();
    for _ in 0..256 {
        registry.allocate("other", ResourceKind::Socket).unwrap();
    }
    let mut core = worker(dir.path(), Path::new("/nonexistent/mihomo"));
    assert_eq!(
        core.start(&config(direct("warning"))).unwrap_err(),
        CoreError::Failed
    );
    assert!(
        registry
            .list()
            .unwrap()
            .iter()
            .all(|lease| lease.kind == ResourceKind::Socket)
    );
    assert!(core.process_id().is_none());
}

#[test]
fn mihomo_api_peer_check_is_not_copied() {
    assert_eq!(
        peer_sites(),
        vec!["core/mihomo/api.rs".to_owned(), "helper/auth.rs".to_owned(),]
    );
}

#[test]
fn worker_lifecycle_on_harness() {
    if std::env::var("CM_I04_LIFECYCLE_INNER").ok().as_deref() == Some("1") {
        lifecycle_inner();
        return;
    }
    let Some(mihomo) = std::env::var_os("CM_TEST_MIHOMO") else {
        eprintln!("I04.T04.c SKIPPED: CM_TEST_MIHOMO not set");
        return;
    };
    let probe = Command::new("unshare").args(["-rn", "true"]).output();
    assert!(
        probe.as_ref().is_ok_and(|output| output.status.success()),
        "user and network namespaces are required for I04.T04.c"
    );
    let dir = TempDirGuard::new("cm-i04-lifecycle").unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let exe = std::env::current_exe().unwrap();
    let output = Command::new("timeout")
        .args(["90", "unshare", "-rn", "--"])
        .arg(&exe)
        .arg("--exact")
        .arg("worker_lifecycle_on_harness")
        .arg("--nocapture")
        .env("CM_I04_LIFECYCLE_INNER", "1")
        .env("CM_TEST_MIHOMO", &mihomo)
        .env("CM_I04_LIFECYCLE_DIR", dir.path())
        .output()
        .expect("spawn lifecycle harness");
    reap_recorded_group(dir.path());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "lifecycle harness failed: status {} stdout {} stderr {}",
        output.status,
        tail(&stdout),
        tail(&stderr)
    );
    assert!(
        !stdout.contains("SKIPPED") && !stderr.contains("SKIPPED"),
        "harness skipped a core check"
    );
    assert!(
        stdout.contains("I04_LIFECYCLE_OK") || stderr.contains("I04_LIFECYCLE_OK"),
        "inner lifecycle did not finish"
    );
}

fn tail(text: &str) -> &str {
    let start = text.len().saturating_sub(2000);
    text.get(start..).unwrap_or(text)
}

fn reap_recorded_group(dir: &Path) {
    let Ok(text) = fs::read_to_string(dir.join("worker.pid")) else {
        return;
    };
    let Ok(pid) = text.trim().parse::<u32>() else {
        return;
    };
    if !group_alive(pid) {
        return;
    }
    let killed = Command::new("kill")
        .args(["-s", "KILL", "--", &format!("-{pid}")])
        .status();
    assert!(
        killed.is_ok_and(|status| status.success()),
        "leaked worker process group {pid}"
    );
}

fn lifecycle_inner() {
    let base = PathBuf::from(std::env::var_os("CM_I04_LIFECYCLE_DIR").expect("lifecycle dir"));
    let binary = PathBuf::from(std::env::var_os("CM_TEST_MIHOMO").expect("mihomo"));
    let up = Command::new("ip")
        .args(["link", "set", "lo", "up"])
        .status();
    assert!(
        up.is_ok_and(|status| status.success()),
        "lo did not come up"
    );

    let mut core = worker(&base, &binary);
    let ready = core.start(&config(direct("warning"))).expect("start");
    assert_eq!(ready.api, ApiState::ApiReady);
    let pid = core.process_id().expect("pid");
    fs::write(base.join("worker.pid"), pid.to_string()).unwrap();
    assert!(group_alive(pid));

    let socket = core.socket_path().expect("socket").to_path_buf();
    let run = base.join("state/instances/e1/run");
    assert!(socket.starts_with(&run));
    assert_eq!(
        fs::metadata(&run).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let published: Value = serde_json::from_slice(&core.config_bytes().unwrap()).unwrap();
    assert_eq!(
        published["external-controller-unix"].as_str(),
        socket.to_str()
    );

    let reloaded = core.reload(&config(direct("error"))).expect("reload");
    assert_eq!(reloaded.api, ApiState::ApiReady);
    let published = String::from_utf8(core.config_bytes().unwrap()).unwrap();
    assert!(published.contains("\"log-level\":\"error\""));
    let configs = request(
        &socket,
        euid(),
        "GET",
        "/configs",
        None,
        Duration::from_secs(2),
    )
    .expect("configs");
    assert_eq!(
        configs.get("log-level").and_then(Value::as_str),
        Some("error")
    );

    let kept = core.config_bytes().unwrap();
    let rejected = core.reload(&config(rejected_proxy())).unwrap_err();
    assert_eq!(rejected, CoreError::InvalidConfig);
    assert!(!rejected.to_string().contains("203.0.113.5"));
    assert_eq!(core.config_bytes().unwrap(), kept);
    assert_eq!(core.health().unwrap().api, ApiState::ApiReady);

    // The file has passed mihomo -t; make only the API PUT fail, then restore
    // the listening socket and verify that the old config is still published.
    let moved_socket = socket.with_extension("held");
    fs::rename(&socket, &moved_socket).unwrap();
    assert_eq!(
        core.reload(&config(direct("debug"))).unwrap_err(),
        CoreError::InvalidConfig
    );
    assert_eq!(core.config_bytes().unwrap(), kept);
    fs::rename(&moved_socket, &socket).unwrap();
    assert_eq!(core.health().unwrap().api, ApiState::ApiReady);
    let configs = request(
        &socket,
        euid(),
        "GET",
        "/configs",
        None,
        Duration::from_secs(2),
    )
    .expect("configs after failed PUT");
    assert_eq!(
        configs.get("log-level").and_then(Value::as_str),
        Some("error")
    );

    let geo = serde_json::to_vec(&json!({
        "mode": "direct",
        "rules": ["GEOIP,RU,DIRECT"]
    }))
    .unwrap();
    assert_eq!(
        core.reload(&config(geo)).unwrap_err(),
        CoreError::InvalidConfig
    );
    assert_eq!(core.config_bytes().unwrap(), kept);
    assert_eq!(core.health().unwrap().api, ApiState::ApiReady);

    core.stop().unwrap();
    assert!(!group_alive(pid));
    assert!(!socket.exists());
    assert!(core.leases().unwrap().is_empty());
    assert!(core.process_id().is_none());
    assert_eq!(core.health().unwrap(), CoreReadiness::DOWN);

    core.start(&config(direct("warning"))).expect("restart");
    let pid = core.process_id().expect("restart pid");
    fs::write(base.join("worker.pid"), pid.to_string()).unwrap();
    let held = core.leases().unwrap();
    assert!(held.len() >= 2);
    let killed = Command::new("kill")
        .args(["-s", "KILL", "--", &pid.to_string()])
        .status();
    assert!(killed.is_ok_and(|status| status.success()));
    let mut down = false;
    for _ in 0..50 {
        if core.health().unwrap() == CoreReadiness::DOWN {
            down = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(down, "kill -9 did not surface Down");
    // Down can come from the API before the killed group is reaped; restart needs it gone.
    let mut gone = false;
    for _ in 0..50 {
        if !core.group_alive() {
            gone = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(gone, "killed process group is still alive");
    let dropped = core.reclaim_if_absent(&[]).unwrap();
    assert_eq!(dropped, held.len());
    assert!(core.leases().unwrap().is_empty());
    core.start(&config(direct("warning")))
        .expect("restart after crash");
    let restarted_pid = core.process_id().expect("restarted pid");
    fs::write(base.join("worker.pid"), restarted_pid.to_string()).unwrap();
    assert_eq!(core.leases().unwrap().len(), 2);
    core.stop().unwrap();
    assert!(!group_alive(pid));
    assert!(!group_alive(restarted_pid));
    assert!(core.leases().unwrap().is_empty());
    assert!(core.socket_path().is_none());
    eprintln!("I04_LIFECYCLE_OK");
}
