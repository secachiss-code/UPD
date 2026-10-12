//! Сведение после гибели контроллера и после загрузки машины.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

use crate::core::leases::{LeaseRegistry, ResourceKind};
use crate::net::host_commands::host_destroy_commands;
use crate::net::host_firewall::host_firewall_commands;
use crate::net::host_plan;
use crate::net::observe::Drift;
use crate::net::{self, Cmd, TunnelNet};

use super::core_ops::{Workers, kill_pidfile};
use super::host_ops::{self, HOST_NET_KEY};
use super::net_ops::NetCtx;
use super::owner;
use super::protocol::{ControlError, ReplyData};
use super::registry::Generations;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BootReport {
    pub owners: u32,
    pub compensated: u32,
    pub cleared_nets: u32,
    pub repaired: u32,
    pub killed_cores: u32,
    pub orphans: u32,
}

pub fn reconcile_all(
    base: &Path,
    workers: &Workers,
    nets: &Arc<NetCtx>,
    now: i64,
) -> Result<BootReport, ControlError> {
    let mut report = BootReport::default();
    let owners = owner_ids(base);
    report.owners = owners.len() as u32;
    for uid in &owners {
        if let Ok(owned) = owner::owned(base, *uid, "journal")
            && let Ok(reply) = workers.reconcile(&owned, "boot", now)
            && let Some(ReplyData::Reconciled { compensated }) = reply.data
        {
            report.compensated = report.compensated.saturating_add(compensated);
        }
        report.killed_cores = report.killed_cores.saturating_add(stop_cores(base, *uid));
    }
    let mut kept = leased(nets);
    let tables: Vec<u32> = kept.iter().map(|(_, _, tunnel)| tunnel.table).collect();
    let observed = {
        let mut query = nets.query.lock().unwrap_or_else(|error| error.into_inner());
        net::query::observe(&mut *query, &tables).ok()
    };
    if let Some(observed) = observed.as_ref() {
        let mut still = Vec::new();
        for (uid, instance, tunnel) in kept {
            let netns = observed.netns.iter().any(|name| name == &tunnel.netns);
            let veth = observed.links.iter().any(|name| name == &tunnel.veth_host);
            if !netns && !veth {
                clear_net(nets, base, uid, &instance, tunnel.index);
                report.cleared_nets = report.cleared_nets.saturating_add(1);
            } else {
                still.push((uid, instance, tunnel));
            }
        }
        kept = still;
        let desired: Vec<TunnelNet> = kept.iter().map(|(_, _, tunnel)| tunnel.clone()).collect();
        let drift = net::observe::audit(&desired, observed);
        let mut repaired_index = std::collections::BTreeSet::new();
        for item in &drift {
            match item {
                Drift::OrphanVeth(_) | Drift::OrphanRule(_) => {
                    run_loose(nets, &net::observe::repair_commands(item, &desired));
                    report.orphans = report.orphans.saturating_add(1);
                }
                Drift::MissingNetns(_) => {}
                other => {
                    if let Some(index) = drift_index(other) {
                        let netns_up = kept.iter().any(|(_, _, tunnel)| {
                            tunnel.index == index
                                && observed.netns.iter().any(|name| name == &tunnel.netns)
                        });
                        if netns_up {
                            run_loose(nets, &net::observe::repair_commands(other, &desired));
                            repaired_index.insert(index);
                        }
                    }
                }
            }
        }
        report.repaired = repaired_index.len() as u32;
    }
    reconcile_host(base, nets, observed.as_ref());
    let desired: Vec<TunnelNet> = kept.iter().map(|(_, _, tunnel)| tunnel.clone()).collect();
    let commands = net::firewall::replace_commands(&desired);
    run_strict(nets, &commands)?;
    Ok(report)
}

fn owner_ids(base: &Path) -> Vec<u32> {
    let mut ids = Vec::new();
    let Ok(entries) = fs::read_dir(base) else {
        return ids;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(rest) = name.strip_prefix('u') else {
            continue;
        };
        let Ok(uid) = rest.parse::<u32>() else {
            continue;
        };
        let Ok(meta) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            continue;
        }
        ids.push(uid);
    }
    ids.sort_unstable();
    ids
}

fn stop_cores(base: &Path, uid: u32) -> u32 {
    let instances = base.join(format!("u{uid}")).join("instances");
    let Ok(entries) = fs::read_dir(&instances) else {
        return 0;
    };
    let mut killed = 0u32;
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let pid = entry.path().join("core.pid");
        if !pid.is_file() {
            continue;
        }
        if kill_pidfile(&pid, uid) {
            killed = killed.saturating_add(1);
        }
        if let Ok(owned) = owner::owned(base, uid, &name)
            && let Ok(mut generations) = Generations::load(&owned.generations_path())
        {
            let _ = generations.clear(&name);
        }
    }
    killed
}

