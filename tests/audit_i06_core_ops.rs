//! C14: the worker lifecycle through the controller, with fake adapters.

mod pack_support;

use std::fs;
use std::sync::atomic::Ordering;

use cm::common::contract_fixtures::TempDirGuard;
use cm::controller::core_ops::MAX_INSTANCES_PER_UID;
use cm::controller::drop::RunAs;
use cm::controller::journal::{Journal, TxnStatus};
use cm::controller::owner::{Owned, owned};
use cm::controller::protocol::{ControlError, Reply, ReplyData};
use cm::core::FakeOp;
use cm::core::adapter::CoreError;
use pack_support::{CONFIG, Rig, rig, write_config};

fn run_as(uid: u32) -> RunAs {
    RunAs {
        uid,
        gid: uid,
        groups: vec![uid],
    }
}

fn start(rig: &Rig, owner: &Owned, txn: &str, generation: u64) -> Result<Reply, ControlError> {
    rig.deps
        .workers
        .start(owner, &run_as(owner.uid), txn, "digest", generation, 1)
}

fn status(rig: &Rig, owner: &Owned) -> (bool, Option<u64>, String) {
    match rig.deps.workers.status(owner, "status-1").data {
        Some(ReplyData::Status {
            running,
            generation,
            api,
            ..
        }) => (running, generation, api),
        other => panic!("{other:?}"),
    }
}

fn generations(owner: &Owned) -> String {
    fs::read_to_string(owner.generations_path()).unwrap_or_default()
}

#[test]
fn c14_lifecycle() {
    let dir = TempDirGuard::new("cm-i06-c14").unwrap();
    let base = dir.path();
    let uid = cm::common::sys::euid();
    let rig = rig(base, false, None);
    let owner = owned(base, uid, "browser").unwrap();
    for generation in [1, 3] {
        write_config(base, uid, "browser", generation, CONFIG);
    }

    let first = start(&rig, &owner, "start-0001", 1).unwrap();
    assert_eq!(first.data, Some(ReplyData::Started { generation: 1 }));
    assert_eq!(
        status(&rig, &owner),
        (true, Some(1), "api_ready".to_owned())
    );
    // The same key answers from the journal; nothing runs twice.
    assert_eq!(start(&rig, &owner, "start-0001", 1).unwrap(), first);
    assert_eq!(rig.factory.made.load(Ordering::SeqCst), 1);
    assert_eq!(
        start(&rig, &owner, "start-0002", 1).err(),
        Some(ControlError::Conflict)
    );
    // No config for generation 2: the factory is not called.
    assert_eq!(
        rig.deps
            .workers
            .reload(&owner, "reload-0001", "digest", 1, 2, 1)
            .err(),
        Some(ControlError::InvalidConfig)
    );
    write_config(base, uid, "browser", 2, CONFIG);

    // A reload the adapter refuses leaves the generation where it was.
    rig.factory
        .script
        .lock()
        .unwrap()
        .push_back((FakeOp::Reload, CoreError::InvalidConfig));
    let before = generations(&owner);
    assert_eq!(
        rig.deps
            .workers
            .reload(&owner, "reload-0002", "digest", 1, 2, 1)
            .err(),
        Some(ControlError::InvalidConfig)
    );
    assert_eq!(status(&rig, &owner).1, Some(1));
    assert_eq!(generations(&owner), before);

    let reloaded = rig
        .deps
        .workers
        .reload(&owner, "reload-0003", "digest", 1, 2, 1)
        .unwrap();
    assert_eq!(reloaded.data, Some(ReplyData::Started { generation: 2 }));
    assert_eq!(status(&rig, &owner).1, Some(2));
    assert_eq!(
        rig.deps
            .workers
            .reload(&owner, "reload-0004", "digest", 1, 3, 1)
            .err(),
        Some(ControlError::GenerationMismatch)
    );
    assert_eq!(
        rig.deps
            .workers
            .stop(&owner, "stop-0000", "digest", 1, 1)
            .err(),
        Some(ControlError::GenerationMismatch)
    );
    assert!(status(&rig, &owner).0);

    rig.deps
        .workers
        .stop(&owner, "stop-0001", "digest", 2, 1)
        .unwrap();
    assert_eq!(status(&rig, &owner), (false, None, "down".to_owned()));
    assert_eq!(
        rig.deps
            .workers
            .stop(&owner, "stop-0002", "digest", 2, 1)
            .err(),
        Some(ControlError::NotRunning)
    );
    assert!(!generations(&owner).contains("browser"));
}

