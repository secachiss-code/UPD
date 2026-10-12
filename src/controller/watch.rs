//! Сторож: внешние интерфейсы, ядра, сеть, затем срок жизни туннеля.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::common::test_mode;
use crate::net::failure::{self, Failure};
use crate::net::host_commands::uplink_commands;
use crate::net::host_plan::HOST_INSTANCE;
use crate::net::observe::Drift;
use crate::net::policy::{FailureMode, Lifetime};
use crate::net::state::{self, TunnelEvent, TunnelState};
use crate::net::{self, TunnelNet};

use super::core_ops::Workers;
use super::host_ops;
use super::net_ops::NetCtx;
use super::owner::{self, Owned};
use super::protocol::{ControlError, Reply, ReplyData};

pub const WATCH_INTERVAL_MS: u64 = 2_000;
pub const EMPTY_GRACE_MS: i64 = 5_000;

#[derive(Clone)]
struct Track {
    state: TunnelState,
    failure: failure::FailureState,
    restart_at_ms: Option<i64>,
    had_holders: bool,
    empty_since_ms: Option<i64>,
    holders: u32,
}

pub struct Watch {
    tracks: BTreeMap<(u32, String), Track>,
    seq: u32,
    proc_root: PathBuf,
}

impl Watch {
    pub fn new() -> Self {
        Self::with_proc_root(PathBuf::from("/proc"))
    }

    pub fn with_proc_root(proc_root: PathBuf) -> Self {
        Self {
            tracks: BTreeMap::new(),
            seq: 0,
            proc_root,
        }
    }

    pub fn note_start(&mut self, uid: u32, instance: &str) {
        let track = self.track(uid, instance);
        track.state = state::next(track.state, TunnelEvent::StartRequested);
        track.failure = failure::FailureState::default();
        track.restart_at_ms = None;
    }

    pub fn note_stop(&mut self, uid: u32, instance: &str) {
        let track = self.track(uid, instance);
        track.state = state::next(track.state, TunnelEvent::StopRequested);
        track.failure = failure::FailureState::default();
        track.restart_at_ms = None;
        track.had_holders = false;
        track.empty_since_ms = None;
        track.holders = 0;
    }

    pub fn note_launch(&mut self, uid: u32, instance: &str) {
        let track = self.track(uid, instance);
        track.had_holders = true;
        track.empty_since_ms = None;
    }

    pub fn state(&self, uid: u32, instance: &str) -> Option<(TunnelState, u8)> {
        self.tracks
            .get(&(uid, instance.to_owned()))
            .map(|track| (track.state, track.failure.restarts))
    }

    pub fn holders(&self, uid: u32, instance: &str) -> u32 {
        self.tracks
            .get(&(uid, instance.to_owned()))
            .map(|track| track.holders)
            .unwrap_or(0)
    }

    pub fn tick(&mut self, base: &Path, workers: &Workers, nets: &Arc<NetCtx>, now_ms: i64) {
        let due = self.due(now_ms);
        self.host_step(base, workers, nets, now_ms);
        self.cores(base, workers, now_ms, &due);
        self.network(workers, nets, now_ms);
        self.lifetime(base, workers, nets, now_ms);
    }

    fn due(&self, now_ms: i64) -> BTreeSet<(u32, String)> {
        self.tracks
            .iter()
            .filter_map(|((uid, instance), track)| {
                track
                    .restart_at_ms
                    .is_some_and(|at| now_ms >= at)
                    .then_some((*uid, instance.clone()))
            })
            .collect()
    }

    fn core_ids(&self, workers: &Workers) -> BTreeSet<(u32, String)> {
        let mut ids: BTreeSet<_> = workers.running().into_iter().collect();
        for ((uid, instance), track) in &self.tracks {
            if track.restart_at_ms.is_some() {
                ids.insert((*uid, instance.clone()));
            }
        }
        ids
    }

