//! Shared pieces for the I06 and PACK audits: a captured peer, scripted authorizers and a
//! factory of fake adapters. Nothing here touches the host network or the real polkit.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use cm::controller::actions::Authorizer;
use cm::controller::core_ops::{AdapterFactory, Workers};
use cm::controller::dispatch::{Deps, OpSlots};
use cm::controller::drop::RunAs;
use cm::controller::net_ops::NetCtx;
use cm::controller::owner::Owned;
use cm::controller::peer::{self, PeerIdentity};
use cm::controller::protocol::ControlError;
use cm::core::adapter::{
    CoreAdapter, CoreCapabilities, CoreConfig, CoreError, CoreReadiness, CoreStatistics,
};
use cm::core::{FakeAdapter, FakeOp};
use cm::net::{RecordingExec, TunnelNet};

pub const CONFIG: &str = r#"{"mode":"direct","ipv6":false}"#;

/// This process seen from the other end of a socket pair. The pair stays alive in the value.
pub struct SelfPeer {
    pub identity: PeerIdentity,
    _pair: (UnixStream, UnixStream),
}

pub fn self_peer() -> SelfPeer {
    let pair = UnixStream::pair().expect("socketpair");
    let identity = peer::capture(&pair.0).expect("capture");
    SelfPeer {
        identity,
        _pair: pair,
    }
}

/// A child that connects to a listener and waits on its stdin. Dropping the handle ends it.
pub struct ChildPeer {
    pub identity: PeerIdentity,
    pub child: Child,
    _stream: UnixStream,
}

pub fn child_peer(dir: &Path) -> ChildPeer {
    let path = dir.join("peer.sock");
    let _ = fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("bind");
    let child = Command::new("python3")
        .args([
            "-c",
            "import socket,sys\ns=socket.socket(socket.AF_UNIX)\ns.connect(sys.argv[1])\nsys.stdin.read()",
        ])
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("python3");
    let (stream, _) = listener.accept().expect("accept");
    let identity = peer::capture(&stream).expect("capture child");
    ChildPeer {
        identity,
        child,
        _stream: stream,
    }
}

impl ChildPeer {
    pub fn finish(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}

impl Drop for ChildPeer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
pub struct ScriptedAuth {
    pub deny: bool,
    pub actions: Mutex<Vec<String>>,
}

impl Authorizer for ScriptedAuth {
    fn authorize(&self, _peer: &PeerIdentity, action: &str) -> Result<(), ControlError> {
        self.actions.lock().unwrap().push(action.to_owned());
        if self.deny {
            Err(ControlError::Denied)
        } else {
            Ok(())
        }
    }
}

type Script = Arc<Mutex<VecDeque<(FakeOp, CoreError)>>>;
type Gate = Arc<(Mutex<bool>, Condvar)>;

/// Fake adapters that share one failure script, so a test can fail a later call of a
/// running adapter. With a gate, `start` blocks until the gate opens.
#[derive(Default)]
pub struct FakeFactory {
    pub made: AtomicUsize,
    pub script: Script,
    pub tunnels: Mutex<Vec<Option<TunnelNet>>>,
    pub gate: Option<Gate>,
    pub lease_dir: Option<PathBuf>,
}

pub fn gate() -> Gate {
    Arc::new((Mutex::new(false), Condvar::new()))
}

pub fn open_gate(gate: &Gate) {
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
}

impl AdapterFactory for FakeFactory {
    fn make(
        &self,
        _owned: &Owned,
        _run_as: &RunAs,
        tunnel: Option<&TunnelNet>,
    ) -> Box<dyn CoreAdapter + Send> {
        self.made.fetch_add(1, Ordering::SeqCst);
        self.tunnels.lock().unwrap().push(tunnel.cloned());
        Box::new(Scripted {
            inner: FakeAdapter::new(),
            script: Arc::clone(&self.script),
            gate: self.gate.clone(),
        })
    }

