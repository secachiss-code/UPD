//! K14, X02: the tunnel network exists only as a controller transaction.

mod pack_support;

use std::fs;
use std::sync::atomic::Ordering;

use cm::common::contract_fixtures::TempDirGuard;
use cm::common::sys::euid;
use cm::controller::dispatch::handle;
use cm::controller::journal::{Journal, TxnStatus};
use cm::controller::owner::owned;
use cm::core::{LeaseRegistry, ResourceKind};
use cm::net::Program;
use pack_support::{CONFIG, Rig, child_peer, frame, reply, rig, self_peer, write_config};
use serde_json::{Value, json};

fn apply(instance: &str) -> Value {
    json!({"type":"net_apply","instance":instance,"generation":1})
}

fn revert(instance: &str) -> Value {
    json!({"type":"net_revert","instance":instance,"generation":1})
}

fn tunnel_leases(rig: &Rig) -> Vec<(String, String)> {
    let Ok(registry) = LeaseRegistry::open(&rig.base.join("leases")) else {
        return Vec::new();
    };
    registry
        .list()
        .unwrap()
        .into_iter()
        .filter(|lease| lease.kind == ResourceKind::Tunnel)
        .map(|lease| (lease.holder, lease.value))
        .collect()
}

fn ip_commands(rig: &Rig) -> usize {
    rig.exec
        .lock()
        .unwrap()
        .ran
        .iter()
        .filter(|cmd| cmd.program != Program::Nft)
        .count()
}

#[test]
fn k14_apply_and_revert() {
    let dir = TempDirGuard::new("cm-pack-k14").unwrap();
    let rig = rig(dir.path(), false, None);
    let peer = self_peer();
    let call = |id: &str, op: Value| reply(&handle(&frame(id, op), &peer.identity, &rig.deps));
    let uid = euid();

    assert_eq!(call("frame-0001", revert("browser"))["code"], "not_running");
    let first = call("frame-0002", apply("browser"));
    assert_eq!(
        first["data"],
        json!({"type":"net","index":0,"netns":"cm-0"})
    );
    assert_eq!(
        tunnel_leases(&rig),
        [(format!("u{uid}-browser"), "0".to_owned())]
    );
    let resolv = dir.path().join("etc-netns/cm-0/resolv.conf");
    assert_eq!(
        fs::read_to_string(&resolv).unwrap(),
        "nameserver 198.18.0.2\noptions edns0\n"
    );
    assert_eq!(ip_commands(&rig), 17);
    {
        let exec = rig.exec.lock().unwrap();
        assert_eq!(exec.ran[0].program, Program::Nft);
        assert!(exec.ran[0].stdin.as_deref().unwrap().contains("\"cmv0h\""));
        assert!(exec.ran.iter().any(|cmd| cmd.args.ends_with(&[
            "mode".to_owned(),
            "tun".to_owned(),
            "user".to_owned(),
            uid.to_string()
        ])));
    }
    // The same key answers from the journal and runs nothing.
    assert_eq!(call("frame-0002", apply("browser")), first);
    assert_eq!(ip_commands(&rig), 17);
    assert_eq!(call("frame-0003", apply("browser"))["code"], "conflict");

    assert_eq!(
        call("frame-0004", apply("mail"))["data"],
        json!({"type":"net","index":1,"netns":"cm-1"})
    );
    assert_eq!(call("frame-0005", revert("browser"))["code"], "ok");
    assert!(!resolv.exists());
    assert_eq!(
        tunnel_leases(&rig),
        [(format!("u{uid}-mail"), "1".to_owned())]
    );
    assert_eq!(
        call("frame-0006", apply("browser"))["data"]["index"],
        0,
        "a freed number is reused"
    );
    assert_eq!(*rig.auth.actions.lock().unwrap(), ["io.github.cm.net"; 7]);
}

#[test]
fn k14_failed_create_leaves_nothing() {
    let dir = TempDirGuard::new("cm-pack-k14-fail").unwrap();
    let rig = rig(dir.path(), false, None);
    let peer = self_peer();
    rig.exec.lock().unwrap().fail_at = Some(5);
    let answer = reply(&handle(
        &frame("frame-0001", apply("browser")),
        &peer.identity,
        &rig.deps,
    ));
    assert_eq!(answer["code"], "failed");
    assert!(tunnel_leases(&rig).is_empty());
    assert!(!dir.path().join("etc-netns/cm-0").exists());
    let owner = owned(dir.path(), euid(), "browser").unwrap();
    let journal = Journal::open(&owner.journal_path()).unwrap();
    assert_eq!(
        journal.find("frame-0001").unwrap().unwrap().status,
        TxnStatus::Aborted
    );
    assert!(
        !fs::read_to_string(owner.generations_path())
            .unwrap_or_default()
            .contains("net:")
    );
    // The rollback ran: the last commands delete what was created.
    let exec = rig.exec.lock().unwrap();
    assert!(
        exec.ran
            .iter()
            .any(|cmd| cmd.args == ["netns", "del", "cm-0"])
    );
}