    fn host_step(&mut self, base: &Path, workers: &Workers, nets: &NetCtx, now_ms: i64) {
        let Ok(Some(mut state)) = host_ops::load_state(base) else {
            return;
        };
        if state.mode != "tunnel" {
            return;
        }
        if !workers.try_begin(state.uid, HOST_INSTANCE) {
            return;
        }
        let _guard = HostLock {
            workers,
            uid: state.uid,
        };
        let new = {
            let mut query = nets.query.lock().unwrap_or_else(|error| error.into_inner());
            match net::query::observe_uplinks(&mut *query) {
                Ok(uplinks) => uplinks,
                Err(_) => return,
            }
        };
        if new != state.uplinks {
            let host = net::host_plan::host_net(state.uid);
            let commands = uplink_commands(&host, &state.uplinks, &new);
            {
                let mut exec = nets.exec.lock().unwrap_or_else(|error| error.into_inner());
                for command in &commands {
                    let _ = exec.run(command);
                }
            }
            state.uplinks = new;
            let _ = host_ops::save_state(&host_ops::state_path(base), &state);
        }
        let host = net::host_plan::host_net(state.uid);
        let Some(repair) = repair_host_net(nets, &host, &state.uplinks) else {
            return;
        };
        if repair.complete
            && repair
                .drift
                .iter()
                .any(|item| matches!(item, net::host_audit::HostDrift::MissingTun))
        {
            self.track(state.uid, HOST_INSTANCE).restart_at_ms = Some(now_ms);
        }
    }

    fn cores(
        &mut self,
        base: &Path,
        workers: &Workers,
        now_ms: i64,
        due: &BTreeSet<(u32, String)>,
    ) {
        for (uid, instance) in self.core_ids(workers) {
            if !workers.try_begin(uid, &instance) {
                continue;
            }
            self.core(base, workers, uid, &instance, now_ms, due);
            workers.end(uid, &instance);
        }
    }

    fn core(
        &mut self,
        base: &Path,
        workers: &Workers,
        uid: u32,
        instance: &str,
        now_ms: i64,
        due: &BTreeSet<(u32, String)>,
    ) {
        let key = (uid, instance.to_owned());
        if due.contains(&key) {
            self.restart_now(base, workers, uid, instance, now_ms);
            return;
        }
        if self.scheduled_now(uid, instance, now_ms) {
            return;
        }
        let Some(alive) = workers.core_ready(uid, instance) else {
            return;
        };
        if alive {
            let track = self.track(uid, instance);
            if matches!(track.state, TunnelState::Stopped | TunnelState::Failed) {
                track.state = state::next(track.state, TunnelEvent::StartRequested);
            }
            track.state = state::next(track.state, TunnelEvent::CoreReady);
            track.restart_at_ms = None;
            return;
        }
        let detected = {
            let track = self.track(uid, instance);
            if track.restart_at_ms.is_some() || track.state == TunnelState::Failed {
                None
            } else {
                Some(state::next(track.state, TunnelEvent::CoreDown))
            }
        };
        let Some(next_state) = detected else {
            return;
        };
        self.track(uid, instance).state = next_state;
        self.schedule_failure(base, uid, instance, now_ms);
    }

    fn scheduled_now(&self, uid: u32, instance: &str, now_ms: i64) -> bool {
        self.tracks
            .get(&(uid, instance.to_owned()))
            .and_then(|track| track.restart_at_ms)
            .is_some_and(|at| now_ms >= at)
    }

    fn restart_now(
        &mut self,
        base: &Path,
        workers: &Workers,
        uid: u32,
        instance: &str,
        now_ms: i64,
    ) {
        let result = owner::owned(base, uid, instance).and_then(|owned| workers.restart(&owned));
        match result {
            Ok(()) => self.track(uid, instance).restart_at_ms = None,
            Err(_) => self.schedule_failure(base, uid, instance, now_ms),
        }
    }

    fn schedule_failure(&mut self, base: &Path, uid: u32, instance: &str, now_ms: i64) {
        let mode = failure_mode(base, uid, instance);
        let track = self.track(uid, instance);
        let (action, next) = failure::decide(Failure::CoreDown, mode, &track.failure, now_ms);
        track.failure = next;
        match action {
            failure::Action::RestartCore { delay_ms } => {
                track.restart_at_ms = Some(now_ms.saturating_add(delay_ms as i64));
            }
            failure::Action::GiveUp => {
                track.state = TunnelState::Failed;
                track.restart_at_ms = None;
            }
            _ => {}
        }
    }

    fn network(&mut self, workers: &Workers, nets: &NetCtx, _now_ms: i64) {
        let leased = leased(nets);
        let tables: Vec<u32> = leased.iter().map(|(_, _, tunnel)| tunnel.table).collect();
        let observed = {
            let mut query = nets.query.lock().unwrap_or_else(|error| error.into_inner());
            match net::query::observe(&mut *query, &tables) {
                Ok(observed) => observed,
                Err(_) => return,
            }
        };
        let desired: Vec<TunnelNet> = leased.iter().map(|(_, _, tunnel)| tunnel.clone()).collect();
        let drift = net::observe::audit(&desired, &observed);
        for item in &drift {
            if let Some((uid, instance)) = owner_of(&leased, item)
                && workers.try_begin(uid, instance)
            {
                let track = self.track(uid, instance);
                track.state = state::next(track.state, TunnelEvent::DriftFound);
                workers.end(uid, instance);
            }
            let commands = net::observe::repair_commands(item, &desired);
            if commands.is_empty() {
                continue;
            }
            let mut exec = nets.exec.lock().unwrap_or_else(|error| error.into_inner());
            let mut ok = true;
            for command in &commands {
                if exec.run(command).is_err() {
                    ok = false;
                    break;
                }
            }
            drop(exec);
            if ok
                && let Some((uid, instance)) = owner_of(&leased, item)
                && workers.try_begin(uid, instance)
            {
                let track = self.track(uid, instance);
                track.state = state::next(track.state, TunnelEvent::Repaired);
                workers.end(uid, instance);
            }
        }
    }

