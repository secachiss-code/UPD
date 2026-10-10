//! Один порядок на каждый кадр: декод, личность, права, слот, владелец, операция.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::actions::{Authorizer, action_for};
use super::codec::{self, decode, encode};
use super::core_ops::Workers;
use super::drop::run_as_for;
use super::owner::owned;
use super::peer::PeerIdentity;
use super::protocol::{ControlError, Op, Reply};

pub const MAX_CONCURRENT_OPS: usize = 4;

pub struct OpSlots {
    current: Mutex<usize>,
}

pub struct SlotGuard<'a> {
    slots: &'a OpSlots,
}

impl OpSlots {
    pub fn new() -> Self {
        Self {
            current: Mutex::new(0),
        }
    }

    pub fn try_acquire(&self) -> Result<SlotGuard<'_>, ControlError> {
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *current >= MAX_CONCURRENT_OPS {
            return Err(ControlError::Busy);
        }
        *current += 1;
        drop(current);
        Ok(SlotGuard { slots: self })
    }
}

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        let mut current = self
            .slots
            .current
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *current = current.saturating_sub(1);
    }
}

impl Default for OpSlots {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Deps {
    pub base: PathBuf,
    pub authorizer: Arc<dyn Authorizer>,
    pub workers: Arc<Workers>,
    pub slots: Arc<OpSlots>,
    pub now: fn() -> i64,
    pub nets: Arc<super::net_ops::NetCtx>,
}

pub fn handle(frame: &[u8], peer: &PeerIdentity, deps: &Deps) -> Vec<u8> {
    let request = match decode(frame) {
        Ok(request) => request,
        Err(error) => return encode(&Reply::error("", error)),
    };
    encode(&execute(&request, peer, deps))
}

fn execute(request: &super::protocol::Request, peer: &PeerIdentity, deps: &Deps) -> Reply {
    if let Err(error) = peer.verify() {
        return Reply::error(&request.id, error);
    }
    let action = action_for(request.op.class());
    if let Err(error) = deps.authorizer.authorize(peer, action) {
        return Reply::error(&request.id, error);
    }
    let _slot = match deps.slots.try_acquire() {
        Ok(guard) => guard,
        Err(error) => return Reply::error(&request.id, error),
    };
    let instance = request.op.instance().unwrap_or("journal");
    let owned = match owned(&deps.base, peer.uid, instance) {
        Ok(owned) => owned,
        Err(error) => return Reply::error(&request.id, error),
    };
    let now = (deps.now)();
    let digest = codec::request_digest(request);
    let result = match &request.op {
        Op::WorkerStart { generation, .. } => {
            let run_as = match run_as_for(peer.uid, peer.gid) {
                Ok(run_as) => run_as,
                Err(error) => return Reply::error(&request.id, error),
            };
            deps.workers
                .start(&owned, &run_as, &request.id, &digest, *generation, now)
        }
        Op::WorkerReload {
            generation,
            next_generation,
            ..
        } => deps.workers.reload(
            &owned,
            &request.id,
            &digest,
            *generation,
            *next_generation,
            now,
        ),
        Op::WorkerStop { generation, .. } => {
            deps.workers
                .stop(&owned, &request.id, &digest, *generation, now)
        }
        Op::WorkerStatus { .. } => Ok(deps.workers.status(&owned, &request.id)),
        Op::Reconcile => deps.workers.reconcile(&owned, &request.id, now),
        Op::NetApply { generation, .. } => {
            super::net_ops::apply(&owned, *generation, &request.id, &digest, now, &deps.nets)
        }
        Op::NetRevert { generation, .. } => {
            super::net_ops::revert(&owned, *generation, &request.id, &digest, now, &deps.nets)
        }
        Op::AppLaunch {
            generation,
            program,
            args,
            ..
        } => super::app_ops::launch(
            &owned,
            &request.id,
            *generation,
            program,
            args,
            peer,
            &deps.workers,
            &deps.nets,
            &deps.base,
        ),
    };
    match result {
        Ok(reply) => reply,
        Err(error) => Reply::error(&request.id, error),
    }
}
