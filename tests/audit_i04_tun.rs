//! I04.T03.b: TUN ownership on a user and network namespace.
//!
//! Variant A is a persistent device created before mihomo, owned by uid 1.
//! Variant B lets mihomo create the device. The chosen variant is A.
//! Without `CM_TEST_MIHOMO` the test prints SKIPPED.

use cm::common::contract_fixtures::TempDirGuard;
use cm::common::sys::euid;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn subuid(name: &str) -> u32 {
    let text = fs::read_to_string("/etc/subuid").unwrap_or_default();
    for line in text.lines() {
        let mut parts = line.split(':');
        if parts.next() == Some(name)
            && let Some(start) = parts.next()
            && let Ok(start) = start.parse::<u32>()
        {
            return start;
        }
    }
    panic!("no /etc/subuid range for {name}");
}

fn login() -> String {
    if let Ok(name) = std::env::var("USER")
        && !name.is_empty()
    {
        return name;
    }
    let output = Command::new("id").arg("-un").output().unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn tun_ownership_on_harness() {
    if std::env::var("CM_I04_TUN_INNER").ok().as_deref() == Some("1") {
        harness_inner();
        return;
    }
    let Some(mihomo) = std::env::var_os("CM_TEST_MIHOMO") else {
        eprintln!("I04.T03.b SKIPPED: CM_TEST_MIHOMO not set");
        return;
    };
    let uid = euid();
    let sub = subuid(&login());
    let dir = TempDirGuard::new("cm-i04-tun").unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let exe = std::env::current_exe().unwrap();
    let output = Command::new("timeout")
        .args(["40", "unshare", "--user", "--net"])
        .args([
            format!("--map-users=0:{uid}:1"),
            format!("--map-users=1:{sub}:1"),
            format!("--map-groups=0:{uid}:1"),
            format!("--map-groups=1:{sub}:1"),
            "--".to_owned(),
        ])
        .arg(&exe)
        .arg("--exact")
        .arg("tun_ownership_on_harness")
        .arg("--nocapture")
        .env("CM_I04_TUN_INNER", "1")
        .env("CM_TEST_MIHOMO", &mihomo)
        .env("CM_I04_TUN_DIR", dir.path())
        .output()
        .expect("spawn tun harness");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "tun harness failed: status {} stdout {} stderr {}",
        output.status,
        tail(&stdout),
        tail(&stderr)
    );
    assert!(!stdout.contains("SKIPPED") && !stderr.contains("SKIPPED"));
    assert!(stdout.contains("I04_TUN_OK") || stderr.contains("I04_TUN_OK"));
    for line in stdout.lines().chain(stderr.lines()) {
        if line.starts_with("variant_") || line.contains("I04_TUN_OK") {
            eprintln!("{line}");
        }
    }
}

fn tail(text: &str) -> &str {
    let start = text.len().saturating_sub(2500);
    text.get(start..).unwrap_or(text)
}

