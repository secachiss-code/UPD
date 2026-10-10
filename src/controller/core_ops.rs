//! Жизненный цикл worker как транзакции журнала владельца.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::drop::RunAs;
use super::journal::{Journal, TxnState};
use super::owner::{Owned, read_owned_config};
use super::peer::parse_start_time;
use super::protocol::{ControlError, Reply, ReplyData};
use super::registry::{self, Generations, Replay};
use super::txn::{self, Step, TxnMeta};
use crate::core::adapter::{
    ApiState, CoreAdapter, CoreConfig, CoreError, CoreReadiness, RemoteState, RouteState,
};
use crate::core::instance::InstanceRoot;
use crate::core::leases::LeaseRegistry;
use crate::core::mihomo::lifecycle::MihomoWorker;

pub const MAX_INSTANCES_PER_UID: usize = 16;

pub trait AdapterFactory: Send + Sync {
    fn make(
        &self,
        owned: &Owned,
        run_as: &RunAs,
        tunnel: Option<&crate::net::TunnelNet>,
    ) -> Box<dyn CoreAdapter + Send>;
    fn lease_dir(&self) -> Option<&Path> {
        None
    }
}

pub struct MihomoFactory {
    pub binary: PathBuf,
    pub lease_dir: PathBuf,
}

impl AdapterFactory for MihomoFactory {
    fn make(
        &self,
        owned: &Owned,
        run_as: &RunAs,
        tunnel: Option<&crate::net::TunnelNet>,
    ) -> Box<dyn CoreAdapter + Send> {
        let mut worker = MihomoWorker::new(
            owned.instance.clone(),
            InstanceRoot::new(&owned.root),
            &self.lease_dir,
            &self.binary,
        );
        worker.set_run_as(run_as.clone());
        if let Some(tunnel) = tunnel {
            worker.set_tunnel_net(tunnel.clone());
        }
        Box::new(worker)
    }

    fn lease_dir(&self) -> Option<&Path> {
        Some(&self.lease_dir)
    }
}

struct Running {
    adapter: Box<dyn CoreAdapter + Send>,
    generation: u64,
    config: CoreConfig,
}

enum Slot {
    Pending,
    Ready(Running),
}

type Key = (u32, String);
type Slots = Arc<Mutex<HashMap<Key, Slot>>>;

pub struct Workers {
    factory: Arc<dyn AdapterFactory>,
    slots: Slots,
    lease_dir: Option<PathBuf>,
    net: Mutex<Option<Arc<super::net_ops::NetCtx>>>,
}

impl Workers {
    pub fn new(factory: Arc<dyn AdapterFactory>) -> Self {
        let lease_dir = factory.lease_dir().map(Path::to_owned);
        Self {
            factory,
            slots: Arc::new(Mutex::new(HashMap::new())),
            lease_dir,
            net: Mutex::new(None),
        }
    }

    pub fn set_net(&self, ctx: Arc<super::net_ops::NetCtx>) {
        *self.net.lock().unwrap_or_else(|error| error.into_inner()) = Some(ctx);
    }

