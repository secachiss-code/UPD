//! Снятие всего, что поставил CM: ядра, сети приложений, туннель хоста, таблица `inet cm`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::core::leases::{Lease, LeaseError, LeaseRegistry, ResourceKind};
use crate::net::commands::{Cmd, Program};
use crate::net::host_commands::host_destroy_commands;
use crate::net::host_firewall::host_firewall_commands;
use crate::net::host_plan;
use crate::net::observe::{ObservedRule, parse_links, parse_rules};
use crate::net::query::{self, NetQuery};
use crate::net::{self, TunnelNet};

use super::core_ops::{Workers, kill_pidfile};
use super::host_ops::{self, HOST_NET_KEY};
use super::net_ops::NetCtx;
use super::registry::Generations;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TeardownReport {
    pub cores: u32,
    pub nets: u32,
    pub host: bool,
    pub errors: u32,
}

pub fn teardown_all(base: &Path, workers: &Workers, nets: &Arc<NetCtx>) -> TeardownReport {
    let mut report = TeardownReport::default();
    // Ожидание — до мьютекса сторожа. Кадр TunnelState держит busy и затем берёт
    // тот же мьютекс; обратный порядок зацикливает оба.
    report.errors = report.errors.saturating_add(wait_busy(workers));
    let watch = workers.watch_handle();
    let mut hold = watch
        .as_ref()
        .map(|watch| watch.lock().unwrap_or_else(|error| error.into_inner()));
    if let Some(guard) = hold.as_mut() {
        for (uid, instance) in tracked_instances(base, workers) {
            guard.note_stop(uid, &instance);
        }
    }
    let (cores, errors) = stop_cores(base, workers);
    report.cores = cores;
    report.errors = report.errors.saturating_add(errors);
    let (nets_n, errors) = drop_nets(base, nets);
    report.nets = nets_n;
    report.errors = report.errors.saturating_add(errors);
    let (host, errors) = drop_host(base, nets);
    report.host = host;
    report.errors = report.errors.saturating_add(errors);
    run_commands(
        nets,
        &net::firewall::replace_commands(&[]),
        &mut report.errors,
    );
    report
}

fn tracked_instances(base: &Path, workers: &Workers) -> Vec<(u32, String)> {
    let mut ids = BTreeSet::new();
    ids.extend(workers.running());
    ids.extend(workers.busy_instances());
    for uid in owner_uids(base) {
        ids.insert((uid, crate::net::host_plan::HOST_INSTANCE.to_owned()));
        for instance in instance_names(base, uid) {
            ids.insert((uid, instance));
        }
    }
    ids.into_iter().collect()
}