    fn lifetime(&mut self, base: &Path, workers: &Workers, nets: &Arc<NetCtx>, now_ms: i64) {
        let keys: Vec<(u32, String)> = self.tracks.keys().cloned().collect();
        for (uid, instance) in keys {
            if instance == HOST_INSTANCE || !workers.try_begin(uid, &instance) {
                continue;
            }
            let action = self.lifetime_one(base, nets, uid, &instance, now_ms);
            if action {
                match self.stop_instance(base, workers, nets, uid, &instance, now_ms) {
                    Ok(()) => {
                        let track = self.track(uid, &instance);
                        track.state = TunnelState::Stopped;
                        track.had_holders = false;
                        track.empty_since_ms = None;
                        track.holders = 0;
                        track.restart_at_ms = None;
                    }
                    Err(_) => {
                        let track = self.track(uid, &instance);
                        track.state = TunnelState::Blocked;
                        track.empty_since_ms = None;
                    }
                }
            }
            workers.end(uid, &instance);
        }
    }

    fn lifetime_one(
        &mut self,
        base: &Path,
        nets: &Arc<NetCtx>,
        uid: u32,
        instance: &str,
        now_ms: i64,
    ) -> bool {
        let Ok(owned) = owner::owned(base, uid, instance) else {
            return false;
        };
        let policy = owner::read_policy(&owned).unwrap_or_default();
        let holders = holder_count(base, nets.as_ref(), uid, &owned, &self.proc_root);
        let track = self.track(uid, instance);
        track.holders = holders as u32;
        if policy.lifetime != Lifetime::Refcount || !track.had_holders {
            track.empty_since_ms = None;
            return false;
        }
        if holders > 0 {
            track.empty_since_ms = None;
            return false;
        }
        match track.empty_since_ms {
            None => {
                track.empty_since_ms = Some(now_ms);
                false
            }
            Some(since) if now_ms.saturating_sub(since) >= EMPTY_GRACE_MS => true,
            Some(_) => false,
        }
    }