    pub fn start(
        &self,
        owned: &Owned,
        run_as: &RunAs,
        txn: &str,
        digest: &str,
        generation: u64,
        now: i64,
    ) -> Result<Reply, ControlError> {
        let mut journal = Journal::open(&owned.journal_path())?;
        let generations = Generations::load(&owned.generations_path())?;
        if let Replay::Stored(text) = registry::replay_or_fresh(&journal, txn, digest)? {
            return parse_reply(&text);
        }
        registry::check_start(generations.current(owned.instance.as_str()), generation)?;
        self.ensure_free(owned)?;
        let config = read_owned_config(owned, generation)?;
        self.reserve(owned)?;
        let held = Arc::new(Mutex::new(None));
        let pid_path = pid_path(owned);
        let mut steps: Vec<Box<dyn Step + '_>> = vec![
            Box::new(AdapterStart {
                factory: Arc::clone(&self.factory),
                owned: owned.clone(),
                run_as: run_as.clone(),
                config: config.clone(),
                held: Arc::clone(&held),
                pid_path: pid_path.clone(),
                tunnel: self
                    .lease_dir
                    .as_deref()
                    .and_then(|dir| leased_tunnel(dir, owned)),
            }),
            Box::new(RecordGeneration {
                path: owned.generations_path(),
                instance: owned.instance.as_str().to_owned(),
                generation,
                previous: None,
            }),
        ];
        let reply = Reply::ok(txn, Some(ReplyData::Started { generation }));
        let text = serde_json::to_string(&reply).map_err(|_| ControlError::Failed)?;
        let result = txn::run(
            &mut journal,
            &meta(txn, owned, generation, digest),
            &mut steps,
            &text,
            &crash_point,
            now,
        );
        let adapter = held
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if result.is_ok() {
            if let Some(adapter) = adapter {
                self.slots
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .insert(
                        key(owned),
                        Slot::Ready(Running {
                            adapter,
                            generation,
                            config,
                        }),
                    );
            } else {
                self.clear_slot(owned);
                return Err(ControlError::Failed);
            }
        } else {
            self.clear_slot(owned);
        }
        result?;
        Ok(reply)
    }

    pub fn reload(
        &self,
        owned: &Owned,
        txn: &str,
        digest: &str,
        generation: u64,
        next: u64,
        now: i64,
    ) -> Result<Reply, ControlError> {
        let mut journal = Journal::open(&owned.journal_path())?;
        let generations = Generations::load(&owned.generations_path())?;
        if let Replay::Stored(text) = registry::replay_or_fresh(&journal, txn, digest)? {
            return parse_reply(&text);
        }
        registry::check_running(generations.current(owned.instance.as_str()), generation)?;
        let config = read_owned_config(owned, next)?;
        let previous = self.take_ready(owned)?;
        let held = Arc::new(Mutex::new(Some(previous.adapter)));
        let mut steps: Vec<Box<dyn Step + '_>> = vec![
            Box::new(AdapterReload {
                held: Arc::clone(&held),
                next: config.clone(),
                previous: previous.config.clone(),
            }),
            Box::new(RecordGeneration {
                path: owned.generations_path(),
                instance: owned.instance.as_str().to_owned(),
                generation: next,
                previous: Some(generation),
            }),
        ];
        let reply = Reply::ok(txn, Some(ReplyData::Started { generation: next }));
        let text = serde_json::to_string(&reply).map_err(|_| ControlError::Failed)?;
        let result = txn::run(
            &mut journal,
            &meta(txn, owned, next, digest),
            &mut steps,
            &text,
            &crash_point,
            now,
        );
        let adapter = held
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        if result.is_ok() {
            if let Some(adapter) = adapter {
                slots.insert(
                    key(owned),
                    Slot::Ready(Running {
                        adapter,
                        generation: next,
                        config,
                    }),
                );
            }
        } else if let Some(adapter) = adapter {
            slots.insert(
                key(owned),
                Slot::Ready(Running {
                    adapter,
                    generation: previous.generation,
                    config: previous.config,
                }),
            );
        } else {
            slots.remove(&key(owned));
        }
        result?;
        Ok(reply)
    }

    pub fn stop(
        &self,
        owned: &Owned,
        txn: &str,
        digest: &str,
        generation: u64,
        now: i64,
    ) -> Result<Reply, ControlError> {
        let mut journal = Journal::open(&owned.journal_path())?;
        let generations = Generations::load(&owned.generations_path())?;
        if let Replay::Stored(text) = registry::replay_or_fresh(&journal, txn, digest)? {
            return parse_reply(&text);
        }
        registry::check_running(generations.current(owned.instance.as_str()), generation)?;
        let previous = self.take_ready(owned)?;
        let held = Arc::new(Mutex::new(Some(previous.adapter)));
        let mut steps: Vec<Box<dyn Step + '_>> = vec![
            Box::new(AdapterStop {
                held: Arc::clone(&held),
                pid_path: pid_path(owned),
                uid: owned.uid,
            }),
            Box::new(ClearGeneration {
                path: owned.generations_path(),
                instance: owned.instance.as_str().to_owned(),
                previous: None,
            }),
        ];
        let reply = Reply::ok(txn, None);
        let text = serde_json::to_string(&reply).map_err(|_| ControlError::Failed)?;
        let result = txn::run(
            &mut journal,
            &meta(txn, owned, generation, digest),
            &mut steps,
            &text,
            &crash_point,
            now,
        );
        let adapter = held
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        if let Err(error) = result {
            if let Some(adapter) = adapter {
                slots.insert(
                    key(owned),
                    Slot::Ready(Running {
                        adapter,
                        generation: previous.generation,
                        config: previous.config,
                    }),
                );
            } else {
                slots.remove(&key(owned));
            }
            return Err(error);
        }
        slots.remove(&key(owned));
        Ok(reply)
    }

    pub fn status(&self, owned: &Owned, txn: &str) -> Reply {
        let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        let Some(Slot::Ready(running)) = slots.get_mut(&key(owned)) else {
            return Reply::ok(txn, Some(down_status()));
        };
        // Запись остаётся и после гибели процесса: её снимает `worker_stop` с этим поколением.
        let alive = running.adapter.alive();
        let readiness = if alive {
            running.adapter.health().unwrap_or(CoreReadiness::DOWN)
        } else {
            CoreReadiness::DOWN
        };
        Reply::ok(
            txn,
            Some(ReplyData::Status {
                running: alive,
                generation: Some(running.generation),
                api: api_name(readiness.api).to_owned(),
                route: route_name(readiness.route).to_owned(),
                remote: remote_name(readiness.remote).to_owned(),
            }),
        )
    }

    pub fn reconcile(&self, owned: &Owned, txn: &str, now: i64) -> Result<Reply, ControlError> {
        let mut journal = Journal::open(&owned.journal_path())?;
        let slots = Arc::clone(&self.slots);
        let lease_dir = self.lease_dir.clone();
        let root = owned.root.clone();
        let generations = owned.generations_path();
        let uid = owned.uid;
        let net = self
            .net
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let compensated = txn::reconcile(
            &mut journal,
            &mut |state| {
                if let Some(net) = &net
                    && let Some(steps) = super::net_ops::compensate_steps(state, net, uid)
                {
                    return steps;
                }
                reconcile_steps(&slots, &lease_dir, &root, &generations, uid, state)
            },
            now,
        )?;
        Ok(Reply::ok(txn, Some(ReplyData::Reconciled { compensated })))
    }

    pub fn stop_all(&self) {
        let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        let keys: Vec<Key> = slots.keys().cloned().collect();
        for key in keys {
            if let Some(Slot::Ready(mut running)) = slots.remove(&key) {
                let _ = running.adapter.stop();
            }
        }
    }

    fn ensure_free(&self, owned: &Owned) -> Result<(), ControlError> {
        let slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        if slots.contains_key(&key(owned)) {
            return Err(ControlError::Conflict);
        }
        if uid_count(&slots, owned.uid) >= MAX_INSTANCES_PER_UID {
            return Err(ControlError::Quota);
        }
        Ok(())
    }

    fn reserve(&self, owned: &Owned) -> Result<(), ControlError> {
        let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        if slots.contains_key(&key(owned)) {
            return Err(ControlError::Conflict);
        }
        if uid_count(&slots, owned.uid) >= MAX_INSTANCES_PER_UID {
            return Err(ControlError::Quota);
        }
        slots.insert(key(owned), Slot::Pending);
        Ok(())
    }

    fn clear_slot(&self, owned: &Owned) {
        self.slots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&key(owned));
    }

    fn take_ready(&self, owned: &Owned) -> Result<Running, ControlError> {
        let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        match slots.remove(&key(owned)) {
            Some(Slot::Ready(running)) => {
                slots.insert(key(owned), Slot::Pending);
                Ok(running)
            }
            Some(Slot::Pending) => {
                slots.insert(key(owned), Slot::Pending);
                Err(ControlError::Busy)
            }
            None => Err(ControlError::NotRunning),
        }
    }
}

