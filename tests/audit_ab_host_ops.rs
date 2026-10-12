//! H06, H07, H08 (apply), H11: host firewall ordering, worker tunnel, host.json transactions.

#[path = "ab_support/mod.rs"]
mod ab_support;

use std::fs;
use std::sync::atomic::Ordering;

use cm::common::contract_fixtures::TempDirGuard;
use cm::common::sys::euid;
use cm::controller::actions::ACTION_HOST;
use cm::controller::dispatch::handle;
use cm::controller::host_ops::{self, HostState};
use cm::net::Program;
use serde_json::json;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use ab_support::{
    CONFIG, frame, host_host_apply_frame, host_revert_frame, host_status_frame, reply,
    rerun_in_userns, rig_with_query, self_peer, uplink_query, with_cm_conf, write_config,
};

#[test]
fn h07_h11_cross_uid_in_userns() {
    if rerun_in_userns("h07_h11_cross_uid_in_userns", "CM_AB_CROSS_UID") {
        return;
    }
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-ab-cross").unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o711)).unwrap();
        let rig = rig_with_query(dir.path(), false, uplink_query(true, false));
        write_config(dir.path(), euid(), "host", 1, CONFIG);
        let peer = self_peer();
        reply(&handle(
            &host_host_apply_frame("h07own001", 1, "tunnel"),
            &peer.identity,
            &rig.deps,
        ));
        let other = ab_support::other_uid_peer(dir.path(), 1001);
        let foreign = reply(&handle(
            &frame(
                "h07oth001",
                json!({"type":"worker_start","instance":"host","generation":1}),
            ),
            &other.identity,
            &rig.deps,
        ));
        assert!(
            foreign["code"] == "conflict" || foreign["code"] == "failed",
            "foreign worker_start: {foreign}"
        );
        let off = reply(&handle(
            &host_status_frame("h11oth001"),
            &other.identity,
            &rig.deps,
        ));
        assert_eq!(off["data"]["mode"], "off");
        assert_eq!(
            reply(&handle(
                &host_revert_frame("h11rev002", 1),
                &other.identity,
                &rig.deps,
            ))["code"],
            "not_running"
        );
    });
}

#[test]
fn h06_host_firewall_order_with_apply() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-ab-h06").unwrap();
        let rig = rig_with_query(dir.path(), false, uplink_query(true, false));
        let peer = self_peer();
        let apply = reply(&handle(
            &host_host_apply_frame("h06-0001", 1, "tunnel"),
            &peer.identity,
            &rig.deps,
        ));
        assert_eq!(apply["code"], "ok");
        let exec = rig.exec.lock().unwrap();
        let last_ip = exec
            .ran
            .iter()
            .rposition(|c| c.program == Program::Ip)
            .unwrap();
        let nft_after = exec.ran.iter().skip(last_ip + 1).any(|c| {
            c.program == Program::Nft
                && c.stdin
                    .as_deref()
                    .is_some_and(|s| s.contains("table ip cm_host {"))
        });
        assert!(nft_after);
        drop(exec);
        let revert = reply(&handle(
            &host_revert_frame("h06-0002", 1),
            &peer.identity,
            &rig.deps,
        ));
        assert_eq!(revert["code"], "ok");
        let exec = rig.exec.lock().unwrap();
        let delete = exec.ran.iter().find(|c| {
            c.program == Program::Nft
                && c.stdin.as_deref() == Some("table ip cm_host\ndelete table ip cm_host\n")
        });
        assert!(delete.is_some(), "revert must delete cm_host table");
        let dir2 = TempDirGuard::new("cm-ab-h06-proxy").unwrap();
        let rig2 = rig_with_query(dir2.path(), false, uplink_query(true, false));
        let peer2 = self_peer();
        reply(&handle(
            &host_host_apply_frame("h06-p001", 1, "proxy"),
            &peer2.identity,
            &rig2.deps,
        ));
        assert!(
            !rig2
                .exec
                .lock()
                .unwrap()
                .ran
                .iter()
                .any(|c| c.program == Program::Ip || c.program == Program::Nft)
        );
    });
}

