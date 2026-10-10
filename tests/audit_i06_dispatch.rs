//! C06, C15: one order of checks for every frame; the reply carries a code and data only.

mod pack_support;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use cm::common::contract_fixtures::TempDirGuard;
use cm::common::sys::euid;
use cm::controller::dispatch::{MAX_CONCURRENT_OPS, handle};
use pack_support::{
    CONFIG, child_peer, frame, gate, open_gate, reply, rig, self_peer, write_config,
};
use serde_json::json;

fn start_op(instance: &str) -> serde_json::Value {
    json!({"type":"worker_start","instance":instance,"generation":1})
}

#[test]
fn c06_identity_is_checked_before_every_operation() {
    let dir = TempDirGuard::new("cm-i06-c06").unwrap();
    let rig = rig(dir.path(), false, None);
    let mut peer = child_peer(dir.path());
    let status = json!({"type":"worker_status","instance":"browser"});
    let first = reply(&handle(
        &frame("frame-0001", status.clone()),
        &peer.identity,
        &rig.deps,
    ));
    assert_eq!(first["code"], "ok");
    peer.finish();
    let second = reply(&handle(
        &frame("frame-0002", start_op("browser")),
        &peer.identity,
        &rig.deps,
    ));
    assert_eq!(second["code"], "peer_changed");
    assert_eq!(rig.factory.made.load(Ordering::SeqCst), 0);
    // Authorization is not even asked for a peer that is gone.
    assert_eq!(rig.auth.actions.lock().unwrap().len(), 1);
}

#[test]
fn c15_garbage_and_denial() {
    let dir = TempDirGuard::new("cm-i06-c15-deny").unwrap();
    let peer = self_peer();
    let denying = rig(dir.path(), true, None);
    assert_eq!(
        handle(b"garbage", &peer.identity, &denying.deps),
        b"{\"v\":1,\"id\":\"\",\"ok\":false,\"code\":\"bad_frame\",\"data\":null}\n"
    );
    write_config(dir.path(), euid(), "browser", 1, CONFIG);
    let owner_dir = dir.path().join(format!("u{}", euid()));
    std::fs::remove_dir_all(&owner_dir).unwrap();
    for (id, op) in [
        ("frame-0001", start_op("browser")),
        (
            "frame-0002",
            json!({"type":"net_apply","instance":"browser","generation":1}),
        ),
        (
            "frame-0003",
            json!({"type":"app_launch","instance":"browser","generation":1,"program":"/bin/true","args":[]}),
        ),
        (
            "frame-0004",
            json!({"type":"instance_prepare","instance":"browser"}),
        ),
    ] {
        let answer = reply(&handle(&frame(id, op), &peer.identity, &denying.deps));
        assert_eq!(answer["code"], "denied");
        assert_eq!(answer["id"], id);
    }
    assert_eq!(denying.factory.made.load(Ordering::SeqCst), 0);
    assert!(denying.exec.lock().unwrap().ran.is_empty());
    assert!(!owner_dir.exists(), "a denied request creates nothing");
}

#[test]
fn c15_actions_and_owner_directory() {
    let dir = TempDirGuard::new("cm-i06-c15-allow").unwrap();
    let peer = self_peer();
    let rig = rig(dir.path(), false, None);
    // The config sits in another uid's directory: this peer cannot name it.
    write_config(
        dir.path(),
        euid() + 1,
        "browser",
        1,
        "{\"secret-marker-i06\":1}",
    );
    let mut texts = Vec::new();
    let mut call = |id: &str, op: serde_json::Value| {
        let bytes = handle(&frame(id, op), &peer.identity, &rig.deps);
        texts.push(String::from_utf8(bytes.clone()).unwrap());
        reply(&bytes)
    };
    assert_eq!(
        call("frame-0001", start_op("browser"))["code"],
        "invalid_config"
    );
    assert_eq!(rig.factory.made.load(Ordering::SeqCst), 0);
    write_config(dir.path(), euid(), "browser", 1, CONFIG);
    let started = call("frame-0002", start_op("browser"));
    assert_eq!(started["data"], json!({"type":"started","generation":1}));
    let status = call(
        "frame-0003",
        json!({"type":"worker_status","instance":"browser"}),
    );
    assert_eq!(status["data"]["running"], true);
    let reconciled = call("frame-0004", json!({"type":"reconcile"}));
    assert_eq!(
        reconciled["data"],
        json!({"type":"reconciled","compensated":0})
    );
    // Operations that were stubs in I06 now answer for themselves.
    let launch = call(
        "frame-0005",
        json!({"type":"app_launch","instance":"browser","generation":1,"program":"/bin/true","args":[]}),
    );
    assert_eq!(launch["code"], "not_running");
    assert_eq!(
        *rig.auth.actions.lock().unwrap(),
        [
            "io.github.cm.worker",
            "io.github.cm.worker",
            "io.github.cm.status",
            "io.github.cm.worker",
            "io.github.cm.app"
        ]
    );
    let base = dir.path().display().to_string();
    let user = std::env::var("USER").unwrap_or_else(|_| "no-user-name".to_owned());
    for text in &texts {
        for leak in [
            base.as_str(),
            "gen-",
            "/proc",
            user.as_str(),
            "secret-marker",
        ] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
    }
}

#[test]
fn c15_busy_when_all_slots_are_taken() {
    let dir = TempDirGuard::new("cm-i06-c15-busy").unwrap();
    let barrier = gate();
    let rig = Arc::new(rig(dir.path(), false, Some(barrier.clone())));
    let mut threads = Vec::new();
    for index in 0..MAX_CONCURRENT_OPS {
        let name = format!("i{index}");
        write_config(dir.path(), euid(), &name, 1, CONFIG);
        let rig = Arc::clone(&rig);
        threads.push(std::thread::spawn(move || {
            let peer = self_peer();
            let id = format!("frame-{index:04}");
            reply(&handle(
                &frame(&id, start_op(&name)),
                &peer.identity,
                &rig.deps,
            ))
        }));
    }
    for _ in 0..500 {
        if rig.factory.made.load(Ordering::SeqCst) == MAX_CONCURRENT_OPS {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(rig.factory.made.load(Ordering::SeqCst), MAX_CONCURRENT_OPS);
    let peer = self_peer();
    let status = json!({"type":"worker_status","instance":"i0"});
    let busy = reply(&handle(
        &frame("frame-busy", status.clone()),
        &peer.identity,
        &rig.deps,
    ));
    assert_eq!(busy["code"], "busy");
    open_gate(&barrier);
    for thread in threads {
        assert_eq!(thread.join().unwrap()["code"], "ok");
    }
    let free = reply(&handle(
        &frame("frame-free", status),
        &peer.identity,
        &rig.deps,
    ));
    assert_eq!(free["code"], "ok");
}
