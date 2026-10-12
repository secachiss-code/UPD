//! Туннель хоста: один файл состояния на машину и транзакция, как у сети приложения.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::net::host::{self, HostBlock};
use crate::net::host_commands::{host_create_commands, host_destroy_commands};
use crate::net::host_firewall::host_firewall_commands;
use crate::net::host_plan::{self, HostNet, Uplink};
use crate::net::query;

use super::core_ops::Workers;
use super::journal::Journal;
use super::net_ops::NetCtx;
use super::owner::Owned;
use super::protocol::{ControlError, Reply, ReplyData};
use super::registry::{self, Generations, Replay};
use super::txn::{self, Step, TxnMeta};

pub const HOST_NET_KEY: &str = "net:host";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostState {
    pub uid: u32,
    pub mode: String,
    pub generation: u64,
    pub uplinks: Vec<Uplink>,
}

pub fn state_path(base: &Path) -> PathBuf {
    base.join("host.json")
}

pub fn load_state(base: &Path) -> Result<Option<HostState>, ControlError> {
    let path = state_path(base);
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ControlError::Failed),
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(ControlError::Failed);
    }
    let bytes = fs::read(&path).map_err(|_| ControlError::Failed)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| ControlError::Failed)
}

pub fn apply(
    owned: &Owned,
    generation: u64,
    mode: &str,
    txn: &str,
    digest: &str,
    now: i64,
    ctx: &Arc<NetCtx>,
) -> Result<Reply, ControlError> {
    if mode != "proxy" && mode != "tunnel" {
        return Err(ControlError::BadArgument);
    }
    let mut journal = Journal::open(&owned.journal_path())?;
    if let Replay::Stored(text) = registry::replay_or_fresh(&journal, txn, digest)? {
        return parse_reply(&text);
    }
    let base = base_of(owned);
    if load_state(&base)?.is_some() {
        return Err(ControlError::Conflict);
    }
    let uplinks = if mode == "tunnel" {
        let observed = {
            let mut query = ctx.query.lock().unwrap_or_else(|error| error.into_inner());
            query::observe(&mut *query, &[]).map_err(|_| ControlError::Failed)?
        };
        let integration = host::detect(Path::new("/run"));
        match host::preflight(
            crate::profiles::HostMode::Tunnel,
            &observed.links,
            &observed.rules,
            &integration,
        ) {
            Ok(()) => {}
            Err(HostBlock::Resolved) => return Err(ControlError::Unsupported),
            Err(HostBlock::LegacyCm | HostBlock::ForeignAutoRoute) => {
                return Err(ControlError::Conflict);
            }
        }
        let mut query = ctx.query.lock().unwrap_or_else(|error| error.into_inner());
        query::observe_uplinks(&mut *query).map_err(|_| ControlError::Failed)?
    } else {
        Vec::new()
    };
    let state = HostState {
        uid: owned.uid,
        mode: mode.to_owned(),
        generation,
        uplinks: uplinks.clone(),
    };
    let names: Vec<String> = uplinks.iter().map(|item| item.dev.clone()).collect();
    let mut steps: Vec<Box<dyn Step + '_>> = vec![Box::new(WriteHostState {
        path: state_path(&base),
        state: state.clone(),
        written: false,
    })];
    if mode == "tunnel" {
        let host = host_plan::host_net(owned.uid);
        steps.push(Box::new(HostCreate {
            ctx: Arc::clone(ctx),
            host: host.clone(),
            uplinks: uplinks.clone(),
        }));
        steps.push(Box::new(HostFirewall {
            ctx: Arc::clone(ctx),
            host: Some(host),
        }));
    }
    steps.push(Box::new(RecordHost {
        path: owned.generations_path(),
        generation,
        previous: None,
    }));
    let reply = Reply::ok(
        txn,
        Some(ReplyData::Host {
            mode: mode.to_owned(),
            generation: Some(generation),
            uplinks: names,
        }),
    );
    let text = serde_json::to_string(&reply).map_err(|_| ControlError::Failed)?;
    txn::run(
        &mut journal,
        &meta(txn, owned, generation, digest),
        &mut steps,
        &text,
        &super::core_ops::crash_point,
        now,
    )?;
    Ok(reply)
}