fn reconcile_steps(
    slots: &Slots,
    lease_dir: &Option<PathBuf>,
    root: &Path,
    generations: &Path,
    uid: u32,
    state: &TxnState,
) -> Vec<Box<dyn Step>> {
    state
        .steps_done
        .iter()
        .map(|name| -> Box<dyn Step> {
            match name.as_str() {
                "adapter_start" => Box::new(StopOwned {
                    slots: Arc::clone(slots),
                    key: (uid, state.instance.clone()),
                    pid_path: root
                        .join("instances")
                        .join(&state.instance)
                        .join("core.pid"),
                    lease_dir: lease_dir.clone(),
                    holder: state.instance.clone(),
                }),
                "record_generation" => Box::new(RevertGeneration {
                    path: generations.to_owned(),
                    instance: state.instance.clone(),
                }),
                _ => Box::new(Noop {
                    name: "adapter_stop",
                }),
            }
        })
        .collect()
}

struct AdapterStart {
    factory: Arc<dyn AdapterFactory>,
    owned: Owned,
    run_as: RunAs,
    config: CoreConfig,
    held: Arc<Mutex<Option<Box<dyn CoreAdapter + Send>>>>,
    pid_path: PathBuf,
    tunnel: Option<crate::net::TunnelNet>,
}

