//! Сеть туннеля как транзакция контроллера. Номер берётся из аренды, не из кадра.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::core::leases::{LeaseError, LeaseRegistry, ResourceKind};
use crate::net::{self, NetExec, TunnelNet};
use crate::profiles::Ipv6Policy;

use super::journal::{Journal, TxnState};
use super::owner::Owned;
use super::protocol::{ControlError, Reply, ReplyData};
use super::registry::{self, Generations, Replay};
use super::txn::{self, Step, TxnMeta};

pub struct NetCtx {
    pub exec: Arc<Mutex<dyn NetExec + Send>>,
    pub lease_dir: PathBuf,
    pub etc_root: PathBuf,
}

pub fn apply(
    owned: &Owned,
    generation: u64,
    txn: &str,
    digest: &str,
    now: i64,
    ctx: &Arc<NetCtx>,
) -> Result<Reply, ControlError> {
    let mut journal = Journal::open(&owned.journal_path())?;
    let generations = Generations::load(&owned.generations_path())?;
    if let Replay::Stored(text) = registry::replay_or_fresh(&journal, txn, digest)? {
        return parse_reply(&text);
    }
    let key = net_key(owned);
    registry::check_start(generations.current(&key), generation)?;
    if find_index(ctx, owned)?.is_some() {
        return Err(ControlError::Conflict);
    }
    let shared = Arc::new(Mutex::new(None::<u8>));
    let mut steps: Vec<Box<dyn Step + '_>> = vec![
        Box::new(LeaseTunnel {
            ctx: Arc::clone(ctx),
            owned: owned.clone(),
            shared: Arc::clone(&shared),
            index: None,
        }),
        Box::new(ResolvFile {
            ctx: Arc::clone(ctx),
            shared: Arc::clone(&shared),
            created: false,
        }),
        Box::new(NetCreate {
            ctx: Arc::clone(ctx),
            owned: owned.clone(),
            shared: Arc::clone(&shared),
        }),
        Box::new(RecordNet {
            path: owned.generations_path(),
            instance: key,
            generation,
            previous: None,
        }),
    ];
    let txn_id = txn.to_owned();
    let shared_reply = Arc::clone(&shared);
    txn::run_late(
        &mut journal,
        &meta(txn, owned, generation, digest),
        &mut steps,
        &|| {
            let index = shared_reply
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .unwrap_or(0);
            let reply = Reply::ok(
                &txn_id,
                Some(ReplyData::Net {
                    index,
                    netns: format!("cm-{index}"),
                }),
            );
            serde_json::to_string(&reply).unwrap_or_default()
        },
        &super::core_ops::crash_point,
        now,
    )?;
    let index = shared
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .ok_or(ControlError::Failed)?;
    Ok(Reply::ok(
        txn,
        Some(ReplyData::Net {
            index,
            netns: format!("cm-{index}"),
        }),
    ))
}

