//! Шаги транзакции: при отказе компенсация в обратном порядке, обрыв оставляет Pending.

use super::journal::{Event, Journal, Record, TxnState, TxnStatus};
use super::protocol::ControlError;

pub trait Step {
    fn name(&self) -> &'static str;
    fn apply(&mut self) -> Result<(), ControlError>;
    fn compensate(&mut self) -> Result<(), ControlError>;
}

pub struct TxnMeta<'a> {
    pub txn: &'a str,
    pub instance: &'a str,
    pub generation: u64,
    pub digest: &'a str,
}

pub fn run(
    journal: &mut Journal,
    meta: &TxnMeta,
    steps: &mut [Box<dyn Step + '_>],
    reply: &str,
    crash_before: &dyn Fn(&str) -> bool,
    now: i64,
) -> Result<(), ControlError> {
    run_late(
        journal,
        meta,
        steps,
        &|| reply.to_owned(),
        crash_before,
        now,
    )
}

pub fn run_late(
    journal: &mut Journal,
    meta: &TxnMeta,
    steps: &mut [Box<dyn Step + '_>],
    reply: &dyn Fn() -> String,
    crash_before: &dyn Fn(&str) -> bool,
    now: i64,
) -> Result<(), ControlError> {
    journal.append(&record(meta, Event::Begin, None, None, now))?;
    let mut done = Vec::new();
    for (index, step) in steps.iter_mut().enumerate() {
        if crash_before(step.name()) {
            return Err(ControlError::Crashed);
        }
        if let Err(error) = step.apply() {
            return compensate_failed(journal, meta, steps, &done, now, error);
        }
        journal.append(&record(meta, Event::Step, Some(step.name()), None, now))?;
        done.push(index);
    }
    if crash_before("commit") {
        return Err(ControlError::Crashed);
    }
    let text = reply();
    journal.append(&record(meta, Event::Commit, None, Some(&text), now))?;
    Ok(())
}

pub fn reconcile(
    journal: &mut Journal,
    steps_for: &mut dyn FnMut(&TxnState) -> Vec<Box<dyn Step>>,
    now: i64,
) -> Result<u32, ControlError> {
    let pending: Vec<TxnState> = journal
        .replay()?
        .into_iter()
        .filter(|state| state.status == TxnStatus::Pending)
        .collect();
    let mut compensated = 0u32;
    for state in pending {
        let mut steps = steps_for(&state);
        let mut failed = false;
        for step in steps.iter_mut().rev() {
            if step.compensate().is_err() {
                failed = true;
            }
        }
        let meta = TxnMeta {
            txn: &state.txn,
            instance: &state.instance,
            generation: state.generation,
            digest: &state.digest,
        };
        if failed {
            journal.append(&record(&meta, Event::CompensationFailed, None, None, now))?;
        } else {
            journal.append(&record(&meta, Event::Compensated, None, None, now))?;
            compensated = compensated.saturating_add(1);
        }
    }
    Ok(compensated)
}

fn compensate_failed(
    journal: &mut Journal,
    meta: &TxnMeta,
    steps: &mut [Box<dyn Step + '_>],
    done: &[usize],
    now: i64,
    error: ControlError,
) -> Result<(), ControlError> {
    let mut failed = false;
    for index in done.iter().rev() {
        if steps[*index].compensate().is_err() {
            failed = true;
        }
    }
    if failed {
        journal.append(&record(meta, Event::CompensationFailed, None, None, now))?;
        Err(ControlError::Failed)
    } else {
        journal.append(&record(meta, Event::Abort, None, None, now))?;
        Err(error)
    }
}

fn record(
    meta: &TxnMeta,
    event: Event,
    step: Option<&str>,
    reply: Option<&str>,
    now: i64,
) -> Record {
    Record {
        txn: meta.txn.to_owned(),
        event,
        instance: meta.instance.to_owned(),
        generation: meta.generation,
        digest: meta.digest.to_owned(),
        step: step.map(str::to_owned),
        reply: reply.map(str::to_owned),
        at: now,
    }
}
