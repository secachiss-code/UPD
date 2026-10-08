//! I04.T04.e: N app workers versus one mihomo with N listeners.
//!
//! A measurement, not a gate check. Run on demand:
//!
//! ```text
//! CM_TEST_MIHOMO=$HOME/.cache/cm-cores/mihomo/mihomo CM_I04_TOPOLOGY_OUT=/path/out.json \
//!   cargo test --offline --locked --target x86_64-unknown-linux-musl \
//!   --test audit_i04_topology -- --ignored --nocapture
//! ```
//!
//! Everything runs inside `unshare -rn`. Each tunnel has its own local upstream
//! (an HTTP CONNECT proxy) so traffic of tunnel `i` only leaves through node `i`.

use cm::common::contract_fixtures::TempDirGuard;
use cm::common::sys::euid;
use cm::core::mihomo::MihomoWorker;
use cm::core::mihomo::api::request;
use cm::core::{CoreAdapter, CoreConfig, InstanceId, InstanceRoot, ResourceKind};
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const SIZES: [usize; 3] = [1, 5, 20];
const UPSTREAM_BASE: u16 = 19100;
const SINGLE_BASE: u16 = 21000;
const BULK: usize = 4 * 1024 * 1024;
const SLOW_BYTES: usize = 60;
const SLOW_STEP: Duration = Duration::from_millis(50);
const IDLE_WINDOW: Duration = Duration::from_secs(5);

#[test]
#[ignore = "measurement for I04.T04.e; run with --ignored"]
fn worker_topology_measurement() {
    if std::env::var("CM_I04_TOPOLOGY_INNER").ok().as_deref() == Some("1") {
        inner();
        return;
    }
    let mihomo = std::env::var_os("CM_TEST_MIHOMO").expect("CM_TEST_MIHOMO is required");
    let dir = TempDirGuard::new("cm-i04-topology").unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let exe = std::env::current_exe().unwrap();
    let output = Command::new("timeout")
        .args(["900", "unshare", "-rn", "--"])
        .arg(&exe)
        .args([
            "--exact",
            "worker_topology_measurement",
            "--ignored",
            "--nocapture",
        ])
        .env("CM_I04_TOPOLOGY_INNER", "1")
        .env("CM_TEST_MIHOMO", &mihomo)
        .env("CM_I04_TOPOLOGY_DIR", dir.path())
        .output()
        .expect("spawn topology harness");
    reap(dir.path());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "topology harness failed: {}\n{}\n{}",
        output.status,
        stdout,
        stderr
    );
    let rows: Vec<Value> = stdout
        .lines()
        .chain(stderr.lines())
        .filter_map(|line| line.strip_prefix("TOPOLOGY "))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), SIZES.len() * 2, "{stdout}\n{stderr}");
    let report = json!({
        "mihomo": mihomo.to_string_lossy(),
        "clock_ticks_per_second": clock_ticks(),
        "rows": rows,
    });
    let text = serde_json::to_string_pretty(&report).unwrap();
    println!("{text}");
    if let Some(out) = std::env::var_os("CM_I04_TOPOLOGY_OUT") {
        fs::write(out, format!("{text}\n")).unwrap();
    }
}

fn inner() {
    let base = PathBuf::from(std::env::var_os("CM_I04_TOPOLOGY_DIR").unwrap());
    let binary = PathBuf::from(std::env::var_os("CM_TEST_MIHOMO").unwrap());
    let up = Command::new("ip")
        .args(["link", "set", "lo", "up"])
        .status();
    assert!(
        up.is_ok_and(|status| status.success()),
        "lo did not come up"
    );
    let largest = SIZES.iter().copied().max().unwrap();
    for index in 0..largest {
        serve(UPSTREAM_BASE + index as u16);
    }
    for size in SIZES {
        let row = measure_workers(&base, &binary, size);
        println!("TOPOLOGY {row}");
        let row = measure_single(&base, &binary, size);
        println!("TOPOLOGY {row}");
    }
}

// ---------- variant: one worker per tunnel ----------