pub fn revert(
    owned: &Owned,
    generation: u64,
    txn: &str,
    digest: &str,
    now: i64,
    ctx: &Arc<NetCtx>,
) -> Result<Reply, ControlError> {
    let mut journal = Journal::open(&owned.journal_path())?;
    let generations = Generations::load(&owned.generations_path())?;
    if let Replay::Stored(text) = registry::replay_or_fresh(&journal, txn, digest)? {
        return parse_reply(&text);
    }
    let key = net_key(owned);
    registry::check_running(generations.current(&key), generation)?;
    let Some(index) = find_index(ctx, owned)? else {
        return Err(ControlError::NotRunning);
    };
    let mut steps: Vec<Box<dyn Step + '_>> = vec![
        Box::new(NetDestroy {
            ctx: Arc::clone(ctx),
            owned: owned.clone(),
            index,
        }),
        Box::new(ResolvRemove {
            ctx: Arc::clone(ctx),
            index,
        }),
        Box::new(ReleaseTunnel {
            ctx: Arc::clone(ctx),
            owned: owned.clone(),
            index,
        }),
        Box::new(ClearNet {
            path: owned.generations_path(),
            instance: key,
            previous: None,
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

pub fn compensate_steps(
    state: &TxnState,
    ctx: &Arc<NetCtx>,
    root_uid: u32,
) -> Option<Vec<Box<dyn Step>>> {
    let net = state.steps_done.iter().any(|name| {
        matches!(
            name.as_str(),
            "lease_tunnel" | "resolv_conf" | "net_create" | "net_destroy" | "release_tunnel"
        )
    });
    if !net {
        return None;
    }
    let owned_instance = state.instance.clone();
    Some(
        state
            .steps_done
            .iter()
            .map(|name| -> Box<dyn Step> {
                match name.as_str() {
                    "lease_tunnel" | "release_tunnel" | "record_generation"
                    | "clear_generation" => Box::new(ReleaseByName {
                        ctx: Arc::clone(ctx),
                        uid: root_uid,
                        instance: owned_instance.clone(),
                    }),
                    "resolv_conf" => Box::new(ResolvRemoveKnown {
                        ctx: Arc::clone(ctx),
                        uid: root_uid,
                        instance: owned_instance.clone(),
                    }),
                    "net_create" | "net_destroy" => Box::new(DestroyByName {
                        ctx: Arc::clone(ctx),
                        uid: root_uid,
                        instance: owned_instance.clone(),
                    }),
                    _ => Box::new(Noop),
                }
            })
            .collect(),
    )
}

struct LeaseTunnel {
    ctx: Arc<NetCtx>,
    owned: Owned,
    shared: Arc<Mutex<Option<u8>>>,
    index: Option<u8>,
}

impl Step for LeaseTunnel {
    fn name(&self) -> &'static str {
        "lease_tunnel"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        fs::create_dir_all(&self.ctx.lease_dir).map_err(|_| ControlError::Failed)?;
        let registry = open_registry(&self.ctx.lease_dir)?;
        let holder = holder(&self.owned);
        let lease = match registry.allocate(&holder, ResourceKind::Tunnel) {
            Ok(lease) => lease,
            Err(LeaseError::Exhausted) => return Err(ControlError::Quota),
            Err(LeaseError::Busy) => return Err(ControlError::Busy),
            Err(_) => return Err(ControlError::Failed),
        };
        let index = lease.value.parse().map_err(|_| ControlError::Failed)?;
        self.index = Some(index);
        *self
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(index);
        Ok(())
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        if let Some(index) = self.index {
            release(&self.ctx, &self.owned, index)?;
        }
        Ok(())
    }
}

struct ResolvFile {
    ctx: Arc<NetCtx>,
    shared: Arc<Mutex<Option<u8>>>,
    created: bool,
}

impl Step for ResolvFile {
    fn name(&self) -> &'static str {
        "resolv_conf"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        let index = self
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .ok_or(ControlError::Failed)?;
        write_resolv(&self.ctx.etc_root, index)?;
        self.created = true;
        Ok(())
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        if self.created
            && let Some(index) = *self
                .shared
                .lock()
                .unwrap_or_else(|error| error.into_inner())
        {
            remove_resolv(&self.ctx.etc_root, index);
        }
        Ok(())
    }
}

struct NetCreate {
    ctx: Arc<NetCtx>,
    owned: Owned,
    shared: Arc<Mutex<Option<u8>>>,
}

impl Step for NetCreate {
    fn name(&self) -> &'static str {
        "net_create"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        let index = self
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .ok_or(ControlError::Failed)?;
        let tunnel = net::tunnel_net(index, self.owned.uid).map_err(|_| ControlError::Failed)?;
        let all = active_tunnels(&self.ctx)?;
        let mut exec = self
            .ctx
            .exec
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        net::create(&tunnel, Ipv6Policy::Block, &all, &mut *exec).map_err(|_| ControlError::Failed)
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        let Some(index) = *self
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner())
        else {
            return Ok(());
        };
        destroy_index(&self.ctx, index, self.owned.uid)
    }
}