fn run(args: &[&str]) -> String {
    let output = Command::new(args[0])
        .args(&args[1..])
        .output()
        .unwrap_or_else(|error| panic!("{}: {error}", args[0]));
    assert!(
        output.status.success(),
        "{} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn cap_eff(pid: u32) -> u64 {
    let text = fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    let line = text
        .lines()
        .find(|line| line.starts_with("CapEff:"))
        .unwrap();
    let hex = line.split_whitespace().nth(1).unwrap();
    u64::from_str_radix(hex, 16).unwrap()
}

fn write_config(path: &Path, device: &str, mtu: u16) {
    let body = format!(
        r#"{{"mode":"direct","log-level":"info","find-process-mode":"off","ipv6":true,"tun":{{"enable":true,"device":"{device}","stack":"system","auto-route":false,"auto-redirect":false,"mtu":{mtu},"inet4-address":["172.19.0.1/30"],"inet6-address":["fd00:c::1/64"]}}}}"#
    );
    fs::write(path, body).unwrap();
}

fn wait_log(path: &Path, pid: u32) -> String {
    let mut log = String::new();
    for _ in 0..50 {
        log = fs::read_to_string(path).unwrap_or_default();
        if log.contains("Tun adapter listening") || !still_alive(pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    log
}

fn still_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-s", "0", "--", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn harness_inner() {
    let base = PathBuf::from(std::env::var_os("CM_I04_TUN_DIR").expect("tun dir"));
    fs::set_permissions(&base, fs::Permissions::from_mode(0o711)).unwrap();
    let binary = PathBuf::from(std::env::var_os("CM_TEST_MIHOMO").expect("mihomo"));
    run(&["ip", "link", "set", "lo", "up"]);

    let core_dir = base.join("core");
    fs::create_dir(&core_dir).unwrap();
    let core_config = core_dir.join("config.json");
    write_config(&core_config, "cmtunb", 1400);
    let core_log_path = core_dir.join("log");
    let core_log_file = fs::File::create(&core_log_path).unwrap();
    let mut core = Command::new(&binary)
        .arg("-d")
        .arg(&core_dir)
        .arg("-f")
        .arg(&core_config)
        .stdin(Stdio::null())
        .stdout(core_log_file.try_clone().unwrap())
        .stderr(core_log_file)
        .spawn()
        .unwrap();
    let core_log = wait_log(&core_log_path, core.id());
    assert!(
        core_log.contains("cmtunb([198.18.0.1/30],[])"),
        "core-created tun log: {core_log}"
    );
    assert!(
        core_log.contains("mtu: 1400") && core_log.contains("auto route: false"),
        "{core_log}"
    );
    let link = run(&["ip", "-d", "link", "show", "cmtunb"]);
    assert!(link.contains("mtu 1400"), "{link}");
    assert!(link.contains("persist off"), "{link}");
    let addr6 = run(&["ip", "-6", "addr", "show", "cmtunb"]);
    assert!(
        !addr6.contains("scope global"),
        "core-created tun gained a global ipv6 address: {addr6}"
    );
    let caps = cap_eff(core.id());
    eprintln!("variant_b_cap_eff={caps:016x}");
    assert!(caps & (1 << 12) != 0, "core lacks CAP_NET_ADMIN: {caps:x}");
    let routes = run(&["ip", "route"]);
    assert!(
        !routes.lines().any(|line| line.starts_with("default")),
        "{routes}"
    );
    core.kill().expect("stop core-created tun");
    core.wait().expect("reap core-created tun");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let gone = Command::new("ip")
        .args(["link", "show", "cmtunb"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    assert!(!gone, "core-created tun survived kill -9");

    let owned = base.join("owned");
    fs::create_dir(&owned).unwrap();
    fs::set_permissions(&owned, fs::Permissions::from_mode(0o777)).unwrap();
    let script = r#"
import os, ctypes, ctypes.util, struct
libc = ctypes.CDLL(ctypes.util.find_library("c"), use_errno=True)
fd = os.open("/dev/net/tun", os.O_RDWR)
ifr = struct.pack("16sH", b"cmtun2", 0x0001 | 0x1000)
if libc.ioctl(fd, 0x400454ca, ifr) < 0:
    raise OSError(ctypes.get_errno(), "TUNSETIFF")
if libc.ioctl(fd, 0x400454cc, ctypes.c_int(1)) < 0:
    raise OSError(ctypes.get_errno(), "TUNSETOWNER")
if libc.ioctl(fd, 0x400454cb, ctypes.c_int(1)) < 0:
    raise OSError(ctypes.get_errno(), "TUNSETPERSIST")
os.close(fd)
"#;
    let python = Command::new("python3")
        .arg("-c")
        .arg(script)
        .status()
        .unwrap();
    assert!(python.success(), "persistent tun was not created");
    run(&["ip", "link", "set", "cmtun2", "mtu", "1400"]);
    run(&["ip", "addr", "add", "198.18.0.1/30", "dev", "cmtun2"]);
    run(&["ip", "-6", "addr", "add", "fd00:c::1/64", "dev", "cmtun2"]);
    run(&["ip", "link", "set", "cmtun2", "up"]);
    let owned_config = owned.join("config.json");
    write_config(&owned_config, "cmtun2", 1400);
    fs::set_permissions(&owned_config, fs::Permissions::from_mode(0o644)).unwrap();
    let owned_log_path = owned.join("log");
    let owned_log_file = fs::File::create(&owned_log_path).unwrap();
    let mut worker = Command::new("setpriv")
        .args([
            "--reuid=1",
            "--regid=1",
            "--clear-groups",
            "--inh-caps=-all",
            "--bounding-set=-all",
            "--",
        ])
        .arg(&binary)
        .arg("-d")
        .arg(&owned)
        .arg("-f")
        .arg(&owned_config)
        .stdin(Stdio::null())
        .stdout(owned_log_file.try_clone().unwrap())
        .stderr(owned_log_file)
        .spawn()
        .unwrap();
    let owned_log = wait_log(&owned_log_path, worker.id());
    assert!(
        owned_log.contains("cmtun2([198.18.0.1/30],[fd00:c::1/64])"),
        "{owned_log}"
    );
    assert!(owned_log.contains("auto route: false"), "{owned_log}");
    let owned_caps = cap_eff(worker.id());
    eprintln!("variant_a_cap_eff={owned_caps:016x}");
    assert_eq!(owned_caps, 0, "unprivileged core kept capabilities");
    let owned_link = run(&["ip", "-d", "link", "show", "cmtun2"]);
    assert!(owned_link.contains("mtu 1400"), "{owned_link}");
    assert!(owned_link.contains("persist on"), "{owned_link}");
    assert!(owned_link.contains("state UP"), "{owned_link}");
    worker.kill().expect("stop owned tun");
    worker.wait().expect("reap owned tun");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let present = Command::new("ip")
        .args(["link", "show", "cmtun2"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    assert!(present, "persistent tun disappeared after kill -9");
    run(&["ip", "link", "del", "cmtun2"]);
    eprintln!("I04_TUN_OK");
}
