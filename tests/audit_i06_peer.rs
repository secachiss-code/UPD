//! C05: peer identity comes from the socket; a peer that went away is noticed.

mod pack_support;

use std::fs;
use std::os::unix::fs::MetadataExt;

use cm::common::contract_fixtures::TempDirGuard;
use cm::common::sys::euid;
use cm::controller::peer::{parse_cgroup, parse_start_time, session_of};
use cm::controller::protocol::ControlError;

#[test]
fn c05_capture_reads_this_process() {
    let peer = pack_support::self_peer();
    let identity = &peer.identity;
    assert_eq!(identity.uid, euid());
    assert_eq!(identity.pid, std::process::id() as i32);
    let stat = fs::read_to_string("/proc/self/stat").unwrap();
    assert_eq!(Some(identity.start_time), parse_start_time(&stat));
    assert_eq!(
        identity.netns_inode,
        fs::metadata("/proc/self/ns/net").unwrap().ino()
    );
    assert!(identity.alive());
    assert_eq!(identity.verify(), Ok(()));
    let debug = format!("{identity:?}");
    assert!(
        !debug.contains(".slice") && !debug.contains("cgroup"),
        "{debug}"
    );
}

#[test]
fn c05_parsers() {
    assert_eq!(
        parse_start_time("1234 (a b) c) S 1 1 1 0 -1 4194560 1 0 0 0 0 0 0 0 20 0 1 0 987654 0 0"),
        Some(987654)
    );
    assert_eq!(parse_start_time("garbage"), None);
    assert_eq!(
        parse_cgroup("0::/user.slice/user-1000.slice/session-3.scope\n").as_deref(),
        Some("/user.slice/user-1000.slice/session-3.scope")
    );
    assert_eq!(parse_cgroup(""), None);
    assert_eq!(
        session_of("/user.slice/user-1000.slice/session-3.scope").as_deref(),
        Some("3")
    );
    assert_eq!(
        session_of("/user.slice/user-1000.slice/user@1000.service/app.slice/x.scope"),
        None
    );
}

#[test]
fn c05_dead_peer_is_noticed() {
    let dir = TempDirGuard::new("cm-i06-c05").unwrap();
    let mut peer = pack_support::child_peer(dir.path());
    assert_ne!(peer.identity.pid, std::process::id() as i32);
    assert!(peer.identity.alive());
    assert_eq!(peer.identity.verify(), Ok(()));
    peer.finish();
    assert!(!peer.identity.alive());
    assert_eq!(peer.identity.verify(), Err(ControlError::PeerChanged));
}