pub fn revert(
    owned: &Owned,
    generation: u64,
    txn: &str,
    digest: &str,
    now: i64,
    ctx: &Arc<NetCtx>,
    workers: &Workers,
) -> Result<Reply, ControlError> {
    let mut journal = Journal::open(&owned.journal_path())?;
    if let Replay::Stored(text) = registry::replay_or_fresh(&journal, txn, digest)? {
        return parse_reply(&text);
    }
    let base = base_of(owned);
    let Some(state) = load_state(&base)? else {
        return Err(ControlError::NotRunning);
    };
    if state.uid != owned.uid {
        return Err(ControlError::NotRunning);
    }
    if state.generation != generation {
        return Err(ControlError::GenerationMismatch);
    }
    let status = workers.status(owned, "status");
    if let Some(ReplyData::Status { running: true, .. }) = status.data {
        return Err(ControlError::Conflict);
    }
    let host = host_plan::host_net(owned.uid);
    let mut steps: Vec<Box<dyn Step + '_>> = vec![
        Box::new(HostFirewall {
            ctx: Arc::clone(ctx),
            host: None,
        }),
        Box::new(HostDestroy {
            ctx: Arc::clone(ctx),
            host,
            uplinks: state.uplinks.clone(),
        }),
        Box::new(DeleteHostState {
            path: state_path(&base),
        }),
        Box::new(ClearHost {
            path: owned.generations_path(),
        }),
    ];
    let reply = Reply::ok(txn, None);
    let text = serde_json::to_string(&reply).map_err(|_| ControlError::Failed)?;
    txn::run(
        &mut journal,
        &meta(txn, owned, generation, digest),
        &mut steps,
        &text,
        &super::core_ops::crash_point,
        now,
    )?;
    Ok(reply)
}

pub fn status(owned: &Owned, txn: &str) -> Reply {
    let base = base_of(owned);
    let Ok(Some(state)) = load_state(&base) else {
        return off(txn);
    };
    if state.uid != owned.uid {
        return off(txn);
    }
    Reply::ok(
        txn,
        Some(ReplyData::Host {
            mode: state.mode,
            generation: Some(state.generation),
            uplinks: state.uplinks.into_iter().map(|item| item.dev).collect(),
        }),
    )
}

/// Сеть ядра `host`. Нет своего файла или чужой uid — `conflict`: имя зарезервировано.
pub fn required_tunnel(owned: &Owned) -> Result<Option<crate::net::TunnelNet>, ControlError> {
    let Some(state) = load_state(&base_of(owned))? else {
        return Err(ControlError::Conflict);
    };
    if state.uid != owned.uid {
        return Err(ControlError::Conflict);
    }
    match state.mode.as_str() {
        "tunnel" => Ok(Some(host_plan::host_net(owned.uid).as_tunnel())),
        "proxy" => Ok(None),
        _ => Err(ControlError::Conflict),
    }
}

pub fn base_of(owned: &Owned) -> PathBuf {
    owned
        .root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| owned.root.clone())
}

fn off(txn: &str) -> Reply {
    Reply::ok(
        txn,
        Some(ReplyData::Host {
            mode: "off".to_owned(),
            generation: None,
            uplinks: Vec::new(),
        }),
    )
}

pub(crate) fn save_state(path: &Path, state: &HostState) -> Result<(), ControlError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|_| ControlError::Failed)?;
    }
    let tmp = path.with_file_name("host.json.tmp");
    let bytes = serde_json::to_vec(state).map_err(|_| ControlError::Failed)?;
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&tmp)
            .map_err(|_| ControlError::Failed)?;
        file.write_all(&bytes).map_err(|_| ControlError::Failed)?;
        file.sync_all().map_err(|_| ControlError::Failed)?;
    }
    fs::rename(&tmp, path).map_err(|_| ControlError::Failed)
}