fn worker_document(index: usize, log_level: &str) -> CoreConfig {
    let name = format!("node{index}");
    let bytes = serde_json::to_vec(&json!({
        "mode": "rule",
        "log-level": log_level,
        "find-process-mode": "off",
        "ipv6": false,
        "proxies": [{
            "name": name,
            "type": "http",
            "server": "127.0.0.1",
            "port": UPSTREAM_BASE + index as u16
        }],
        "proxy-groups": [{"name": "Proxy", "type": "select", "proxies": [name]}],
        "rules": ["MATCH,Proxy"]
    }))
    .unwrap();
    CoreConfig::from_bytes(bytes).unwrap()
}

fn measure_workers(base: &Path, binary: &Path, size: usize) -> Value {
    let root = base.join(format!("workers-{size}"));
    fs::create_dir_all(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let mut workers = Vec::with_capacity(size);
    let started = Instant::now();
    for index in 0..size {
        let id = InstanceId::new(&format!("t{index}")).unwrap();
        let mut worker = MihomoWorker::new(
            id,
            InstanceRoot::new(root.join("state")),
            root.join("leases"),
            binary,
        );
        worker
            .start(&worker_document(index, "warning"))
            .unwrap_or_else(|error| panic!("worker {index} start: {error:?}"));
        record(
            base,
            &format!("w{size}-{index}"),
            worker.process_id().unwrap(),
        );
        workers.push(worker);
    }
    let start_ms = started.elapsed().as_millis();
    let ports: Vec<u16> = workers
        .iter()
        .enumerate()
        .map(|(index, worker)| leased_port(worker, &format!("t{index}")))
        .collect();
    let distinct: std::collections::BTreeSet<u16> = ports.iter().copied().collect();
    assert_eq!(distinct.len(), size, "worker ports overlap: {ports:?}");
    let pids: Vec<u32> = workers.iter().map(|w| w.process_id().unwrap()).collect();
    let mut row = common_metrics("workers", size, start_ms, &pids, &ports);

    // Reload of tunnel 0 while long connections run on tunnel 0 and tunnel 1.
    let own = slow_fetch(ports[0]);
    let neighbour = (size > 1).then(|| slow_fetch(ports[1]));
    std::thread::sleep(Duration::from_millis(600));
    let reload_started = Instant::now();
    workers[0]
        .reload(&worker_document(0, "error"))
        .expect("worker reload");
    let reload_ms = reload_started.elapsed().as_millis();
    let own_bytes = own.join().unwrap();
    let neighbour_bytes = neighbour.map(|handle| handle.join().unwrap());
    row["reload_ms"] = json!(reload_ms);
    row["reload_own_connection_bytes"] = json!(own_bytes);
    row["reload_neighbour_connection_bytes"] = json!(neighbour_bytes);
    row["reload_neighbour_listener_ok"] =
        json!((size > 1).then(|| fetch(ports[1], "bulk").is_ok_and(|n| n == BULK)));

    // kill -9 of tunnel 0: how many other tunnels still carry traffic.
    kill(pids[0]);
    std::thread::sleep(Duration::from_millis(300));
    row["after_kill_other_tunnels_ok"] = json!(count_ok(&ports[1..]));
    row["after_kill_other_tunnels_total"] = json!(size - 1);
    for mut worker in workers {
        match worker.stop() {
            Ok(()) | Err(_) => {}
        }
    }
    row
}

/// The registry is shared by all workers, so the lease is picked by holder.
fn leased_port(worker: &MihomoWorker, holder: &str) -> u16 {
    worker
        .leases()
        .unwrap()
        .into_iter()
        .find(|lease| lease.kind == ResourceKind::Port && lease.holder == holder)
        .and_then(|lease| lease.value.parse().ok())
        .unwrap()
}

// ---------- variant: one core, N listeners ----------

fn single_document(size: usize, socket: &Path, log_level: &str) -> Vec<u8> {
    let proxies: Vec<Value> = (0..size)
        .map(|index| {
            json!({
                "name": format!("node{index}"),
                "type": "http",
                "server": "127.0.0.1",
                "port": UPSTREAM_BASE + index as u16
            })
        })
        .collect();
    let listeners: Vec<Value> = (0..size)
        .map(|index| {
            json!({
                "name": format!("in{index}"),
                "type": "mixed",
                "listen": "127.0.0.1",
                "port": SINGLE_BASE + index as u16,
                "proxy": format!("node{index}")
            })
        })
        .collect();
    serde_json::to_vec(&json!({
        "mode": "rule",
        "log-level": log_level,
        "find-process-mode": "off",
        "ipv6": false,
        "external-controller-unix": socket,
        "proxies": proxies,
        "listeners": listeners,
        "rules": ["MATCH,DIRECT"]
    }))
    .unwrap()
}

fn measure_single(base: &Path, binary: &Path, size: usize) -> Value {
    let home = base.join(format!("single-{size}"));
    fs::create_dir_all(&home).unwrap();
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
    let socket = home.join("api.sock");
    let config = home.join("config.json");
    fs::write(&config, single_document(size, &socket, "warning")).unwrap();
    let ports: Vec<u16> = (0..size).map(|index| SINGLE_BASE + index as u16).collect();
    let started = Instant::now();
    let child = spawn_core(binary, &home, &config);
    record(base, &format!("s{size}"), child.id());
    wait_api(&socket);
    for port in &ports {
        wait_port(*port);
    }
    let start_ms = started.elapsed().as_millis();
    let pid = child.id();
    let mut row = common_metrics("single", size, start_ms, &[pid], &ports);

    let own = slow_fetch(ports[0]);
    let neighbour = (size > 1).then(|| slow_fetch(ports[1]));
    std::thread::sleep(Duration::from_millis(600));
    fs::write(&config, single_document(size, &socket, "error")).unwrap();
    let reload_started = Instant::now();
    request(
        &socket,
        euid(),
        "PUT",
        "/configs?force=true",
        Some(json!({ "path": config })),
        Duration::from_secs(8),
    )
    .expect("single reload");
    for port in &ports {
        wait_port(*port);
    }
    let reload_ms = reload_started.elapsed().as_millis();
    let own_bytes = own.join().unwrap();
    let neighbour_bytes = neighbour.map(|handle| handle.join().unwrap());
    row["reload_ms"] = json!(reload_ms);
    row["reload_own_connection_bytes"] = json!(own_bytes);
    row["reload_neighbour_connection_bytes"] = json!(neighbour_bytes);
    row["reload_neighbour_listener_ok"] =
        json!((size > 1).then(|| fetch(ports[1], "bulk").is_ok_and(|n| n == BULK)));

    kill(pid);
    std::thread::sleep(Duration::from_millis(300));
    row["after_kill_other_tunnels_ok"] = json!(count_ok(&ports[1..]));
    row["after_kill_other_tunnels_total"] = json!(size - 1);
    let mut child = child;
    match child.wait() {
        Ok(_) | Err(_) => {}
    }
    row
}

fn spawn_core(binary: &Path, home: &Path, config: &Path) -> Child {
    let mut command = Command::new(binary);
    command
        .arg("-d")
        .arg(home)
        .arg("-f")
        .arg(config)
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
    command.spawn().expect("single mihomo")
}

fn wait_api(socket: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if socket.exists()
            && request(
                socket,
                euid(),
                "GET",
                "/version",
                None,
                Duration::from_millis(400),
            )
            .is_ok()
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("single core api did not answer");
}

fn wait_port(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("listener {port} did not open");
}

// ---------- shared measurements ----------

fn common_metrics(
    variant: &str,
    size: usize,
    start_ms: u128,
    pids: &[u32],
    ports: &[u16],
) -> Value {
    std::thread::sleep(Duration::from_secs(2));
    let (rss_idle, pss_idle) = memory(pids);
    let ticks_before = ticks(pids);
    std::thread::sleep(IDLE_WINDOW);
    let idle_ticks = ticks(pids) - ticks_before;
    let ticks_before = ticks(pids);
    let load_started = Instant::now();
    for port in ports {
        let got = fetch(*port, "bulk").expect("bulk fetch");
        assert_eq!(got, BULK, "tunnel {port} body");
    }
    let load_ms = load_started.elapsed().as_millis();
    let load_ticks = ticks(pids) - ticks_before;
    let (rss_load, pss_load) = memory(pids);
    let per_second = clock_ticks() as f64;
    let megabytes = (BULK * ports.len()) as f64 / (1024.0 * 1024.0);
    json!({
        "variant": variant,
        "tunnels": size,
        "processes": pids.len(),
        "start_ms": start_ms,
        "rss_idle_kib": rss_idle,
        "pss_idle_kib": pss_idle,
        "rss_after_load_kib": rss_load,
        "pss_after_load_kib": pss_load,
        "idle_cpu_percent": round(idle_ticks as f64 / per_second / IDLE_WINDOW.as_secs_f64() * 100.0),
        "load_megabytes": megabytes,
        "load_ms": load_ms,
        "load_cpu_ms_per_megabyte": round(load_ticks as f64 / per_second * 1000.0 / megabytes),
    })
}

fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn memory(pids: &[u32]) -> (u64, u64) {
    let mut rss = 0;
    let mut pss = 0;
    for pid in pids {
        let text = fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).unwrap();
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            let key = parts.next().unwrap_or("");
            let value: u64 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            match key {
                "Rss:" => rss += value,
                "Pss:" => pss += value,
                _ => {}
            }
        }
    }
    (rss, pss)
}

