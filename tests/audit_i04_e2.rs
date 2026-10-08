//! I04.T04.d and I04.T05.a: worker traffic counters and two workers beside host-legacy.
//!
//! The process test runs inside `unshare -rn`. Counters are the core API totals.
//! They are compared with nft on each worker's own remote, not added to a TUN device.

use cm::common::contract_fixtures::TempDirGuard;
use cm::common::sys::euid;
use cm::core::mihomo::MihomoWorker;
use cm::core::mihomo::api::request;
use cm::core::{ApiState, CoreAdapter, CoreConfig, InstanceId, InstanceRoot, ResourceKind};
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const BODY_A: usize = 256 * 1024;
const BODY_B: usize = 512 * 1024;

fn worker_document(name: &str, port: u16) -> CoreConfig {
    let bytes = serde_json::to_vec(&json!({
        "mode": "rule",
        "log-level": "warning",
        "find-process-mode": "off",
        "ipv6": false,
        "proxies": [{
            "name": name,
            "type": "http",
            "server": "127.0.0.1",
            "port": port
        }],
        "proxy-groups": [{
            "name": "Proxy",
            "type": "select",
            "proxies": [name]
        }],
        "rules": [format!("MATCH,{name}")]
    }))
    .unwrap();
    CoreConfig::from_bytes(bytes).unwrap()
}

fn serve(port: u16, body: usize) {
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    std::thread::spawn(move || {
        loop {
            let Ok((mut socket, _)) = listener.accept() else {
                return;
            };
            std::thread::spawn(move || {
                let mut buf = vec![0u8; 4096];
                let mut got = 0usize;
                while !buf[..got].windows(4).any(|mark| mark == b"\r\n\r\n") {
                    let Ok(n) = socket.read(&mut buf[got..]) else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    got += n;
                    if got == buf.len() {
                        return;
                    }
                }
                let head = String::from_utf8_lossy(&buf[..got]);
                if head.starts_with("CONNECT") {
                    if socket
                        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                        .is_err()
                    {
                        return;
                    }
                    let mut inner = [0u8; 2048];
                    if socket.read(&mut inner).is_err() {
                        return;
                    }
                }
                let payload = vec![b'z'; body];
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let header_written = socket.write_all(header.as_bytes());
                let body_written = header_written.and_then(|()| socket.write_all(&payload));
                match body_written {
                    Ok(()) | Err(_) => {}
                }
            });
        }
    });
}

fn pull(port: u16, minimum: usize) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    stream
        .write_all(
            b"GET http://203.0.113.10/ HTTP/1.1\r\nHost: 203.0.113.10\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
    let mut got = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(error) => panic!("proxy read: {error}"),
        }
        if got.len() >= minimum {
            break;
        }
    }
    let preview = String::from_utf8_lossy(&got[..got.len().min(180)]);
    assert!(
        got.windows(64)
            .any(|window| window.iter().all(|byte| *byte == b'z')),
        "worker port {port} did not return the remote body ({} bytes): {preview}",
        got.len()
    );
}

fn within_five_percent(api: u64, captured: u64) -> bool {
    if captured == 0 {
        return false;
    }
    let diff = u128::from(api.abs_diff(captured));
    diff * 100 <= u128::from(captured) * 5
}