fn run_strict(ctx: &NetCtx, commands: &[crate::net::Cmd]) -> Result<(), ControlError> {
    let mut exec = ctx.exec.lock().unwrap_or_else(|error| error.into_inner());
    for command in commands {
        exec.run(command).map_err(|_| ControlError::Failed)?;
    }
    Ok(())
}

fn run_loose(ctx: &NetCtx, commands: &[crate::net::Cmd]) {
    let mut exec = ctx.exec.lock().unwrap_or_else(|error| error.into_inner());
    for command in commands {
        let _ = exec.run(command);
    }
}

struct WriteHostState {
    path: PathBuf,
    state: HostState,
    written: bool,
}

impl Step for WriteHostState {
    fn name(&self) -> &'static str {
        "write_host_state"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        save_state(&self.path, &self.state)?;
        self.written = true;
        Ok(())
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        if self.written {
            let _ = fs::remove_file(&self.path);
        }
        Ok(())
    }
}

struct HostCreate {
    ctx: Arc<NetCtx>,
    host: HostNet,
    uplinks: Vec<Uplink>,
}

impl Step for HostCreate {
    fn name(&self) -> &'static str {
        "host_create"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        run_strict(&self.ctx, &host_create_commands(&self.host, &self.uplinks))
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        run_loose(&self.ctx, &host_destroy_commands(&self.host, &self.uplinks));
        Ok(())
    }
}

struct HostDestroy {
    ctx: Arc<NetCtx>,
    host: HostNet,
    uplinks: Vec<Uplink>,
}

impl Step for HostDestroy {
    fn name(&self) -> &'static str {
        "host_destroy"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        run_loose(&self.ctx, &host_destroy_commands(&self.host, &self.uplinks));
        Ok(())
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        Ok(())
    }
}

struct HostFirewall {
    ctx: Arc<NetCtx>,
    host: Option<HostNet>,
}

impl Step for HostFirewall {
    fn name(&self) -> &'static str {
        "host_firewall"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        run_strict(&self.ctx, &host_firewall_commands(self.host.as_ref()))
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        run_loose(&self.ctx, &host_firewall_commands(None));
        Ok(())
    }
}

struct RecordHost {
    path: PathBuf,
    generation: u64,
    previous: Option<u64>,
}

impl Step for RecordHost {
    fn name(&self) -> &'static str {
        "record_generation"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        let mut generations = Generations::load(&self.path)?;
        self.previous = generations.current(HOST_NET_KEY);
        generations.set(HOST_NET_KEY, self.generation)
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        let mut generations = Generations::load(&self.path)?;
        match self.previous {
            Some(generation) => generations.set(HOST_NET_KEY, generation),
            None => generations.clear(HOST_NET_KEY),
        }
    }
}

struct DeleteHostState {
    path: PathBuf,
}

impl Step for DeleteHostState {
    fn name(&self) -> &'static str {
        "write_host_state"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(ControlError::Failed),
        }
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        Ok(())
    }
}

struct ClearHost {
    path: PathBuf,
}

impl Step for ClearHost {
    fn name(&self) -> &'static str {
        "record_generation"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        let mut generations = Generations::load(&self.path)?;
        generations.clear(HOST_NET_KEY)
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        Ok(())
    }
}

fn meta<'a>(txn: &'a str, owned: &'a Owned, generation: u64, digest: &'a str) -> TxnMeta<'a> {
    TxnMeta {
        txn,
        instance: owned.instance.as_str(),
        generation,
        digest,
    }
}

fn parse_reply(text: &str) -> Result<Reply, ControlError> {
    if text.is_empty() {
        return Err(ControlError::Failed);
    }
    serde_json::from_str(text).map_err(|_| ControlError::Failed)
}