fn wait_busy(workers: &Workers) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let busy = workers.busy_instances();
        if busy.is_empty() {
            return 0;
        }
        if Instant::now() >= deadline {
            return busy.len() as u32;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn stop_cores(base: &Path, workers: &Workers) -> (u32, u32) {
    let mut cores = 0u32;
    let mut errors = 0u32;
    let mut seen = BTreeSet::new();
    let mut running = workers.running();
    running.sort();
    for (uid, instance) in running {
        seen.insert((uid, instance.clone()));
        match workers.stop_memory(uid, &instance) {
            Ok(true) => cores = cores.saturating_add(1),
            Ok(false) => {}
            Err(_) => {
                errors = errors.saturating_add(1);
                cores = cores.saturating_add(1);
            }
        }
        let _ = signal_pid(base, uid, &instance);
    }
    let mut uids = owner_uids(base);
    for (uid, _) in &seen {
        uids.push(*uid);
    }
    uids.sort_unstable();
    uids.dedup();
    for uid in uids {
        for instance in instance_names(base, uid) {
            if seen.contains(&(uid, instance.clone())) {
                continue;
            }
            if signal_pid(base, uid, &instance) {
                cores = cores.saturating_add(1);
            }
        }
        clear_core_generations(base, uid, &mut errors);
    }
    (cores, errors)
}

fn signal_pid(base: &Path, uid: u32, instance: &str) -> bool {
    let path = pid_path(base, uid, instance);
    path.is_file() && kill_pidfile(&path, uid)
}

fn pid_path(base: &Path, uid: u32, instance: &str) -> PathBuf {
    base.join(format!("u{uid}"))
        .join("instances")
        .join(instance)
        .join("core.pid")
}

fn clear_core_generations(base: &Path, uid: u32, errors: &mut u32) {
    let path = generations_path(base, uid);
    if !path.is_file() {
        return;
    }
    let Ok(mut generations) = Generations::load(&path) else {
        *errors = errors.saturating_add(1);
        return;
    };
    let keys: Vec<String> = generations
        .entries()
        .into_iter()
        .map(|(key, _)| key)
        .filter(|key| !key.starts_with("net:"))
        .collect();
    for key in keys {
        if generations.clear(&key).is_err() {
            *errors = errors.saturating_add(1);
        }
    }
}

fn drop_nets(base: &Path, nets: &NetCtx) -> (u32, u32) {
    let mut errors = 0u32;
    let Ok(registry) = LeaseRegistry::open(&nets.lease_dir) else {
        return (0, 0);
    };
    let Ok(leases) = registry.list() else {
        return (0, 1);
    };
    let mut tunnels: Vec<Lease> = leases
        .iter()
        .filter(|lease| lease.kind == ResourceKind::Tunnel)
        .cloned()
        .collect();
    tunnels.sort_by(|left, right| {
        left.value
            .cmp(&right.value)
            .then(left.holder.cmp(&right.holder))
    });
    let planned: Vec<Option<(u32, String, u8, TunnelNet)>> =
        tunnels.iter().map(tunnel_of).collect();
    for (index, lease) in tunnels.iter().enumerate() {
        let Some((uid, instance, number, tunnel)) = planned[index].clone() else {
            errors = errors.saturating_add(1);
            if let Some((uid, instance)) = parse_holder(&lease.holder) {
                clear_generation(base, uid, &format!("net:{instance}"), &mut errors);
            }
            if release_holder(&registry, &lease.holder).is_err() {
                errors = errors.saturating_add(1);
            }
            continue;
        };
        let remaining: Vec<TunnelNet> = planned
            .iter()
            .skip(index + 1)
            .filter_map(|item| item.as_ref().map(|(_, _, _, tunnel)| tunnel.clone()))
            .collect();
        {
            let mut exec = nets.exec.lock().unwrap_or_else(|error| error.into_inner());
            if net::destroy(&tunnel, &remaining, &mut *exec).is_err() {
                errors = errors.saturating_add(1);
            }
        }
        remove_netns_dir(&nets.etc_root, number, &mut errors);
        if release_holder(&registry, &lease.holder).is_err() {
            errors = errors.saturating_add(1);
        }
        clear_generation(base, uid, &format!("net:{instance}"), &mut errors);
    }
    (tunnels.len() as u32, errors)
}

fn tunnel_of(lease: &Lease) -> Option<(u32, String, u8, TunnelNet)> {
    let (uid, instance) = parse_holder(&lease.holder)?;
    let number = lease.value.parse::<u8>().ok()?;
    let tunnel = net::tunnel_net(number, uid).ok()?;
    Some((uid, instance, number, tunnel))
}

fn parse_holder(holder: &str) -> Option<(u32, String)> {
    let rest = holder.strip_prefix('u')?;
    let (uid, instance) = rest.split_once('-')?;
    let uid = uid.parse().ok()?;
    if instance.is_empty() {
        return None;
    }
    Some((uid, instance.to_owned()))
}

fn release_holder(registry: &LeaseRegistry, holder: &str) -> Result<(), LeaseError> {
    let leases = registry.list()?;
    for lease in leases.into_iter().filter(|lease| lease.holder == holder) {
        match registry.release(&lease.holder, lease.kind, &lease.value) {
            Ok(()) | Err(LeaseError::Conflict) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn remove_netns_dir(root: &Path, index: u8, errors: &mut u32) {
    let dir = root.join(format!("cm-{index}"));
    if !dir.exists() {
        return;
    }
    let _ = fs::remove_file(dir.join("resolv.conf"));
    if fs::remove_dir(&dir).is_err() {
        *errors = errors.saturating_add(1);
    }
}

fn drop_host(base: &Path, nets: &NetCtx) -> (bool, u32) {
    let mut errors = 0u32;
    match host_ops::load_state(base) {
        Ok(Some(state)) => {
            let host = host_plan::host_net(state.uid);
            run_commands(nets, &host_firewall_commands(None), &mut errors);
            run_commands(
                nets,
                &host_destroy_commands(&host, &state.uplinks),
                &mut errors,
            );
            delete_file(&host_ops::state_path(base), &mut errors);
            clear_generation(base, state.uid, HOST_NET_KEY, &mut errors);
            (true, errors)
        }
        Ok(None) => drop_host_observed(base, nets),
        Err(_) => {
            errors = errors.saturating_add(1);
            let (found, more) = drop_host_observed(base, nets);
            (found, errors.saturating_add(more))
        }
    }
}

fn drop_host_observed(base: &Path, nets: &NetCtx) -> (bool, u32) {
    let mut errors = 0u32;
    let mut query = nets.query.lock().unwrap_or_else(|error| error.into_inner());
    let marks = observe_marks(&mut *query, &mut errors);
    if !marks.present {
        return (false, errors);
    }
    let uplinks = match query::observe_uplinks(&mut *query) {
        Ok(uplinks) => uplinks,
        Err(_) => {
            errors = errors.saturating_add(1);
            Vec::new()
        }
    };
    drop(query);
    let host = host_plan::host_net(0);
    run_commands(nets, &host_firewall_commands(None), &mut errors);
    run_commands(nets, &host_destroy_commands(&host, &uplinks), &mut errors);
    run_commands(nets, &rule_deletes(&marks), &mut errors);
    for uid in owner_uids(base) {
        clear_generation(base, uid, HOST_NET_KEY, &mut errors);
    }
    (true, errors)
}

struct Marks {
    present: bool,
    rules: Vec<(bool, u32)>,
}

fn observe_marks(query: &mut dyn NetQuery, errors: &mut u32) -> Marks {
    let links = match query.query(Program::Ip, &["-j", "link", "show"]) {
        Ok(text) => match parse_links(&text) {
            Ok(links) => links,
            Err(_) => {
                *errors = errors.saturating_add(1);
                Vec::new()
            }
        },
        Err(_) => {
            *errors = errors.saturating_add(1);
            Vec::new()
        }
    };
    let rules = match query.query(Program::Ip, &["-j", "rule", "show"]) {
        Ok(text) => match parse_rules(&text) {
            Ok(rules) => rules,
            Err(_) => {
                *errors = errors.saturating_add(1);
                Vec::new()
            }
        },
        Err(_) => {
            *errors = errors.saturating_add(1);
            Vec::new()
        }
    };
    let rules6 = match query.query(Program::Ip, &["-j", "-6", "rule", "show"]) {
        Ok(text) => parse_rules(&text).unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    let mut matched = Vec::new();
    for rule in rules.iter().filter(|rule| scheme_rule(rule)) {
        matched.push((false, rule.priority));
    }
    for rule in rules6.iter().filter(|rule| scheme_rule(rule)) {
        matched.push((true, rule.priority));
    }
    matched.sort_unstable();
    matched.dedup();
    let present = links
        .iter()
        .any(|name| name == "cmtunh" || name == "cmsink")
        || !matched.is_empty();
    Marks {
        present,
        rules: matched,
    }
}

/// Правило схемы CM: диапазоны внешних интерфейсов и 1990 смотрят в `main`,
/// 2000 — в таблицу 90, 2001 — в таблицу 91. Чужая таблица на том же приоритете не наша.
fn scheme_rule(rule: &ObservedRule) -> bool {
    match rule.priority {
        1900..=1907 | 1950..=1981 | 1990 => rule.table == "main",
        2000 => rule.table == "90",
        2001 => rule.table == "91",
        _ => false,
    }
}

fn rule_deletes(marks: &Marks) -> Vec<Cmd> {
    marks
        .rules
        .iter()
        .map(|(v6, priority)| rule_del(*v6, *priority))
        .collect()
}

fn rule_del(v6: bool, priority: u32) -> Cmd {
    let mut args = Vec::new();
    if v6 {
        args.push("-6".to_owned());
    }
    args.push("rule".to_owned());
    args.push("del".to_owned());
    args.push("priority".to_owned());
    args.push(priority.to_string());
    Cmd {
        program: Program::Ip,
        args,
        stdin: None,
    }
}

fn run_commands(nets: &NetCtx, commands: &[Cmd], errors: &mut u32) {
    let mut exec = nets.exec.lock().unwrap_or_else(|error| error.into_inner());
    for command in commands {
        if exec.run(command).is_err() {
            *errors = errors.saturating_add(1);
        }
    }
}

fn delete_file(path: &Path, errors: &mut u32) {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => *errors = errors.saturating_add(1),
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
            *errors = errors.saturating_add(1);
        }
        Ok(_) => {
            if fs::remove_file(path).is_err() {
                *errors = errors.saturating_add(1);
            }
        }
    }
}

fn clear_generation(base: &Path, uid: u32, key: &str, errors: &mut u32) {
    let path = generations_path(base, uid);
    if !path.is_file() {
        return;
    }
    let Ok(mut generations) = Generations::load(&path) else {
        *errors = errors.saturating_add(1);
        return;
    };
    if generations.current(key).is_none() {
        return;
    }
    if generations.clear(key).is_err() {
        *errors = errors.saturating_add(1);
    }
}

fn generations_path(base: &Path, uid: u32) -> PathBuf {
    base.join(format!("u{uid}")).join("generations.json")
}

fn owner_uids(base: &Path) -> Vec<u32> {
    let mut ids = Vec::new();
    let Ok(entries) = fs::read_dir(base) else {
        return ids;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
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

fn instance_names(base: &Path, uid: u32) -> Vec<String> {
    let dir = base.join(format!("u{uid}")).join("instances");
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(meta) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            continue;
        }
        names.push(name);
    }
    names.sort();
    names
}