impl Step for AdapterStart {
    fn name(&self) -> &'static str {
        "adapter_start"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        let mut adapter = self
            .factory
            .make(&self.owned, &self.run_as, self.tunnel.as_ref());
        if let Some(tunnel) = &self.tunnel {
            adapter.set_tunnel_net(tunnel.clone());
        }
        adapter.start(&self.config).map_err(map_core)?;
        if let Some(pid) = adapter.core_pid() {
            let _ = write_pid(&self.pid_path, pid);
        }
        *self.held.lock().unwrap_or_else(|error| error.into_inner()) = Some(adapter);
        Ok(())
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        let adapter = self
            .held
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        stop_adapter(adapter)?;
        kill_pidfile(&self.pid_path, self.owned.uid);
        Ok(())
    }
}

struct AdapterReload {
    held: Arc<Mutex<Option<Box<dyn CoreAdapter + Send>>>>,
    next: CoreConfig,
    previous: CoreConfig,
}

impl Step for AdapterReload {
    fn name(&self) -> &'static str {
        "adapter_reload"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        let mut guard = self.held.lock().unwrap_or_else(|error| error.into_inner());
        let adapter = guard.as_mut().ok_or(ControlError::NotRunning)?;
        adapter.reload(&self.next).map(|_| ()).map_err(map_core)
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        let mut guard = self.held.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(adapter) = guard.as_mut() {
            return adapter.reload(&self.previous).map(|_| ()).map_err(map_core);
        }
        Ok(())
    }
}

struct AdapterStop {
    held: Arc<Mutex<Option<Box<dyn CoreAdapter + Send>>>>,
    pid_path: PathBuf,
    uid: u32,
}

impl Step for AdapterStop {
    fn name(&self) -> &'static str {
        "adapter_stop"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        let adapter = self
            .held
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        stop_adapter(adapter)?;
        kill_pidfile(&self.pid_path, self.uid);
        Ok(())
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        Ok(())
    }
}

struct RecordGeneration {
    path: PathBuf,
    instance: String,
    generation: u64,
    previous: Option<u64>,
}

