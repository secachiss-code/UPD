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
use crate::core::instance::{InstanceOwner, InstanceRoot};

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

struct BusyGuard<'a> {
    workers: &'a Workers,
    uid: u32,
    instance: &'a str,
}

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.workers.end(self.uid, self.instance);
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
    // uid сокета проверяется раньше авторизатора: чужой кадр не доходит до pkcheck и до сети.
    if matches!(request.op, super::protocol::Op::Teardown) {
        if peer.uid != 0 {
            return Reply::error(&request.id, ControlError::Denied);
        }
        return teardown_reply(request, deps);
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
    let _busy = if request.op.instance().is_some() {
        if !deps.workers.try_begin(peer.uid, instance) {
            return Reply::error(&request.id, ControlError::Busy);
        }
        Some(BusyGuard {
            workers: &deps.workers,
            uid: peer.uid,
            instance,
        })
    } else {
        None
    };
    let result = match &request.op {
        Op::InstancePrepare { .. } => InstanceRoot::new(&owned.root)
            .create_owned(
                &owned.instance,
                InstanceOwner {
                    uid: peer.uid,
                    gid: peer.gid,
                },
            )
            .map(|_| Reply::ok(&request.id, None))
            .map_err(|_| ControlError::Failed),
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
            env,
            session,
            ..
        } => super::app_ops::launch(
            &owned,
            &request.id,
            *generation,
            program,
            args,
            env,
            session.as_deref(),
            peer,
            &deps.workers,
            &deps.nets,
            &deps.base,
        ),
        Op::AppList { .. } => {
            super::app_ops::list(&owned, &request.id, peer, &deps.nets, &deps.base)
        }
        Op::HostApply { generation, mode } => super::host_ops::apply(
            &owned,
            *generation,
            mode,
            &request.id,
            &digest,
            now,
            &deps.nets,
        ),
        Op::HostRevert { generation } => super::host_ops::revert(
            &owned,
            *generation,
            &request.id,
            &digest,
            now,
            &deps.nets,
            &deps.workers,
        ),
        Op::HostStatus => Ok(super::host_ops::status(&owned, &request.id)),
        Op::Overview => {
            super::watch::overview(&deps.base, peer.uid, &deps.workers, &deps.nets, &request.id)
        }
        Op::Teardown => Ok(teardown_reply(request, deps)),
        Op::TunnelState { instance } => {
            let watch = deps.workers.watch_handle();
            Ok(match watch {
                Some(watch) => {
                    let guard = watch.lock().unwrap_or_else(|error| error.into_inner());
                    super::watch::tunnel_reply(&guard, peer.uid, instance, &request.id)
                }
                None => super::protocol::Reply::ok(
                    &request.id,
                    Some(super::protocol::ReplyData::Tunnel {
                        state: "stopped".to_owned(),
                        restarts: 0,
                        holders: 0,
                    }),
                ),
            })
        }
    };
    match result {
        Ok(reply) => reply,
        Err(error) => Reply::error(&request.id, error),
    }
}

fn teardown_reply(request: &super::protocol::Request, deps: &Deps) -> Reply {
    let report = super::teardown::teardown_all(&deps.base, &deps.workers, &deps.nets);
    Reply::ok(
        &request.id,
        Some(super::protocol::ReplyData::Teardown {
            cores: report.cores,
            nets: report.nets,
            host: report.host,
            errors: report.errors,
        }),
    )
}