fn clear_net(nets: &NetCtx, base: &Path, uid: u32, instance: &str, index: u8) {
    if let Ok(registry) = LeaseRegistry::open(&nets.lease_dir) {
        let holder = format!("u{uid}-{instance}");
        let _ = registry.release(&holder, ResourceKind::Tunnel, &index.to_string());
    }
    let dir = nets.etc_root.join(format!("cm-{index}"));
    let _ = fs::remove_file(dir.join("resolv.conf"));
    let _ = fs::remove_dir(&dir);
    if let Ok(owned) = owner::owned(base, uid, instance)
        && let Ok(mut generations) = Generations::load(&owned.generations_path())
    {
        let _ = generations.clear(&format!("net:{instance}"));
    }
}

fn reconcile_host(base: &Path, nets: &NetCtx, observed: Option<&net::Observed>) {
    let Ok(Some(mut state)) = host_ops::load_state(base) else {
        return;
    };
    let Some(observed) = observed else {
        return;
    };
    let host = host_plan::host_net(state.uid);
    if !observed.links.iter().any(|name| name == &host.tun) {
        run_loose(nets, &host_firewall_commands(None));
        run_loose(nets, &host_destroy_commands(&host, &state.uplinks));
        let _ = fs::remove_file(host_ops::state_path(base));
        if let Ok(owned) = owner::owned(base, state.uid, host_plan::HOST_INSTANCE)
            && let Ok(mut generations) = Generations::load(&owned.generations_path())
        {
            let _ = generations.clear(HOST_NET_KEY);
        }
        return;
    }
    let new = {
        let mut query = nets.query.lock().unwrap_or_else(|error| error.into_inner());
        match net::query::observe_uplinks(&mut *query) {
            Ok(uplinks) => uplinks,
            Err(_) => return,
        }
    };
    if new != state.uplinks {
        run_loose(
            nets,
            &net::host_commands::uplink_commands(&host, &state.uplinks, &new),
        );
        state.uplinks = new;
        let _ = host_ops::save_state(&host_ops::state_path(base), &state);
    }
    let _ = super::watch::repair_host_net(nets, &host, &state.uplinks);
}

fn leased(nets: &NetCtx) -> Vec<(u32, String, TunnelNet)> {
    let Ok(registry) = LeaseRegistry::open(&nets.lease_dir) else {
        return Vec::new();
    };
    let Ok(leases) = registry.list() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for lease in leases
        .into_iter()
        .filter(|lease| lease.kind == ResourceKind::Tunnel)
    {
        let Some(rest) = lease.holder.strip_prefix('u') else {
            continue;
        };
        let Some((uid, instance)) = rest.split_once('-') else {
            continue;
        };
        let Ok(uid) = uid.parse::<u32>() else {
            continue;
        };
        let Ok(index) = lease.value.parse::<u8>() else {
            continue;
        };
        let Ok(tunnel) = net::tunnel_net(index, uid) else {
            continue;
        };
        found.push((uid, instance.to_owned(), tunnel));
    }
    found
}

fn drift_index(drift: &Drift) -> Option<u8> {
    match drift {
        Drift::MissingBlackhole(index)
        | Drift::MissingRule(index)
        | Drift::MissingTunRoute(index)
        | Drift::MissingVeth(index)
        | Drift::MissingTun(index)
        | Drift::MissingNetns(index) => Some(*index),
        Drift::OrphanVeth(_) | Drift::OrphanRule(_) => None,
    }
}

fn run_loose(nets: &NetCtx, commands: &[Cmd]) {
    let mut exec = nets.exec.lock().unwrap_or_else(|error| error.into_inner());
    for command in commands {
        let _ = exec.run(command);
    }
}

fn run_strict(nets: &NetCtx, commands: &[Cmd]) -> Result<(), ControlError> {
    let mut exec = nets.exec.lock().unwrap_or_else(|error| error.into_inner());
    for command in commands {
        exec.run(command).map_err(|_| ControlError::Failed)?;
    }
    Ok(())
}

pub fn print_report(report: &BootReport) {
    if let Ok(text) = serde_json::to_string(report) {
        println!("{text}");
    }
}

pub fn command_base(base: Option<PathBuf>) -> Result<PathBuf, i32> {
    if crate::common::test_mode() {
        base.ok_or(2)
    } else if crate::common::sys::euid() != 0 {
        Err(4)
    } else if base.is_some() {
        Err(2)
    } else {
        Ok(PathBuf::from(super::owner::SYSTEM_BASE))
    }
}