    fn stop_instance(
        &mut self,
        base: &Path,
        workers: &Workers,
        nets: &Arc<NetCtx>,
        uid: u32,
        instance: &str,
        now_ms: i64,
    ) -> Result<(), ControlError> {
        let owned = owner::owned(base, uid, instance)?;
        let generations = super::registry::Generations::load(&owned.generations_path())?;
        let now = journal_time(now_ms);
        if let Some(generation) = generations.current(instance) {
            let txn = self.alloc_txn("stop", now_ms);
            match workers.stop(&owned, &txn, &txn, generation, now) {
                Ok(_) | Err(ControlError::NotRunning) => {}
                Err(error) => return Err(error),
            }
        }
        if let Some(generation) = generations.current(&format!("net:{instance}")) {
            let txn = self.alloc_txn("revert", now_ms);
            match super::net_ops::revert(&owned, generation, &txn, &txn, now, nets) {
                Ok(_) | Err(ControlError::NotRunning) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn alloc_txn(&mut self, kind: &str, now_ms: i64) -> String {
        self.seq = self.seq.wrapping_add(1);
        auto_txn(kind, now_ms, self.seq)
    }

    fn track(&mut self, uid: u32, instance: &str) -> &mut Track {
        self.tracks
            .entry((uid, instance.to_owned()))
            .or_insert_with(|| Track {
                state: TunnelState::Stopped,
                failure: failure::FailureState::default(),
                restart_at_ms: None,
                had_holders: false,
                empty_since_ms: None,
                holders: 0,
            })
    }
}

impl Default for Watch {
    fn default() -> Self {
        Self::new()
    }
}

pub fn state_name(state: TunnelState) -> &'static str {
    match state {
        TunnelState::Stopped => "stopped",
        TunnelState::Starting => "starting",
        TunnelState::Up => "up",
        TunnelState::Degraded => "degraded",
        TunnelState::Blocked => "blocked",
        TunnelState::Failed => "failed",
    }
}

fn failure_mode(base: &Path, uid: u32, instance: &str) -> FailureMode {
    owner::owned(base, uid, instance)
        .and_then(|owned| owner::read_policy(&owned))
        .map(|policy| policy.failure)
        .unwrap_or(FailureMode::Block)
}

fn owner_of<'a>(leased: &'a [(u32, String, TunnelNet)], drift: &Drift) -> Option<(u32, &'a str)> {
    let index = match drift {
        Drift::MissingBlackhole(index)
        | Drift::MissingRule(index)
        | Drift::MissingTunRoute(index)
        | Drift::MissingVeth(index)
        | Drift::MissingTun(index)
        | Drift::MissingNetns(index) => *index,
        Drift::OrphanVeth(_) | Drift::OrphanRule(_) => return None,
    };
    leased
        .iter()
        .find(|(_, _, tunnel)| tunnel.index == index)
        .map(|(uid, instance, _)| (*uid, instance.as_str()))
}

fn leased(nets: &NetCtx) -> Vec<(u32, String, TunnelNet)> {
    let Ok(registry) = crate::core::leases::LeaseRegistry::open(&nets.lease_dir) else {
        return Vec::new();
    };
    let Ok(leases) = registry.list() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for lease in leases
        .into_iter()
        .filter(|lease| lease.kind == crate::core::leases::ResourceKind::Tunnel)
    {
        let Some((uid, instance)) = parse_holder(&lease.holder) else {
            continue;
        };
        let Ok(index) = lease.value.parse::<u8>() else {
            continue;
        };
        let Ok(tunnel) = net::tunnel_net(index, uid) else {
            continue;
        };
        found.push((uid, instance, tunnel));
    }
    found
}

fn parse_holder(holder: &str) -> Option<(u32, String)> {
    let rest = holder.strip_prefix('u')?;
    let (uid, instance) = rest.split_once('-')?;
    Some((uid.parse().ok()?, instance.to_owned()))
}

fn holder_count(base: &Path, nets: &NetCtx, uid: u32, owned: &Owned, proc_root: &Path) -> usize {
    let Some(index) = super::net_ops::current_index(nets, owned).ok().flatten() else {
        return 0;
    };
    let path = netns_root(base).join(format!("cm-{index}"));
    let Some(id) = crate::app::instance::netns_of_path(&path) else {
        return 0;
    };
    let procs = crate::app::instance::scan(proc_root);
    crate::app::instance::holders(&procs, id, uid).len()
}

fn netns_root(base: &Path) -> PathBuf {
    if test_mode() {
        base.join("netns")
    } else {
        PathBuf::from("/run/netns")
    }
}

/// "auto-<kind>-<now_ms>-<seq>"; kind — "stop" или "revert".
pub fn auto_txn(kind: &str, now_ms: i64, seq: u32) -> String {
    let now = if now_ms < 0 { 0 } else { now_ms };
    format!("auto-{kind}-{now}-{seq}")
}

fn journal_time(now_ms: i64) -> i64 {
    if now_ms < 0 { 0 } else { now_ms / 1000 }
}

struct HostLock<'a> {
    workers: &'a Workers,
    uid: u32,
}

impl Drop for HostLock<'_> {
    fn drop(&mut self) {
        self.workers.end(self.uid, HOST_INSTANCE);
    }
}

pub(crate) struct HostRepair {
    drift: Vec<net::host_audit::HostDrift>,
    complete: bool,
}

pub(crate) fn repair_host_net(
    nets: &NetCtx,
    host: &net::host_plan::HostNet,
    uplinks: &[net::host_plan::Uplink],
) -> Option<HostRepair> {
    let (observed, rules6, routes6, firewall) = {
        let mut query = nets.query.lock().unwrap_or_else(|error| error.into_inner());
        let observed = net::query::observe(&mut *query, &[host.table, host.sink_table]).ok()?;
        let rules_text = query.query(net::Program::Ip, &["-j", "-6", "rule"]).ok()?;
        let rules6 = net::parse_rules(&rules_text).ok()?;
        let table = host.sink_table.to_string();
        let routes_text = query
            .query(
                net::Program::Ip,
                &["-j", "-6", "route", "show", "table", &table],
            )
            .ok()?;
        let routes6 = net::parse_routes(&routes_text).ok()?;
        let firewall = query
            .query(net::Program::Nft, &["list", "table", "ip", "cm_host"])
            .is_ok();
        (observed, rules6, routes6, firewall)
    };
    let drift = net::host_audit::host_audit(host, uplinks, &observed, &rules6, &routes6, firewall);
    if drift.is_empty() {
        return Some(HostRepair {
            drift,
            complete: true,
        });
    }
    let commands = net::host_audit::host_repair_commands(host, uplinks, &drift);
    let mut exec = nets.exec.lock().unwrap_or_else(|error| error.into_inner());
    for command in &commands {
        if exec.run(command).is_err() {
            return Some(HostRepair {
                drift,
                complete: false,
            });
        }
    }
    Some(HostRepair {
        drift,
        complete: true,
    })
}