impl Step for RecordGeneration {
    fn name(&self) -> &'static str {
        "record_generation"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        let mut generations = Generations::load(&self.path)?;
        self.previous = generations.current(&self.instance);
        generations.set(&self.instance, self.generation)
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        let mut generations = Generations::load(&self.path)?;
        match self.previous {
            Some(generation) => generations.set(&self.instance, generation),
            None => generations.clear(&self.instance),
        }
    }
}

struct ClearGeneration {
    path: PathBuf,
    instance: String,
    previous: Option<u64>,
}

impl Step for ClearGeneration {
    fn name(&self) -> &'static str {
        "clear_generation"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        let mut generations = Generations::load(&self.path)?;
        self.previous = generations.current(&self.instance);
        generations.clear(&self.instance)
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        if let Some(generation) = self.previous {
            let mut generations = Generations::load(&self.path)?;
            generations.set(&self.instance, generation)?;
        }
        Ok(())
    }
}

struct StopOwned {
    slots: Slots,
    key: Key,
    pid_path: PathBuf,
    lease_dir: Option<PathBuf>,
    holder: String,
}

impl Step for StopOwned {
    fn name(&self) -> &'static str {
        "adapter_start"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        Ok(())
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        let adapter = {
            let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
            match slots.remove(&self.key) {
                Some(Slot::Ready(running)) => Some(running.adapter),
                other => {
                    if other.is_some() {
                        slots.insert(self.key.clone(), Slot::Pending);
                    }
                    None
                }
            }
        };
        stop_adapter(adapter)?;
        kill_pidfile(&self.pid_path, self.key.0);
        if let Some(dir) = &self.lease_dir {
            release_holder(dir, &self.holder);
        }
        Ok(())
    }
}

struct RevertGeneration {
    path: PathBuf,
    instance: String,
}

impl Step for RevertGeneration {
    fn name(&self) -> &'static str {
        "record_generation"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        Ok(())
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        let mut generations = Generations::load(&self.path)?;
        generations.clear(&self.instance)
    }
}

struct Noop {
    name: &'static str,
}

impl Step for Noop {
    fn name(&self) -> &'static str {
        self.name
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        Ok(())
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        Ok(())
    }
}

fn leased_tunnel(dir: &Path, owned: &Owned) -> Option<crate::net::TunnelNet> {
    let registry = LeaseRegistry::open(dir).ok()?;
    let holder = format!("u{}-{}", owned.uid, owned.instance.as_str());
    let value = registry
        .list()
        .ok()?
        .into_iter()
        .find(|lease| {
            lease.kind == crate::core::leases::ResourceKind::Tunnel && lease.holder == holder
        })?
        .value;
    let index = value.parse().ok()?;
    crate::net::tunnel_net(index, owned.uid).ok()
}

fn meta<'a>(txn: &'a str, owned: &'a Owned, generation: u64, digest: &'a str) -> TxnMeta<'a> {
    TxnMeta {
        txn,
        instance: owned.instance.as_str(),
        generation,
        digest,
    }
}

fn key(owned: &Owned) -> Key {
    (owned.uid, owned.instance.as_str().to_owned())
}

fn uid_count(slots: &HashMap<Key, Slot>, uid: u32) -> usize {
    slots.keys().filter(|(owner, _)| *owner == uid).count()
}

fn pid_path(owned: &Owned) -> PathBuf {
    owned
        .root
        .join("instances")
        .join(owned.instance.as_str())
        .join("core.pid")
}

fn down_status() -> ReplyData {
    ReplyData::Status {
        running: false,
        generation: None,
        api: "down".to_owned(),
        route: "down".to_owned(),
        remote: "unknown".to_owned(),
    }
}

fn api_name(state: ApiState) -> &'static str {
    match state {
        ApiState::Down => "down",
        ApiState::ApiReady => "api_ready",
    }
}

fn route_name(state: RouteState) -> &'static str {
    match state {
        RouteState::Down => "down",
        RouteState::RouteReady => "route_ready",
    }
}