    fn lease_dir(&self) -> Option<&Path> {
        self.lease_dir.as_deref()
    }
}

struct Scripted {
    inner: FakeAdapter,
    script: Script,
    gate: Option<Gate>,
}

impl Scripted {
    fn scripted(&mut self, op: FakeOp) -> Result<(), CoreError> {
        let mut script = self.script.lock().unwrap();
        if let Some(index) = script.iter().position(|(candidate, _)| *candidate == op) {
            let (_, error) = script.remove(index).expect("index exists");
            return Err(error);
        }
        Ok(())
    }
}

impl CoreAdapter for Scripted {
    fn capabilities(&self) -> CoreCapabilities {
        self.inner.capabilities()
    }
    fn validate(&mut self, config: &CoreConfig) -> Result<(), CoreError> {
        self.inner.validate(config)
    }
    fn start(&mut self, config: &CoreConfig) -> Result<CoreReadiness, CoreError> {
        if let Some(gate) = &self.gate {
            let mut open = gate.0.lock().unwrap();
            while !*open {
                open = gate.1.wait(open).unwrap();
            }
        }
        self.scripted(FakeOp::Start)?;
        self.inner.start(config)
    }
    fn stop(&mut self) -> Result<(), CoreError> {
        self.inner.stop()
    }
    fn reload(&mut self, config: &CoreConfig) -> Result<CoreReadiness, CoreError> {
        self.scripted(FakeOp::Reload)?;
        self.inner.reload(config)
    }
    fn health(&mut self) -> Result<CoreReadiness, CoreError> {
        self.inner.health()
    }
    fn statistics(&mut self) -> Result<CoreStatistics, CoreError> {
        self.inner.statistics()
    }
}

pub struct Rig {
    pub deps: Deps,
    pub auth: Arc<ScriptedAuth>,
    pub factory: Arc<FakeFactory>,
    pub exec: Arc<Mutex<RecordingExec>>,
    pub base: PathBuf,
}

fn fixed_now() -> i64 {
    1_000
}

pub fn rig(base: &Path, deny: bool, gate: Option<Gate>) -> Rig {
    let auth = Arc::new(ScriptedAuth {
        deny,
        actions: Mutex::new(Vec::new()),
    });
    let lease_dir = base.join("leases");
    let factory = Arc::new(FakeFactory {
        gate,
        lease_dir: Some(lease_dir.clone()),
        ..FakeFactory::default()
    });
    let exec = Arc::new(Mutex::new(RecordingExec::default()));
    let nets = Arc::new(NetCtx {
        exec: exec.clone(),
        lease_dir,
        etc_root: base.join("etc-netns"),
    });
    let workers = Arc::new(Workers::new(factory.clone()));
    workers.set_net(Arc::clone(&nets));
    Rig {
        deps: Deps {
            base: base.to_owned(),
            authorizer: auth.clone(),
            workers,
            slots: Arc::new(OpSlots::new()),
            now: fixed_now,
            nets,
        },
        auth,
        factory,
        exec,
        base: base.to_owned(),
    }
}

pub fn frame(id: &str, op: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "v": 1, "id": id, "op": op })).unwrap()
}

pub fn reply(bytes: &[u8]) -> serde_json::Value {
    assert!(bytes.ends_with(b"\n"), "reply is one line");
    serde_json::from_slice(&bytes[..bytes.len() - 1]).expect("reply json")
}

pub fn write_config(base: &Path, uid: u32, instance: &str, generation: u64, text: &str) -> PathBuf {
    let dir = base
        .join(format!("u{uid}"))
        .join("instances")
        .join(instance)
        .join("config");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("gen-{generation}.json"));
    let mut file = fs::File::create(&path).unwrap();
    file.write_all(text.as_bytes()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    path
}

pub struct Xorshift(pub u64);

impl Xorshift {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// One replace, insert or delete of a byte.
pub fn mutate(sample: &[u8], rng: &mut Xorshift) -> Vec<u8> {
    let mut bytes = sample.to_vec();
    let at = (rng.next() as usize) % bytes.len().max(1);
    let byte = rng.next() as u8;
    match rng.next() % 3 {
        0 if !bytes.is_empty() => bytes[at] = byte,
        1 => bytes.insert(at, byte),
        _ if !bytes.is_empty() => {
            bytes.remove(at);
        }
        _ => bytes.push(byte),
    }
    bytes
}

/// Re-run the named test of this binary inside a fresh user and network namespace.
/// Returns true in the outer process once the inner run has passed.
pub fn rerun_in_netns(test: &str, marker: &str) -> bool {
    if std::env::var(marker).ok().as_deref() == Some("1") {
        return false;
    }
    let exe = std::env::current_exe().unwrap();
    let output = Command::new("timeout")
        .args(["120", "unshare", "-rn", "--"])
        .arg(exe)
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(marker, "1")
        .output()
        .expect("unshare");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "inner run failed:\n{text}");
    assert!(
        text.contains("1 passed"),
        "inner run did not execute {test}:\n{text}"
    );
    assert!(!text.contains("SKIPPED"), "inner run skipped:\n{text}");
    true
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Re-run the named test as uid 0 of a user namespace that also maps the subordinate uids,
/// so `setgroups` works and uid 1 exists. Network and mounts are private as well.
pub fn rerun_in_userns(test: &str, marker: &str) -> bool {
    if std::env::var(marker).ok().as_deref() == Some("1") {
        return false;
    }
    let exe = std::env::current_exe().unwrap();
    let output = Command::new("timeout")
        .args([
            "120",
            "unshare",
            "-U",
            "--map-root-user",
            "--map-auto",
            "-n",
            "-m",
            "--",
        ])
        .arg(exe)
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(marker, "1")
        .output()
        .expect("unshare");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "inner run failed:\n{text}");
    assert!(
        text.contains("1 passed"),
        "inner run did not execute {test}:\n{text}"
    );
    assert!(!text.contains("SKIPPED"), "inner run skipped:\n{text}");
    true
}

/// A directory every uid of the namespace can pass through, with a copy of `binary` in it.
pub fn shared_copy(dir: &Path, binary: &Path) -> PathBuf {
    fs::set_permissions(dir, fs::Permissions::from_mode(0o711)).unwrap();
    let copy = dir.join("core-binary");
    fs::copy(binary, &copy).unwrap();
    fs::set_permissions(&copy, fs::Permissions::from_mode(0o755)).unwrap();
    copy
}