#[test]
fn k14_quota_is_64_tunnels() {
    let dir = TempDirGuard::new("cm-pack-k14-quota").unwrap();
    let rig = rig(dir.path(), false, None);
    let peer = self_peer();
    for index in 0..64 {
        let answer = reply(&handle(
            &frame(&format!("frame-{index:04}"), apply(&format!("i{index}"))),
            &peer.identity,
            &rig.deps,
        ));
        assert_eq!(answer["data"]["index"], index, "{answer}");
    }
    let extra = reply(&handle(
        &frame("frame-extra", apply("extra")),
        &peer.identity,
        &rig.deps,
    ));
    assert_eq!(extra["code"], "quota");
    assert_eq!(tunnel_leases(&rig).len(), 64);
    assert!(!dir.path().join("etc-netns/cm-64").exists());
}

#[test]
fn k14_worker_gets_the_tunnel_of_its_network() {
    let dir = TempDirGuard::new("cm-pack-k14-worker").unwrap();
    let rig = rig(dir.path(), false, None);
    let peer = self_peer();
    let call = |id: &str, op: Value| reply(&handle(&frame(id, op), &peer.identity, &rig.deps));
    for name in ["plain", "other", "browser"] {
        write_config(dir.path(), euid(), name, 1, CONFIG);
    }
    let start = |name: &str| json!({"type":"worker_start","instance":name,"generation":1});
    assert_eq!(call("frame-0001", start("plain"))["code"], "ok");
    assert_eq!(call("frame-0002", apply("other"))["code"], "ok");
    assert_eq!(call("frame-0003", apply("browser"))["data"]["index"], 1);
    assert_eq!(call("frame-0004", start("browser"))["code"], "ok");
    let tunnels = rig.factory.tunnels.lock().unwrap();
    assert_eq!(rig.factory.made.load(Ordering::SeqCst), 2);
    assert!(
        tunnels[0].is_none(),
        "an instance without a network gets a port, not a TUN"
    );
    let tunnel = tunnels[1].as_ref().unwrap();
    assert_eq!(
        (tunnel.index, tunnel.tun.as_str(), tunnel.owner_uid),
        (1, "cmtun1", euid())
    );
}

#[test]
fn x02_same_order_of_checks_as_worker_operations() {
    let dir = TempDirGuard::new("cm-pack-x02").unwrap();
    let denying = rig(&dir.path().join("deny"), true, None);
    fs::create_dir_all(&denying.base).unwrap();
    let peer = self_peer();
    for op in [apply("browser"), revert("browser")] {
        let answer = reply(&handle(
            &frame("frame-0001", op),
            &peer.identity,
            &denying.deps,
        ));
        assert_eq!(answer["code"], "denied");
    }
    assert!(denying.exec.lock().unwrap().ran.is_empty());
    assert_eq!(
        *denying.auth.actions.lock().unwrap(),
        ["io.github.cm.net"; 2]
    );

    let allowing = rig(&dir.path().join("allow"), false, None);
    fs::create_dir_all(&allowing.base).unwrap();
    let mut gone = child_peer(dir.path());
    gone.finish();
    let answer = reply(&handle(
        &frame("frame-0002", apply("browser")),
        &gone.identity,
        &allowing.deps,
    ));
    assert_eq!(answer["code"], "peer_changed");
    assert!(allowing.exec.lock().unwrap().ran.is_empty());
    // The tunnel number cannot be named in a request.
    let mut numbered = apply("browser");
    numbered["index"] = json!(5);
    let answer = reply(&handle(
        &frame("frame-0003", numbered),
        &peer.identity,
        &allowing.deps,
    ));
    assert_eq!(answer["code"], "bad_frame");
    for op in [apply("browser"), revert("browser")] {
        let answer = reply(&handle(
            &frame("frame-0004", op.clone()),
            &peer.identity,
            &allowing.deps,
        ));
        assert_ne!(answer["code"], "unsupported", "{op}");
    }
}