fn ticks(pids: &[u32]) -> u64 {
    pids.iter()
        .map(|pid| {
            let text = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
            let rest = &text[text.rfind(')').unwrap() + 2..];
            let fields: Vec<&str> = rest.split_whitespace().collect();
            // utime and stime are fields 14 and 15; `rest` starts at field 3.
            fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
        })
        .sum()
}

fn clock_ticks() -> i64 {
    // SAFETY: sysconf only reads a configuration value.
    unsafe { libc::sysconf(libc::_SC_CLK_TCK) }
}

fn count_ok(ports: &[u16]) -> usize {
    ports
        .iter()
        .filter(|port| fetch(**port, "small").is_ok_and(|n| n == 64 * 1024))
        .count()
}

/// Body bytes received through the proxy on `port` for `/bulk`, `/small` or `/slow`.
fn fetch(port: u16, path: &str) -> Result<usize, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| error.to_string())?;
    let request = format!(
        "GET http://203.0.113.10/{path} HTTP/1.1\r\nHost: 203.0.113.10\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| error.to_string())?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 65536];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    let head = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| format!("no response head ({} bytes)", raw.len()))?;
    Ok(raw.len() - head - 4)
}

fn slow_fetch(port: u16) -> std::thread::JoinHandle<usize> {
    std::thread::spawn(move || fetch(port, "slow").unwrap_or(0))
}

