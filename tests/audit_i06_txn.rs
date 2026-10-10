//! C11, C12, C13: compensation order, replay of a repeated request, generations.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::sync::{Arc, Mutex};

use cm::common::contract_fixtures::TempDirGuard;
use cm::controller::journal::{Event, Journal, Record, TxnState, TxnStatus};
use cm::controller::protocol::ControlError;
use cm::controller::registry::{Generations, Replay, check_running, check_start, replay_or_fresh};
use cm::controller::txn::{self, Step, TxnMeta};

type Calls = Arc<Mutex<Vec<String>>>;

struct Probe {
    name: &'static str,
    calls: Calls,
    apply: Result<(), ControlError>,
    compensate: Result<(), ControlError>,
}

impl Step for Probe {
    fn name(&self) -> &'static str {
        self.name
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{}.apply", self.name));
        self.apply
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{}.compensate", self.name));
        self.compensate
    }
}

fn steps(
    calls: &Calls,
    failing: Option<(&str, ControlError)>,
    bad_undo: Option<&str>,
) -> Vec<Box<dyn Step>> {
    ["A", "B", "C"]
        .into_iter()
        .map(|name| -> Box<dyn Step> {
            Box::new(Probe {
                name,
                calls: Arc::clone(calls),
                apply: match failing {
                    Some((step, error)) if step == name => Err(error),
                    _ => Ok(()),
                },
                compensate: if bad_undo == Some(name) {
                    Err(ControlError::Failed)
                } else {
                    Ok(())
                },
            })
        })
        .collect()
}

const META: TxnMeta<'static> = TxnMeta {
    txn: "txn-0001",
    instance: "browser",
    generation: 1,
    digest: "d",
};

fn events(journal: &Journal) -> Vec<String> {
    let path = journal_path(journal);
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| {
            let record: Record = serde_json::from_str(line).unwrap();
            match record.step {
                Some(step) => format!("{:?} {step}", record.event),
                None => format!("{:?}", record.event),
            }
        })
        .collect()
}

thread_local! {
    static PATH: std::cell::RefCell<std::path::PathBuf> = const { std::cell::RefCell::new(std::path::PathBuf::new()) };
}

fn journal_path(_journal: &Journal) -> std::path::PathBuf {
    PATH.with(|path| path.borrow().clone())
}

fn open(dir: &TempDirGuard) -> Journal {
    let path = dir.path().join("journal.jsonl");
    PATH.with(|slot| *slot.borrow_mut() = path.clone());
    Journal::open(&path).unwrap()
}

fn run(
    journal: &mut Journal,
    calls: &Calls,
    failing: Option<(&str, ControlError)>,
    bad_undo: Option<&str>,
    crash: Option<&str>,
) -> Result<(), ControlError> {
    let mut list: Vec<Box<dyn Step>> = steps(calls, failing, bad_undo);
    txn::run(
        journal,
        &META,
        &mut list,
        "R",
        &|name| crash == Some(name),
        1,
    )
}

fn status(journal: &Journal) -> TxnStatus {
    journal.find("txn-0001").unwrap().unwrap().status
}

#[test]
fn c11_success_and_reverse_compensation() {
    let dir = TempDirGuard::new("cm-i06-c11-a").unwrap();
    let calls = Calls::default();
    let mut journal = open(&dir);
    assert_eq!(run(&mut journal, &calls, None, None, None), Ok(()));
    assert_eq!(*calls.lock().unwrap(), ["A.apply", "B.apply", "C.apply"]);
    assert_eq!(
        events(&journal),
        ["Begin", "Step A", "Step B", "Step C", "Commit"]
    );

    let dir = TempDirGuard::new("cm-i06-c11-b").unwrap();
    let calls = Calls::default();
    let mut journal = open(&dir);
    assert_eq!(
        run(
            &mut journal,
            &calls,
            Some(("B", ControlError::Timeout)),
            None,
            None
        ),
        Err(ControlError::Timeout)
    );
    assert_eq!(
        *calls.lock().unwrap(),
        ["A.apply", "B.apply", "A.compensate"]
    );
    assert_eq!(events(&journal), ["Begin", "Step A", "Abort"]);

    let dir = TempDirGuard::new("cm-i06-c11-c").unwrap();
    let calls = Calls::default();
    let mut journal = open(&dir);
    assert_eq!(
        run(
            &mut journal,
            &calls,
            Some(("C", ControlError::Failed)),
            None,
            None
        ),
        Err(ControlError::Failed)
    );
    assert_eq!(calls.lock().unwrap()[3..], ["B.compensate", "A.compensate"]);

    let dir = TempDirGuard::new("cm-i06-c11-d").unwrap();
    let calls = Calls::default();
    let mut journal = open(&dir);
    assert_eq!(
        run(
            &mut journal,
            &calls,
            Some(("B", ControlError::Timeout)),
            Some("A"),
            None
        ),
        Err(ControlError::Failed)
    );
    assert_eq!(
        events(&journal).last().map(String::as_str),
        Some("CompensationFailed")
    );
    assert_eq!(status(&journal), TxnStatus::Dirty);
}