fn remote_name(state: RemoteState) -> &'static str {
    match state {
        RemoteState::Unknown => "unknown",
        RemoteState::Unreachable => "unreachable",
        RemoteState::RemoteReachable => "remote_reachable",
    }
}

fn map_core(error: CoreError) -> ControlError {
    match error {
        CoreError::InvalidConfig => ControlError::InvalidConfig,
        CoreError::Unsupported => ControlError::Unsupported,
        CoreError::NotRunning => ControlError::NotRunning,
        CoreError::Busy => ControlError::Busy,
        CoreError::Timeout => ControlError::Timeout,
        CoreError::Conflict => ControlError::Conflict,
        _ => ControlError::Failed,
    }
}

fn parse_reply(text: &str) -> Result<Reply, ControlError> {
    serde_json::from_str(text).map_err(|_| ControlError::Failed)
}

fn stop_adapter(adapter: Option<Box<dyn CoreAdapter + Send>>) -> Result<(), ControlError> {
    let Some(mut adapter) = adapter else {
        return Ok(());
    };
    match adapter.stop() {
        Ok(()) | Err(CoreError::NotRunning) => Ok(()),
        Err(error) => Err(map_core(error)),
    }
}

pub(crate) fn crash_point(name: &str) -> bool {
    if !crate::common::test_mode() {
        return false;
    }
    if std::env::var("CM_CONTROLLER_CRASH_BEFORE").ok().as_deref() == Some(name) {
        std::process::abort();
    }
    false
}

fn write_pid(path: &Path, pid: u32) -> Result<(), ()> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).map_err(|_| ())?;
    let start = parse_start_time(&stat).ok_or(())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| ())?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| ())?;
    writeln!(file, "{pid} {start}").map_err(|_| ())?;
    Ok(())
}

/// Сигнал уходит только группе, чей лидер стартовал в записанный момент и принадлежит
/// владельцу экземпляра. Файл лежит в каталоге владельца, поэтому его содержимому
/// контроллер не доверяет: чужой процесс по подложенному pid остановить нельзя.
fn kill_pidfile(path: &Path, uid: u32) {
    if path.as_os_str().is_empty() {
        return;
    }
    let Some(text) = read_pidfile(path) else {
        return;
    };
    let mut parts = text.split_whitespace();
    let Some(pid) = parts.next().and_then(|text| text.parse::<i32>().ok()) else {
        return;
    };
    let Some(start) = parts.next().and_then(|text| text.parse::<u64>().ok()) else {
        return;
    };
    if pid <= 1 {
        let _ = fs::remove_file(path);
        return;
    }
    let current = fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| parse_start_time(&stat));
    let owner = fs::metadata(format!("/proc/{pid}"))
        .ok()
        .map(|meta| std::os::unix::fs::MetadataExt::uid(&meta));
    if current != Some(start) || owner != Some(uid) {
        let _ = fs::remove_file(path);
        return;
    }
    signal_group(pid, libc::SIGTERM);
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && signal_group(pid, 0) {
        std::thread::sleep(Duration::from_millis(20));
    }
    if signal_group(pid, 0) {
        signal_group(pid, libc::SIGKILL);
    }
    let _ = fs::remove_file(path);
}

fn read_pidfile(path: &Path) -> Option<String> {
    use std::io::Read;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut text = String::new();
    file.take(64).read_to_string(&mut text).ok()?;
    Some(text)
}

fn signal_group(pid: i32, signal: i32) -> bool {
    // SAFETY: a negative pid signals the process group created by setsid. Signal 0 only probes.
    unsafe { libc::kill(-pid, signal) == 0 }
}

fn release_holder(dir: &Path, holder: &str) {
    let Ok(registry) = LeaseRegistry::open(dir) else {
        return;
    };
    let Ok(leases) = registry.list() else {
        return;
    };
    for lease in leases.into_iter().filter(|lease| lease.holder == holder) {
        let _ = registry.release(&lease.holder, lease.kind, &lease.value);
    }
}