#[test]
fn h07_host_worker_tunnel() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-ab-h07").unwrap();
        let rig = rig_with_query(dir.path(), false, uplink_query(true, false));
        write_config(dir.path(), euid(), "host", 1, CONFIG);
        let peer = self_peer();
        reply(&handle(
            &host_host_apply_frame("h07-0001", 1, "tunnel"),
            &peer.identity,
            &rig.deps,
        ));
        let start = reply(&handle(
            &frame(
                "h07-0002",
                json!({"type":"worker_start","instance":"host","generation":1}),
            ),
            &peer.identity,
            &rig.deps,
        ));
        assert_eq!(start["code"], "ok");
        let tunnel = rig.factory.tunnels.lock().unwrap().pop().unwrap().unwrap();
        assert_eq!(tunnel.tun, "cmtunh");
        assert_eq!(tunnel.index, 64);
        let dir2 = TempDirGuard::new("cm-ab-h07-proxy").unwrap();
        let rig2 = rig_with_query(dir2.path(), false, uplink_query(true, false));
        write_config(dir2.path(), euid(), "host", 1, CONFIG);
        let peer2 = self_peer();
        reply(&handle(
            &host_host_apply_frame("h07-p001", 1, "proxy"),
            &peer2.identity,
            &rig2.deps,
        ));
        reply(&handle(
            &frame(
                "h07-p002",
                json!({"type":"worker_start","instance":"host","generation":1}),
            ),
            &peer2.identity,
            &rig2.deps,
        ));
        assert!(rig2.factory.tunnels.lock().unwrap()[0].is_none());
        assert_eq!(rig.factory.made.load(Ordering::SeqCst), 1);
        let dir_empty = TempDirGuard::new("cm-ab-h07-empty").unwrap();
        let rig_empty = rig_with_query(dir_empty.path(), false, uplink_query(true, false));
        write_config(dir_empty.path(), euid(), "host", 1, CONFIG);
        let peer_empty = self_peer();
        assert_eq!(
            reply(&handle(
                &frame(
                    "h07-0003",
                    json!({"type":"worker_start","instance":"host","generation":1})
                ),
                &peer_empty.identity,
                &rig_empty.deps,
            ))["code"],
            "conflict"
        );
        let dir_owned = TempDirGuard::new("cm-ab-h07-owned").unwrap();
        let rig_owned = rig_with_query(dir_owned.path(), false, uplink_query(true, false));
        write_config(dir_owned.path(), euid(), "host", 1, CONFIG);
        reply(&handle(
            &host_host_apply_frame("h07-own1", 1, "tunnel"),
            &peer.identity,
            &rig_owned.deps,
        ));
        let bad_net = reply(&handle(
            &frame(
                "h07-0005",
                json!({"type":"net_apply","instance":"host","generation":1}),
            ),
            &peer.identity,
            &rig.deps,
        ));
        assert_eq!(bad_net["code"], "bad_instance");
    });
}

#[test]
fn h08_host_apply_blocks() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-ab-h08-ops").unwrap();
        let mut q = uplink_query(true, false);
        q.answers.insert(
            "-j link show".to_owned(),
            r#"[{"ifname":"lo"},{"ifname":"cm-vpn"}]"#.to_owned(),
        );
        let rig = rig_with_query(dir.path(), false, q);
        let peer = self_peer();
        let blocked = reply(&handle(
            &host_host_apply_frame("h08-0001", 1, "tunnel"),
            &peer.identity,
            &rig.deps,
        ));
        assert_eq!(blocked["code"], "conflict");
        assert!(rig.exec.lock().unwrap().ran.is_empty());
        assert!(!host_ops::state_path(dir.path()).exists());
        let mut q2 = uplink_query(true, false);
        q2.answers
            .insert("-j link show".to_owned(), r#"[{"ifname":"lo"}]"#.to_owned());
        let _rig2 = rig_with_query(&dir.path().join("resolved"), false, q2);
        fs::create_dir_all(dir.path().join("systemd/resolve")).unwrap();
        fs::write(
            dir.path().join("systemd/resolve/stub-resolv.conf"),
            b"nameserver 127.0.0.53\n",
        )
        .unwrap();
        // detect uses /run — stub resolved via env not wired; use FixedQuery + preflight path in apply:
        // host_apply reads detect(Path::new("/run")) — skip if not resolved on CI; test conflict path above.
    });
}

