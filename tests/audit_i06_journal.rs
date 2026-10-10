//! C10: the ownership journal survives a torn write and replays every transaction.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::Path;

use cm::common::contract_fixtures::TempDirGuard;
use cm::controller::journal::{Event, Journal, Record, TxnStatus};
use cm::controller::protocol::ControlError;

fn record(txn: &str, event: Event, step: Option<&str>, reply: Option<&str>) -> Record {
    Record {
        txn: txn.to_owned(),
        event,
        instance: "browser".to_owned(),
        generation: 1,
        digest: "d".to_owned(),
        step: step.map(str::to_owned),
        reply: reply.map(str::to_owned),
        at: 1,
    }
}

fn status(journal: &Journal, txn: &str) -> TxnStatus {
    journal.find(txn).unwrap().unwrap().status
}

#[test]
fn c10_file_is_private_and_never_a_symlink() {
    let dir = TempDirGuard::new("cm-i06-c10-open").unwrap();
    let path = dir.path().join("journal.jsonl");
    Journal::open(&path).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    let link = dir.path().join("link.jsonl");
    symlink(&path, &link).unwrap();
    assert_eq!(Journal::open(&link).err(), Some(ControlError::Failed));
}

#[test]
fn c10_states() {
    let dir = TempDirGuard::new("cm-i06-c10-states").unwrap();
    let mut journal = Journal::open(&dir.path().join("j")).unwrap();
    journal
        .append(&record("p", Event::Begin, None, None))
        .unwrap();
    assert_eq!(status(&journal, "p"), TxnStatus::Pending);
    for item in [
        record("c", Event::Begin, None, None),
        record("c", Event::Step, Some("a"), None),
        record("c", Event::Commit, None, Some("R")),
        record("a", Event::Begin, None, None),
        record("a", Event::Step, Some("a"), None),
        record("a", Event::Abort, None, None),
        record("k", Event::Begin, None, None),
        record("k", Event::Step, Some("a"), None),
        record("k", Event::Compensated, None, None),
        record("d", Event::Begin, None, None),
        record("d", Event::CompensationFailed, None, None),
    ] {
        journal.append(&item).unwrap();
    }
    let committed = journal.find("c").unwrap().unwrap();
    assert_eq!(committed.status, TxnStatus::Committed);
    assert_eq!(committed.steps_done, ["a"]);
    assert_eq!(committed.reply.as_deref(), Some("R"));
    assert_eq!(status(&journal, "a"), TxnStatus::Aborted);
    assert_eq!(status(&journal, "k"), TxnStatus::Aborted);
    assert_eq!(status(&journal, "d"), TxnStatus::Dirty);
    assert!(journal.find("none").unwrap().is_none());
}

#[test]
fn c10_torn_tail_is_dropped() {
    let dir = TempDirGuard::new("cm-i06-c10-torn").unwrap();
    let path = dir.path().join("j");
    let mut journal = Journal::open(&path).unwrap();
    journal
        .append(&record("one", Event::Begin, None, None))
        .unwrap();
    journal
        .append(&record("two", Event::Begin, None, None))
        .unwrap();
    journal
        .append(&record("one", Event::Step, Some("a"), None))
        .unwrap();
    drop(journal);
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"txn\":\"x\",\"ev")
        .unwrap();
    let mut journal = Journal::open(&path).unwrap();
    let states = journal.replay().unwrap();
    // Interleaved transactions come back in the order of their Begin.
    assert_eq!(
        states
            .iter()
            .map(|state| state.txn.as_str())
            .collect::<Vec<_>>(),
        ["one", "two"]
    );
    journal
        .append(&record("two", Event::Abort, None, None))
        .unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.ends_with('\n'));
    for line in text.lines() {
        serde_json::from_str::<Record>(line).unwrap();
    }
}

fn size(path: &Path) -> u64 {
    fs::metadata(path).unwrap().len()
}

#[test]
fn c10_compaction_keeps_open_and_recent() {
    let dir = TempDirGuard::new("cm-i06-c10-compact").unwrap();
    let path = dir.path().join("j");
    let mut text = String::new();
    for name in ["pending-a", "pending-b"] {
        text.push_str(&serde_json::to_string(&record(name, Event::Begin, None, None)).unwrap());
        text.push('\n');
    }
    for index in 0..5000 {
        let name = format!("done-{index:04}");
        for item in [
            record(&name, Event::Begin, None, None),
            record(&name, Event::Commit, None, Some("R")),
        ] {
            text.push_str(&serde_json::to_string(&item).unwrap());
            text.push('\n');
        }
    }
    fs::write(&path, &text).unwrap();
    assert!(size(&path) > 1 << 20);
    let mut journal = Journal::open(&path).unwrap();
    journal.compact().unwrap();
    assert!(size(&path) < 1 << 20);
    let states = journal.replay().unwrap();
    let pending: Vec<&str> = states
        .iter()
        .filter(|state| state.status == TxnStatus::Pending)
        .map(|state| state.txn.as_str())
        .collect();
    assert_eq!(pending, ["pending-a", "pending-b"]);
    let committed: Vec<&str> = states
        .iter()
        .filter(|state| state.status == TxnStatus::Committed)
        .map(|state| state.txn.as_str())
        .collect();
    assert_eq!(committed.len(), 256);
    assert_eq!(committed.first(), Some(&"done-4744"));
    assert_eq!(committed.last(), Some(&"done-4999"));
    // The journal stays usable after the file was replaced.
    journal
        .append(&record("after", Event::Begin, None, None))
        .unwrap();
    assert_eq!(status(&journal, "after"), TxnStatus::Pending);
}