fn reconcile(journal: &mut Journal, calls: &Calls) -> u32 {
    let calls = Arc::clone(calls);
    txn::reconcile(
        journal,
        &mut |state: &TxnState| {
            let all = steps(&calls, None, None);
            all.into_iter()
                .filter(|step| state.steps_done.iter().any(|done| done == step.name()))
                .collect()
        },
        2,
    )
    .unwrap()
}

#[test]
fn c11_crash_leaves_a_trace_reconcile_finishes() {
    let dir = TempDirGuard::new("cm-i06-c11-crash").unwrap();
    let calls = Calls::default();
    let mut journal = open(&dir);
    assert_eq!(
        run(&mut journal, &calls, None, None, Some("B")),
        Err(ControlError::Crashed)
    );
    assert_eq!(*calls.lock().unwrap(), ["A.apply"]);
    assert_eq!(events(&journal), ["Begin", "Step A"]);
    assert_eq!(status(&journal), TxnStatus::Pending);
    calls.lock().unwrap().clear();
    assert_eq!(reconcile(&mut journal, &calls), 1);
    assert_eq!(*calls.lock().unwrap(), ["A.compensate"]);
    assert_eq!(status(&journal), TxnStatus::Aborted);
    calls.lock().unwrap().clear();
    assert_eq!(reconcile(&mut journal, &calls), 0);
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn c11_every_crash_point_is_reconciled() {
    for point in ["A", "B", "C", "commit"] {
        let dir = TempDirGuard::new("cm-i06-c11-points").unwrap();
        let calls = Calls::default();
        let mut journal = open(&dir);
        assert_eq!(
            run(&mut journal, &calls, None, None, Some(point)),
            Err(ControlError::Crashed),
            "{point}"
        );
        if point == "commit" {
            assert_eq!(
                journal.find("txn-0001").unwrap().unwrap().steps_done,
                ["A", "B", "C"]
            );
        }
        calls.lock().unwrap().clear();
        reconcile(&mut journal, &calls);
        if point == "commit" {
            assert_eq!(
                *calls.lock().unwrap(),
                ["C.compensate", "B.compensate", "A.compensate"]
            );
        }
        assert!(
            journal
                .replay()
                .unwrap()
                .iter()
                .all(|state| state.status != TxnStatus::Pending),
            "{point}"
        );
    }
}

fn record(event: Event, reply: Option<&str>) -> Record {
    Record {
        txn: "txn-0001".to_owned(),
        event,
        instance: "browser".to_owned(),
        generation: 1,
        digest: "d".to_owned(),
        step: None,
        reply: reply.map(str::to_owned),
        at: 1,
    }
}

#[test]
fn c12_repeated_request_is_not_executed_again() {
    let dir = TempDirGuard::new("cm-i06-c12").unwrap();
    let mut journal = open(&dir);
    assert!(matches!(
        replay_or_fresh(&journal, "txn-0001", "d"),
        Ok(Replay::Fresh)
    ));
    journal.append(&record(Event::Begin, None)).unwrap();
    assert_eq!(
        replay_or_fresh(&journal, "txn-0001", "d").err(),
        Some(ControlError::Conflict)
    );
    journal.append(&record(Event::Commit, Some("R"))).unwrap();
    match replay_or_fresh(&journal, "txn-0001", "d") {
        Ok(Replay::Stored(reply)) => assert_eq!(reply, "R"),
        _ => panic!("stored reply expected"),
    }
    assert_eq!(
        replay_or_fresh(&journal, "txn-0001", "other").err(),
        Some(ControlError::Conflict)
    );
}

#[test]
fn c13_generations() {
    assert_eq!(check_start(None, 1), Ok(()));
    assert_eq!(check_start(Some(3), 3), Ok(()));
    assert_eq!(check_start(Some(3), 4), Ok(()));
    assert_eq!(
        check_start(Some(3), 2),
        Err(ControlError::GenerationMismatch)
    );
    assert_eq!(check_running(None, 1), Err(ControlError::NotRunning));
    assert_eq!(check_running(Some(3), 3), Ok(()));
    assert_eq!(
        check_running(Some(3), 2),
        Err(ControlError::GenerationMismatch)
    );
    assert_eq!(
        check_running(Some(3), 4),
        Err(ControlError::GenerationMismatch)
    );

    let dir = TempDirGuard::new("cm-i06-c13").unwrap();
    let path = dir.path().join("generations.json");
    fs::write(dir.path().join("generations.json.tmp"), b"garbage").unwrap();
    let mut generations = Generations::load(&path).unwrap();
    assert_eq!(generations.current("browser"), None);
    generations.set("browser", 7).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    assert_eq!(
        Generations::load(&path).unwrap().current("browser"),
        Some(7)
    );
    let mut again = Generations::load(&path).unwrap();
    again.clear("browser").unwrap();
    assert_eq!(Generations::load(&path).unwrap().current("browser"), None);
}