/// HTTP CONNECT upstream: `/bulk` 4 MiB, `/small` 64 KiB, `/slow` 60 bytes over 3 s.
fn serve(port: u16) {
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    std::thread::spawn(move || {
        while let Ok((socket, _)) = listener.accept() {
            std::thread::spawn(move || {
                let _ = handle(socket);
            });
        }
    });
}

fn read_head(socket: &mut TcpStream) -> std::io::Result<String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if socket.read(&mut byte)? == 0 || buf.len() > 8192 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buf.push(byte[0]);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn handle(mut socket: TcpStream) -> std::io::Result<()> {
    let mut head = read_head(&mut socket)?;
    if head.starts_with("CONNECT") {
        socket.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
        head = read_head(&mut socket)?;
    }
    let path = head.split_whitespace().nth(1).unwrap_or("");
    if path.ends_with("/slow") {
        socket.write_all(
            format!("HTTP/1.1 200 OK\r\nContent-Length: {SLOW_BYTES}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )?;
        for _ in 0..SLOW_BYTES {
            socket.write_all(b"s")?;
            std::thread::sleep(SLOW_STEP);
        }
        return Ok(());
    }
    let size = if path.ends_with("/bulk") {
        BULK
    } else {
        64 * 1024
    };
    socket.write_all(
        format!("HTTP/1.1 200 OK\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n")
            .as_bytes(),
    )?;
    socket.write_all(&vec![b'z'; size])
}

fn kill(pid: u32) {
    let status = Command::new("kill")
        .args(["-s", "KILL", "--", &pid.to_string()])
        .status();
    assert!(status.is_ok_and(|status| status.success()), "kill {pid}");
}

fn record(dir: &Path, name: &str, pid: u32) {
    fs::write(dir.join(format!("{name}.pid")), pid.to_string()).unwrap();
}

fn reap(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("pid") {
            continue;
        }
        let Ok(pid) = fs::read_to_string(&path)
            .unwrap_or_default()
            .trim()
            .parse::<i32>()
        else {
            continue;
        };
        if pid <= 0 {
            continue;
        }
        for target in [format!("-{pid}"), pid.to_string()] {
            let _ = Command::new("kill")
                .args(["-s", "KILL", "--", &target])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}
