//! Shared helpers for AB package audits (role 2). Reuses pack rig pieces; sets CM_CONF in tests.
#![allow(dead_code)]

#[path = "../pack_support/mod.rs"]
mod pack_support;

pub use pack_support::*;

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use cm::net::RecordingExec;
use cm::net::commands::{Cmd, Program};
use cm::net::query::FixedQuery;

pub const CM_CONF: &str = "/nonexistent/cm.conf";

pub fn env_guard() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn with_cm_conf<F: FnOnce()>(f: F) {
    let _g = env_guard();
    // SAFETY: serialized by env_guard; tests do not read env concurrently.
    unsafe {
        std::env::set_var("CM_CONF", CM_CONF);
    }
    f();
    // SAFETY: same as above.
    unsafe {
        std::env::remove_var("CM_CONF");
    }
}

pub fn cmd_line(cmd: &Cmd) -> String {
    let program = match cmd.program {
        Program::Ip => "ip",
        Program::Nft => "nft",
        Program::Sysctl => "sysctl",
    };
    format!("{program} {}", cmd.args.join(" "))
}

pub fn cmd_lines(commands: &[Cmd]) -> Vec<String> {
    commands.iter().map(cmd_line).collect()
}

fn fixed_now() -> i64 {
    1_000
}

pub fn rig_with_query(base: &Path, deny: bool, query: FixedQuery) -> Rig {
    let auth = std::sync::Arc::new(ScriptedAuth {
        deny,
        actions: Mutex::new(Vec::new()),
    });
    let lease_dir = base.join("leases");
    let factory = std::sync::Arc::new(FakeFactory {
        lease_dir: Some(lease_dir.clone()),
        ..FakeFactory::default()
    });
    let exec = std::sync::Arc::new(Mutex::new(RecordingExec::default()));
    let nets = std::sync::Arc::new(cm::controller::net_ops::NetCtx {
        exec: exec.clone(),
        query: std::sync::Arc::new(Mutex::new(query)),
        lease_dir,
        etc_root: base.join("etc-netns"),
    });
    let workers = std::sync::Arc::new(cm::controller::core_ops::Workers::new(factory.clone()));
    workers.set_net(std::sync::Arc::clone(&nets));
    Rig {
        deps: cm::controller::dispatch::Deps {
            base: base.to_owned(),
            authorizer: auth.clone(),
            workers,
            slots: std::sync::Arc::new(cm::controller::dispatch::OpSlots::new()),
            now: fixed_now,
            nets,
        },
        auth,
        factory,
        exec,
        base: base.to_owned(),
    }
}

pub fn uplink_query(up0: bool, wlan: bool) -> FixedQuery {
    let mut answers = std::collections::BTreeMap::new();
    let mut v4 = String::from("[");
    if up0 {
        v4.push_str(r#"{"dev":"up0","metric":100}"#);
    }
    if wlan {
        if up0 {
            v4.push(',');
        }
        v4.push_str(r#"{"dev":"wlan0","metric":600}"#);
    }
    v4.push(']');
    answers.insert("-j route show default table main".to_owned(), v4.clone());
    answers.insert(
        "-j -6 route show default table main".to_owned(),
        "[]".to_owned(),
    );
    let mut addrs = r#"["#.to_string();
    if up0 {
        addrs.push_str(r#"{"ifname":"up0","addr_info":[{"local":"192.0.2.2","scope":"global"}]}"#);
    }
    if wlan {
        if up0 {
            addrs.push(',');
        }
        addrs.push_str(r#"{"ifname":"wlan0","addr_info":[{"local":"10.0.0.5","scope":"global"}]}"#);
    }
    addrs.push(']');
    answers.insert("-j -4 addr show".to_owned(), addrs);
    answers.insert("-j rule show".to_owned(), "[]".to_owned());
    answers.insert("-j link show".to_owned(), r#"[{"ifname":"lo"}]"#.to_owned());
    answers.insert("netns list".to_owned(), String::new());
    FixedQuery {
        answers,
        asked: Vec::new(),
    }
}

pub fn host_host_apply_frame(id: &str, generation: u64, mode: &str) -> Vec<u8> {
    frame(
        id,
        serde_json::json!({"type":"host_apply","generation":generation,"mode":mode}),
    )
}

pub fn host_revert_frame(id: &str, generation: u64) -> Vec<u8> {
    frame(
        id,
        serde_json::json!({"type":"host_revert","generation":generation}),
    )
}

pub fn host_status_frame(id: &str) -> Vec<u8> {
    frame(id, serde_json::json!({"type":"host_status"}))
}

pub fn write_host_json(base: &Path, state: &cm::controller::host_ops::HostState) {
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let path = cm::controller::host_ops::state_path(base);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let bytes = serde_json::to_vec(state).unwrap();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    file.write_all(&bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
}

/// Unix peer creds with another uid (needs `setpriv` and subordinate uid).
pub struct OtherPeer {
    pub identity: cm::controller::peer::PeerIdentity,
    _child: std::process::Child,
    _stream: std::os::unix::net::UnixStream,
}

pub fn other_uid_peer(dir: &Path, uid: u32) -> OtherPeer {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::process::{Command, Stdio};

    use cm::controller::peer;

    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o711));
    let path = dir.join(format!("peer-{uid}.sock"));
    let _ = fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("bind");
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o666));
    let mut child = Command::new("setpriv")
        .arg(format!("--reuid={uid}"))
        .arg(format!("--regid={uid}"))
        .arg("--clear-groups")
        .arg("--")
        .arg("python3")
        .arg("-c")
        .arg("import socket,sys\ns=socket.socket(socket.AF_UNIX)\ns.connect(sys.argv[1])\nsys.stdin.read()")
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("setpriv peer");
    listener.set_nonblocking(true).expect("nonblocking");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if child.try_wait().ok().flatten().is_some() {
                    panic!("setpriv peer exited before connect (uid {uid})");
                }
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    panic!("timeout waiting for uid {uid} peer");
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => panic!("accept: {e}"),
        }
    };
    let identity = peer::capture(&stream).expect("capture child");
    OtherPeer {
        identity,
        _child: child,
        _stream: stream,
    }
}