fn counter_bytes(name: &str) -> u64 {
    let output = Command::new("nft")
        .args(["-j", "list", "counter", "inet", "cme2", name])
        .output()
        .expect("nft");
    assert!(
        output.status.success(),
        "nft counter {name}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    value["nftables"]
        .as_array()
        .and_then(|items| {
            items.iter().find_map(|item| {
                item.get("counter")
                    .and_then(|counter| counter.get("bytes"))
                    .and_then(Value::as_u64)
            })
        })
        .unwrap_or_else(|| panic!("nft counter {name} has no bytes: {value}"))
}

fn nft_rules() {
    let commands: &[&[&str]] = &[
        &["add", "table", "inet", "cme2"],
        &[
            "add",
            "chain",
            "inet",
            "cme2",
            "out",
            "{ type filter hook output priority 0 ; policy accept ; }",
        ],
        &["add", "counter", "inet", "cme2", "edge-a"],
        &["add", "counter", "inet", "cme2", "edge-b"],
        &[
            "add",
            "rule",
            "inet",
            "cme2",
            "out",
            "ip",
            "daddr",
            "127.0.0.1",
            "tcp",
            "dport",
            "19081",
            "counter",
            "name",
            "edge-a",
        ],
        &[
            "add",
            "rule",
            "inet",
            "cme2",
            "out",
            "ip",
            "saddr",
            "127.0.0.1",
            "tcp",
            "sport",
            "19081",
            "counter",
            "name",
            "edge-a",
        ],
        &[
            "add",
            "rule",
            "inet",
            "cme2",
            "out",
            "ip",
            "daddr",
            "127.0.0.1",
            "tcp",
            "dport",
            "19082",
            "counter",
            "name",
            "edge-b",
        ],
        &[
            "add",
            "rule",
            "inet",
            "cme2",
            "out",
            "ip",
            "saddr",
            "127.0.0.1",
            "tcp",
            "sport",
            "19082",
            "counter",
            "name",
            "edge-b",
        ],
    ];
    for args in commands {
        let output = Command::new("nft").args(*args).output().expect("nft");
        assert!(
            output.status.success(),
            "nft {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn leased_port(worker: &MihomoWorker, holder: &str) -> u16 {
    worker
        .leases()
        .unwrap()
        .into_iter()
        .find(|lease| lease.holder == holder && lease.kind == ResourceKind::Port)
        .unwrap()
        .value
        .parse()
        .unwrap()
}

fn record(dir: &Path, name: &str, pid: u32) {
    fs::write(dir.join(name), pid.to_string()).unwrap();
}

fn spawn_legacy(binary: &Path, dir: &Path) -> std::process::Child {
    let home = dir.join("legacy");
    fs::create_dir_all(&home).unwrap();
    let socket = dir.join("legacy.sock");
    let config = home.join("config.json");
    fs::write(
        &config,
        serde_json::to_vec(&json!({
            "mixed-port": 17991,
            "allow-lan": false,
            "bind-address": "127.0.0.1",
            "mode": "direct",
            "log-level": "warning",
            "find-process-mode": "off",
            "ipv6": false,
            "external-controller-unix": socket
        }))
        .unwrap(),
    )
    .unwrap();
    let mut command = Command::new(binary);
    command
        .args(["-d"])
        .arg(&home)
        .arg("-f")
        .arg(&config)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid runs in the forked child before exec and only changes that child's session.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn().expect("legacy mihomo")
}

fn legacy_version(socket: &Path) -> Value {
    request(
        socket,
        euid(),
        "GET",
        "/version",
        None,
        Duration::from_secs(2),
    )
    .expect("legacy version")
}

#[test]
fn two_workers_and_legacy_on_harness() {
    if std::env::var("CM_I04_E2_INNER").ok().as_deref() == Some("1") {
        harness_inner();
        return;
    }
    let Some(mihomo) = std::env::var_os("CM_TEST_MIHOMO") else {
        eprintln!("I04.T04.d SKIPPED: CM_TEST_MIHOMO not set");
        return;
    };
    let probe = Command::new("unshare").args(["-rn", "true"]).output();
    assert!(
        probe.as_ref().is_ok_and(|output| output.status.success()),
        "user and network namespaces are required for I04.T05.a"
    );
    let dir = TempDirGuard::new("cm-i04-e2").unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let exe = std::env::current_exe().unwrap();
    let output = Command::new("timeout")
        .args(["120", "unshare", "-rn", "--"])
        .arg(&exe)
        .arg("--exact")
        .arg("two_workers_and_legacy_on_harness")
        .arg("--nocapture")
        .env("CM_I04_E2_INNER", "1")
        .env("CM_TEST_MIHOMO", &mihomo)
        .env("CM_I04_E2_DIR", dir.path())
        .output()
        .expect("spawn e2 harness");
    reap(dir.path());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "e2 harness failed: status {} stdout {} stderr {}",
        output.status,
        tail(&stdout),
        tail(&stderr)
    );
    assert!(
        !stdout.contains("SKIPPED") && !stderr.contains("SKIPPED"),
        "harness skipped a core check"
    );
    assert!(
        stdout.contains("I04_E2_OK") || stderr.contains("I04_E2_OK"),
        "inner harness did not finish"
    );
}

fn tail(text: &str) -> &str {
    let start = text.len().saturating_sub(4000);
    text.get(start..).unwrap_or(text)
}

fn reap(dir: &Path) {
    for name in ["worker-a.pid", "worker-b.pid", "legacy.pid"] {
        let Ok(text) = fs::read_to_string(dir.join(name)) else {
            continue;
        };
        let Ok(pid) = text.trim().parse::<i32>() else {
            continue;
        };
        if pid <= 0 {
            continue;
        }
        for target in [format!("-{pid}"), pid.to_string()] {
            let status = Command::new("kill")
                .args(["-s", "KILL", "--", &target])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            match status {
                Ok(_) | Err(_) => {}
            }
        }
    }
}

fn harness_inner() {
    let base = PathBuf::from(std::env::var_os("CM_I04_E2_DIR").expect("e2 dir"));
    let binary = PathBuf::from(std::env::var_os("CM_TEST_MIHOMO").expect("mihomo"));
    let up = Command::new("ip")
        .args(["link", "set", "lo", "up"])
        .status();
    assert!(
        up.is_ok_and(|status| status.success()),
        "lo did not come up"
    );
    serve(19081, BODY_A);
    serve(19082, BODY_B);
    nft_rules();

    let mut legacy = spawn_legacy(&binary, &base);
    record(&base, "legacy.pid", legacy.id());
    let legacy_socket = base.join("legacy.sock");
    let mut legacy_up = false;
    for _ in 0..50 {
        if legacy_socket.exists()
            && request(
                &legacy_socket,
                euid(),
                "GET",
                "/version",
                None,
                Duration::from_millis(200),
            )
            .is_ok()
        {
            legacy_up = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(legacy_up, "host-legacy api did not answer");

    let leases = base.join("leases");
    let mut first = MihomoWorker::new(
        InstanceId::new("wa").unwrap(),
        InstanceRoot::new(base.join("state")),
        &leases,
        &binary,
    );
    let mut second = MihomoWorker::new(
        InstanceId::new("wb").unwrap(),
        InstanceRoot::new(base.join("state")),
        &leases,
        &binary,
    );
    assert_eq!(
        first.start(&worker_document("edge-a", 19081)).unwrap().api,
        ApiState::ApiReady
    );
    record(&base, "worker-a.pid", first.process_id().unwrap());
    assert_eq!(
        second.start(&worker_document("edge-b", 19082)).unwrap().api,
        ApiState::ApiReady
    );
    record(&base, "worker-b.pid", second.process_id().unwrap());

    let held = first.leases().unwrap();
    let mut seen = Vec::new();
    for lease in &held {
        let key = (lease.kind, lease.value.clone());
        assert!(
            !seen.contains(&key),
            "overlapping lease {} {}",
            lease.holder,
            lease.value
        );
        seen.push(key);
    }
    let ports: Vec<_> = held
        .iter()
        .filter(|lease| lease.kind == ResourceKind::Port)
        .map(|lease| lease.value.as_str())
        .collect();
    let sockets: Vec<_> = held
        .iter()
        .filter(|lease| lease.kind == ResourceKind::Socket)
        .map(|lease| lease.value.as_str())
        .collect();
    assert_eq!(ports.len(), 2, "{ports:?}");
    assert_eq!(sockets.len(), 2, "{sockets:?}");
    assert!(ports[0] != ports[1]);

    let port_a = leased_port(&first, "wa");
    let port_b = leased_port(&second, "wb");
    pull(port_a, BODY_A);
    pull(port_b, BODY_B);
    let stats_a = first.statistics().unwrap();
    let stats_b = second.statistics().unwrap();
    let nft_a = counter_bytes("edge-a");
    let nft_b = counter_bytes("edge-b");
    let api_a = stats_a.upload_bytes.saturating_add(stats_a.download_bytes);
    let api_b = stats_b.upload_bytes.saturating_add(stats_b.download_bytes);
    assert!(
        within_five_percent(api_a, nft_a),
        "worker A api {api_a} (up {} down {}) nft {nft_a}",
        stats_a.upload_bytes,
        stats_a.download_bytes
    );
    assert!(
        within_five_percent(api_b, nft_b),
        "worker B api {api_b} (up {} down {}) nft {nft_b}",
        stats_b.upload_bytes,
        stats_b.download_bytes
    );
    assert!(
        !within_five_percent(api_a, nft_b),
        "worker A matched the other remote: api {api_a} nft-b {nft_b}"
    );
    assert_ne!(nft_a, nft_b);

    let before = second.statistics().unwrap();
    let version = legacy_version(&legacy_socket);
    first.stop().unwrap();
    assert_eq!(second.health().unwrap().api, ApiState::ApiReady);
    assert_eq!(second.statistics().unwrap(), before);
    assert_eq!(legacy_version(&legacy_socket), version);
    let remaining = second.leases().unwrap();
    assert!(remaining.iter().any(|lease| lease.holder == "wb"));
    assert!(remaining.iter().all(|lease| lease.holder != "wa"));

    second.stop().unwrap();
    legacy.kill().expect("legacy stop");
    legacy.wait().expect("legacy wait");
    eprintln!("I04_E2_OK");
}