struct RecordNet {
    path: PathBuf,
    instance: String,
    generation: u64,
    previous: Option<u64>,
}

impl Step for RecordNet {
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

struct NetDestroy {
    ctx: Arc<NetCtx>,
    owned: Owned,
    index: u8,
}

impl Step for NetDestroy {
    fn name(&self) -> &'static str {
        "net_destroy"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        destroy_index(&self.ctx, self.index, self.owned.uid)
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        Ok(())
    }
}

struct ResolvRemove {
    ctx: Arc<NetCtx>,
    index: u8,
}

impl Step for ResolvRemove {
    fn name(&self) -> &'static str {
        "resolv_conf"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        remove_resolv(&self.ctx.etc_root, self.index);
        Ok(())
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        Ok(())
    }
}

struct ReleaseTunnel {
    ctx: Arc<NetCtx>,
    owned: Owned,
    index: u8,
}

impl Step for ReleaseTunnel {
    fn name(&self) -> &'static str {
        "release_tunnel"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        release(&self.ctx, &self.owned, self.index)
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        Ok(())
    }
}

struct ClearNet {
    path: PathBuf,
    instance: String,
    previous: Option<u64>,
}

impl Step for ClearNet {
    fn name(&self) -> &'static str {
        "clear_generation"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        let mut generations = Generations::load(&self.path)?;
        self.previous = generations.current(&self.instance);
        generations.clear(&self.instance)
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        if let Some(generation) = self.previous {
            let mut generations = Generations::load(&self.path)?;
            generations.set(&self.instance, generation)?;
        }
        Ok(())
    }
}

struct ReleaseByName {
    ctx: Arc<NetCtx>,
    uid: u32,
    instance: String,
}

impl Step for ReleaseByName {
    fn name(&self) -> &'static str {
        "lease_tunnel"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        Ok(())
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        let Ok(registry) = LeaseRegistry::open(&self.ctx.lease_dir) else {
            return Ok(());
        };
        let holder = format!("u{}-{}", self.uid, self.instance);
        let Ok(leases) = registry.list() else {
            return Ok(());
        };
        for lease in leases
            .into_iter()
            .filter(|lease| lease.kind == ResourceKind::Tunnel && lease.holder == holder)
        {
            let _ = registry.release(&lease.holder, lease.kind, &lease.value);
            if let Ok(index) = lease.value.parse::<u8>() {
                remove_resolv(&self.ctx.etc_root, index);
                let _ = destroy_index(&self.ctx, index, self.uid);
            }
        }
        Ok(())
    }
}

struct ResolvRemoveKnown {
    ctx: Arc<NetCtx>,
    uid: u32,
    instance: String,
}

impl Step for ResolvRemoveKnown {
    fn name(&self) -> &'static str {
        "resolv_conf"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        Ok(())
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        if let Some(index) = lookup(&self.ctx, self.uid, &self.instance) {
            remove_resolv(&self.ctx.etc_root, index);
        }
        Ok(())
    }
}

struct DestroyByName {
    ctx: Arc<NetCtx>,
    uid: u32,
    instance: String,
}

impl Step for DestroyByName {
    fn name(&self) -> &'static str {
        "net_create"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        Ok(())
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        if let Some(index) = lookup(&self.ctx, self.uid, &self.instance) {
            let _ = destroy_index(&self.ctx, index, self.uid);
        }
        Ok(())
    }
}

struct Noop;

impl Step for Noop {
    fn name(&self) -> &'static str {
        "net_create"
    }
    fn apply(&mut self) -> Result<(), ControlError> {
        Ok(())
    }
    fn compensate(&mut self) -> Result<(), ControlError> {
        Ok(())
    }
}