pub fn overview(
    base: &Path,
    uid: u32,
    workers: &Workers,
    nets: &NetCtx,
    id: &str,
) -> Result<Reply, ControlError> {
    let owned = owner::owned(base, uid, HOST_INSTANCE)?;
    let generations = super::registry::Generations::load(&owned.generations_path())?;
    let host_status = host_ops::status(&owned, id);
    let (mode, generation, uplinks) = match host_status.data {
        Some(ReplyData::Host {
            mode,
            generation,
            uplinks,
        }) => (mode, generation, uplinks),
        _ => ("off".to_owned(), None, Vec::new()),
    };
    let names = instance_names(base, uid);
    let (host_state, host_restarts, rows) = watch_rows(workers, uid, &names);
    let state = if mode == "off" {
        "stopped"
    } else {
        state_name(host_state)
    };
    let host = super::protocol::HostRow {
        mode,
        generation,
        uplinks,
        state: state.to_owned(),
        restarts: u32::from(host_restarts),
        core_pid: workers.core_pid(uid, HOST_INSTANCE),
    };
    let mut tunnels = Vec::new();
    for row in rows {
        let netns = owner::owned(base, uid, &row.name)
            .ok()
            .and_then(|owned| super::net_ops::current_index(nets, &owned).ok().flatten())
            .map(|index| format!("cm-{index}"));
        tunnels.push(super::protocol::TunnelRow {
            instance: row.name.clone(),
            state: state_name(row.state).to_owned(),
            restarts: u32::from(row.restarts),
            holders: row.holders,
            worker_generation: generations.current(&row.name),
            net_generation: generations.current(&format!("net:{}", row.name)),
            netns,
            core_pid: workers.core_pid(uid, &row.name),
        });
    }
    Ok(Reply::ok(id, Some(ReplyData::Overview { host, tunnels })))
}

struct RowSnap {
    name: String,
    state: TunnelState,
    restarts: u8,
    holders: u32,
}

/// Сторож нужен только для state и holders. Блокировка отпускается до core_pid.
fn watch_rows(workers: &Workers, uid: u32, names: &[String]) -> (TunnelState, u8, Vec<RowSnap>) {
    let watch = workers.watch_handle();
    let guard = watch
        .as_ref()
        .map(|watch| watch.lock().unwrap_or_else(|error| error.into_inner()));
    let (host_state, host_restarts) = guard.as_ref().map_or((TunnelState::Stopped, 0), |watch| {
        watch
            .state(uid, HOST_INSTANCE)
            .unwrap_or((TunnelState::Stopped, 0))
    });
    let rows = names
        .iter()
        .map(|name| {
            let (state, restarts) = guard.as_ref().map_or((TunnelState::Stopped, 0), |watch| {
                watch.state(uid, name).unwrap_or((TunnelState::Stopped, 0))
            });
            let holders = guard
                .as_ref()
                .map(|watch| watch.holders(uid, name))
                .unwrap_or(0);
            RowSnap {
                name: name.clone(),
                state,
                restarts,
                holders,
            }
        })
        .collect();
    (host_state, host_restarts, rows)
}

fn instance_names(base: &Path, uid: u32) -> Vec<String> {
    let dir = base.join(format!("u{uid}")).join("instances");
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name == HOST_INSTANCE || crate::core::instance::InstanceId::new(&name).is_err() {
            continue;
        }
        names.push(name);
    }
    names.sort();
    names.truncate(super::protocol::MAX_OVERVIEW_ROWS);
    names
}

pub fn tunnel_reply(watch: &Watch, uid: u32, instance: &str, id: &str) -> super::protocol::Reply {
    let (state, restarts) = watch
        .state(uid, instance)
        .unwrap_or((TunnelState::Stopped, 0));
    super::protocol::Reply::ok(
        id,
        Some(ReplyData::Tunnel {
            state: state_name(state).to_owned(),
            restarts: u32::from(restarts),
            holders: watch.holders(uid, instance),
        }),
    )
}