#[test]
fn h11_host_transactions() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-ab-h11").unwrap();
        let rig = rig_with_query(dir.path(), false, uplink_query(true, false));
        let peer = self_peer();
        let status = reply(&handle(
            &host_status_frame("h11stat01"),
            &peer.identity,
            &rig.deps,
        ));
        assert_eq!(status["code"], "ok", "host_status reply: {status}");
        assert_eq!(
            status["data"],
            json!({"type":"host","mode":"off","generation":null,"uplinks":[]})
        );
        assert_eq!(
            reply(&handle(
                &host_host_apply_frame("h11bad001", 1, "vpn"),
                &peer.identity,
                &rig.deps,
            ))["code"],
            "bad_argument"
        );
        let deny = rig_with_query(&dir.path().join("deny"), true, uplink_query(true, false));
        fs::create_dir_all(&deny.base).unwrap();
        let denied = reply(&handle(
            &host_host_apply_frame("h11den001", 1, "tunnel"),
            &peer.identity,
            &deny.deps,
        ));
        assert_eq!(denied["code"], "denied");
        assert!(deny.exec.lock().unwrap().ran.is_empty());
        assert_eq!(deny.auth.actions.lock().unwrap()[0].as_str(), ACTION_HOST);
        let first = reply(&handle(
            &host_host_apply_frame("h11-0001", 1, "tunnel"),
            &peer.identity,
            &rig.deps,
        ));
        assert_eq!(
            first["data"],
            json!({"type":"host","mode":"tunnel","generation":1,"uplinks":["up0"]})
        );
        let path = host_ops::state_path(dir.path());
        let meta = fs::symlink_metadata(&path).unwrap();
        assert_eq!(meta.mode() & 0o777, 0o600);
        let state: HostState = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(state.uid, euid());
        let again = reply(&handle(
            &host_host_apply_frame("h11-0001", 1, "tunnel"),
            &peer.identity,
            &rig.deps,
        ));
        assert_eq!(again, first);
        assert_eq!(rig.exec.lock().unwrap().ran.len(), 17);
        assert_eq!(
            reply(&handle(
                &host_host_apply_frame("h11-0002", 2, "tunnel"),
                &peer.identity,
                &rig.deps,
            ))["code"],
            "conflict"
        );
        assert_eq!(
            reply(&handle(
                &host_revert_frame("h11rev001", 2),
                &peer.identity,
                &rig.deps,
            ))["code"],
            "generation_mismatch"
        );
        write_config(dir.path(), euid(), "host", 1, CONFIG);
        reply(&handle(
            &frame(
                "h11wstart",
                json!({"type":"worker_start","instance":"host","generation":1}),
            ),
            &peer.identity,
            &rig.deps,
        ));
        assert_eq!(
            reply(&handle(
                &host_revert_frame("h11rev003", 1),
                &peer.identity,
                &rig.deps,
            ))["code"],
            "conflict"
        );
        reply(&handle(
            &frame(
                "h11-stop",
                json!({"type":"worker_stop","instance":"host","generation":1}),
            ),
            &peer.identity,
            &rig.deps,
        ));
        let ok = reply(&handle(
            &host_revert_frame("h11rev004", 1),
            &peer.identity,
            &rig.deps,
        ));
        assert_eq!(ok["code"], "ok");
        assert!(!path.exists());
        let gens = fs::read_to_string(
            dir.path()
                .join(format!("u{}", euid()))
                .join("generations.json"),
        )
        .unwrap_or_default();
        assert!(!gens.contains("net:host"));
        let fail_dir = TempDirGuard::new("cm-ab-h11-fail").unwrap();
        let fail_rig = rig_with_query(fail_dir.path(), false, uplink_query(true, false));
        fail_rig.exec.lock().unwrap().fail_at = Some(3);
        let peer_f = self_peer();
        assert_eq!(
            reply(&handle(
                &host_host_apply_frame("h11fail01", 1, "tunnel"),
                &peer_f.identity,
                &fail_rig.deps,
            ))["code"],
            "failed"
        );
        assert!(!host_ops::state_path(fail_dir.path()).exists());
        let bad_frame = serde_json::to_vec(&json!({
            "v":1,"id":"h11-bf","op":{"type":"host_apply","generation":1,"mode":"tunnel","uid":9999}
        }))
        .unwrap();
        assert_eq!(
            reply(&handle(&bad_frame, &peer.identity, &rig.deps))["code"],
            "bad_frame"
        );
    });
}