fn write_resolv(root: &Path, index: u8) -> Result<(), ControlError> {
    let tunnel = net::tunnel_net(index, 0).map_err(|_| ControlError::Failed)?;
    let dir = root.join(&tunnel.netns);
    fs::create_dir_all(&dir).map_err(|_| ControlError::Failed)?;
    let mut perms = fs::metadata(&dir)
        .map_err(|_| ControlError::Failed)?
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&dir, perms).map_err(|_| ControlError::Failed)?;
    let path = dir.join("resolv.conf");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .map_err(|_| ControlError::Failed)?;
    file.write_all(net::dns::resolv_conf(&tunnel).as_bytes())
        .map_err(|_| ControlError::Failed)?;
    Ok(())
}

fn remove_resolv(root: &Path, index: u8) {
    let dir = root.join(format!("cm-{index}"));
    let _ = fs::remove_file(dir.join("resolv.conf"));
    let _ = fs::remove_dir(&dir);
}

fn destroy_index(ctx: &NetCtx, index: u8, uid: u32) -> Result<(), ControlError> {
    let tunnel = net::tunnel_net(index, uid).map_err(|_| ControlError::Failed)?;
    let all = active_tunnels(ctx).unwrap_or_default();
    let remaining: Vec<TunnelNet> = all.into_iter().filter(|item| item.index != index).collect();
    let mut exec = ctx.exec.lock().unwrap_or_else(|error| error.into_inner());
    net::destroy(&tunnel, &remaining, &mut *exec).map_err(|_| ControlError::Failed)
}

fn active_tunnels(ctx: &NetCtx) -> Result<Vec<TunnelNet>, ControlError> {
    let Ok(registry) = LeaseRegistry::open(&ctx.lease_dir) else {
        return Ok(Vec::new());
    };
    let leases = registry.list().map_err(|_| ControlError::Failed)?;
    let mut tunnels = Vec::new();
    for lease in leases
        .into_iter()
        .filter(|lease| lease.kind == ResourceKind::Tunnel)
    {
        let Ok(index) = lease.value.parse::<u8>() else {
            continue;
        };
        if let Ok(tunnel) = net::tunnel_net(index, 0) {
            tunnels.push(tunnel);
        }
    }
    Ok(tunnels)
}

pub fn current_index(ctx: &NetCtx, owned: &Owned) -> Result<Option<u8>, ControlError> {
    find_index(ctx, owned)
}

fn find_index(ctx: &NetCtx, owned: &Owned) -> Result<Option<u8>, ControlError> {
    Ok(lookup(ctx, owned.uid, owned.instance.as_str()))
}

fn lookup(ctx: &NetCtx, uid: u32, instance: &str) -> Option<u8> {
    let registry = LeaseRegistry::open(&ctx.lease_dir).ok()?;
    let holder = format!("u{uid}-{instance}");
    registry
        .list()
        .ok()?
        .into_iter()
        .find(|lease| lease.kind == ResourceKind::Tunnel && lease.holder == holder)?
        .value
        .parse()
        .ok()
}

fn release(ctx: &NetCtx, owned: &Owned, index: u8) -> Result<(), ControlError> {
    let registry = open_registry(&ctx.lease_dir)?;
    match registry.release(&holder(owned), ResourceKind::Tunnel, &index.to_string()) {
        Ok(()) | Err(LeaseError::Conflict) => Ok(()),
        Err(_) => Err(ControlError::Failed),
    }
}

fn open_registry(dir: &Path) -> Result<LeaseRegistry, ControlError> {
    fs::create_dir_all(dir).map_err(|_| ControlError::Failed)?;
    match LeaseRegistry::open(dir) {
        Ok(registry) => Ok(registry),
        Err(_) => LeaseRegistry::create(dir).map_err(|_| ControlError::Failed),
    }
}

fn holder(owned: &Owned) -> String {
    format!("u{}-{}", owned.uid, owned.instance.as_str())
}

fn net_key(owned: &Owned) -> String {
    format!("net:{}", owned.instance.as_str())
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