#[test]
fn c14_stale_generation_and_missing_config() {
    let dir = TempDirGuard::new("cm-i06-c14-stale").unwrap();
    let base = dir.path();
    let uid = cm::common::sys::euid();
    let rig = rig(base, false, None);
    let owner = owned(base, uid, "browser").unwrap();
    write_config(base, uid, "browser", 0, CONFIG);
    write_config(base, uid, "browser", 1, CONFIG);
    start(&rig, &owner, "start-0001", 1).unwrap();
    rig.deps
        .workers
        .stop(&owner, "stop-0001", "digest", 1, 1)
        .unwrap();
    start(&rig, &owner, "start-0002", 1).unwrap();
    // The registry holds generation 1: an older start is stale, not a conflict.
    assert_eq!(
        start(&rig, &owner, "start-0003", 0).err(),
        Some(ControlError::GenerationMismatch)
    );
    let made = rig.factory.made.load(Ordering::SeqCst);
    assert_eq!(
        start(&rig, &owned(base, uid, "third").unwrap(), "start-0004", 2).err(),
        Some(ControlError::InvalidConfig)
    );
    assert_eq!(rig.factory.made.load(Ordering::SeqCst), made);
}

#[test]
fn c14_failed_start_leaves_nothing() {
    let dir = TempDirGuard::new("cm-i06-c14-fail").unwrap();
    let base = dir.path();
    let uid = cm::common::sys::euid();
    let rig = rig(base, false, None);
    let owner = owned(base, uid, "browser").unwrap();
    write_config(base, uid, "browser", 1, CONFIG);
    rig.factory
        .script
        .lock()
        .unwrap()
        .push_back((FakeOp::Start, CoreError::Failed));
    assert_eq!(
        start(&rig, &owner, "start-0001", 1).err(),
        Some(ControlError::Failed)
    );
    assert_eq!(status(&rig, &owner), (false, None, "down".to_owned()));
    assert!(!generations(&owner).contains("browser"));
    let journal = Journal::open(&owner.journal_path()).unwrap();
    assert_eq!(
        journal.find("start-0001").unwrap().unwrap().status,
        TxnStatus::Aborted
    );
    // The slot is free again.
    start(&rig, &owner, "start-0002", 1).unwrap();
    // CoreError maps to a fixed control code.
    for (core, control) in [
        (CoreError::Unsupported, ControlError::Unsupported),
        (CoreError::Busy, ControlError::Busy),
        (CoreError::RouteUnavailable, ControlError::Failed),
    ] {
        let other = owned(base, uid, "mapped").unwrap();
        write_config(base, uid, "mapped", 1, CONFIG);
        rig.factory
            .script
            .lock()
            .unwrap()
            .push_back((FakeOp::Start, core));
        let txn = format!("start-map-{}", control.code());
        assert_eq!(start(&rig, &other, &txn, 1).err(), Some(control));
    }
}

#[test]
fn c14_quota_and_isolation_between_uids() {
    let dir = TempDirGuard::new("cm-i06-c14-quota").unwrap();
    let base = dir.path();
    let uid = cm::common::sys::euid();
    let rig = rig(base, false, None);
    for index in 0..MAX_INSTANCES_PER_UID {
        let name = format!("i{index}");
        write_config(base, uid, &name, 1, CONFIG);
        start(
            &rig,
            &owned(base, uid, &name).unwrap(),
            &format!("start-{index:04}"),
            1,
        )
        .unwrap();
    }
    write_config(base, uid, "extra", 1, CONFIG);
    assert_eq!(
        start(&rig, &owned(base, uid, "extra").unwrap(), "start-extra", 1).err(),
        Some(ControlError::Quota)
    );
    // Another uid computes another directory and another key: it sees nothing of this one.
    let stranger = owned(base, uid + 1, "i0").unwrap();
    assert!(!status(&rig, &stranger).0);
    assert_eq!(
        rig.deps
            .workers
            .stop(&stranger, "stop-0001", "digest", 1, 1)
            .err(),
        Some(ControlError::NotRunning)
    );
    // A config planted in the stranger's directory by this uid is not the stranger's file.
    write_config(base, uid + 1, "i0", 1, CONFIG);
    assert_eq!(
        start(&rig, &stranger, "start-other", 1).err(),
        Some(ControlError::InvalidConfig)
    );
    assert!(status(&rig, &owned(base, uid, "i0").unwrap()).0);
}
