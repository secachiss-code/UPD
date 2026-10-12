//! `cm tunnel`: подъём и снятие туннеля по описанию из хранилища.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use crate::app::assign::{self, AssignError};
use crate::controller::client::{self, ClientError};
use crate::controller::protocol::{Op, ReplyData};
use crate::core::mihomo::config::document_from_store;
use crate::env::collect::{self, CollectError};
use crate::env::geo::{self, RegionDecision};
use crate::env::invalidate::{self, Invalidation};
use crate::env::sources;
use crate::net::policy::{FailureMode, Lifetime, TunnelPolicy};
use crate::profiles::{Id, Store};

pub fn dispatch(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("up") => up(&args[1..]),
        Some("down") => down(&args[1..]),
        Some("status") => status(&args[1..]),
        Some("verify") => verify(&args[1..]),
        _ => usage(),
    }
}

fn up(args: &[String]) -> i32 {
    let Some(id_text) = args.first() else {
        return usage();
    };
    let mut lifetime = Lifetime::Pinned;
    let mut failure = FailureMode::Block;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--lifetime" => {
                let Some(value) = args.get(index + 1) else {
                    return usage();
                };
                lifetime = match value.as_str() {
                    "pinned" => Lifetime::Pinned,
                    "refcount" => Lifetime::Refcount,
                    _ => return usage(),
                };
                index += 2;
            }
            "--failure" => {
                let Some(value) = args.get(index + 1) else {
                    return usage();
                };
                failure = match value.as_str() {
                    "block" => FailureMode::Block,
                    "failover" => FailureMode::Failover,
                    _ => return usage(),
                };
                index += 2;
            }
            _ => return usage(),
        }
    }
    let Ok(id) = Id::new(id_text) else {
        eprintln!("[unknown_tunnel]");
        return 2;
    };
    let Ok(store) = Store::open(crate::sources::cli::store_root()) else {
        eprintln!("[unknown_tunnel]");
        return 2;
    };
    let Ok(snapshot) = store.read_snapshot() else {
        eprintln!("[unknown_tunnel]");
        return 2;
    };
    let resolved = match assign::tunnel(&snapshot, &id) {
        Ok(resolved) => resolved,
        Err(error) => {
            eprintln!("[{}]", assign_token(error));
            return 2;
        }
    };
    let socket = client::socket_path();
    let (_host, tunnels) = match fetch_overview(&socket) {
        Ok(overview) => overview,
        Err(code) => return code,
    };
    let row = tunnels
        .iter()
        .find(|item| item.instance == resolved.instance);
    let state = row.map(|item| item.state.as_str()).unwrap_or("stopped");
    if state == "up" {
        return 0;
    }
    if matches!(state, "starting" | "degraded" | "blocked") {
        eprintln!("[busy]");
        return 3;
    }
    if matches!(state, "stopped" | "failed")
        && let Some(generation) = row.and_then(|item| item.worker_generation)
    {
        match client::call(
            &socket,
            Op::WorkerStop {
                instance: resolved.instance.clone(),
                generation,
            },
        ) {
            Ok(reply) if reply.ok || reply.code == "not_running" => {}
            Ok(reply) => {
                eprintln!("[{}]", reply.code);
                return 3;
            }
            Err(_) => return 4,
        }
    }
    let generation = row
        .map(|item| {
            item.worker_generation
                .unwrap_or(0)
                .max(item.net_generation.unwrap_or(0))
        })
        .unwrap_or(0)
        .saturating_add(1);
    let had_net = row.and_then(|item| item.net_generation).is_some();
    if controller(
        &socket,
        Op::InstancePrepare {
            instance: resolved.instance.clone(),
        },
    ) != 0
    {
        return 3;
    }
    let document = match document_from_store(
        &store,
        &resolved.source_id,
        resolved.selection.clone(),
        &resolved.dns,
        &[],
        true,
    ) {
        Ok(mut document) => {
            if let Some(object) = document.as_object_mut() {
                object.remove("dns");
            }
            document
        }
        Err(_) => {
            eprintln!("[invalid_config]");
            return 2;
        }
    };
    if write_generation(&resolved.instance, generation, &document).is_err() {
        return 2;
    }
    let policy = TunnelPolicy {
        ipv6: resolved.dns.ipv6,
        fake_ip: resolved.dns.fake_ip,
        lifetime,
        failure,
    };
    if write_policy(&resolved.instance, &policy).is_err() {
        return 2;
    }
    let net = match client::call(
        &socket,
        Op::NetApply {
            instance: resolved.instance.clone(),
            generation,
        },
    ) {
        Ok(reply) if reply.ok || (reply.code == "conflict" && had_net) => reply,
        Ok(reply) if reply.code == "conflict" => {
            eprintln!("[conflict]");
            return 3;
        }
        Ok(reply) => {
            eprintln!("[{}]", reply.code);
            return 3;
        }
        Err(_) => return 4,
    };
    if controller(
        &socket,
        Op::WorkerStart {
            instance: resolved.instance.clone(),
            generation,
        },
    ) != 0
    {
        return 3;
    }
    let netns = match net.data {
        Some(ReplyData::Net { netns, .. }) => netns,
        _ => String::new(),
    };
    println!(
        "instance={} generation={generation} netns={netns}",
        resolved.instance
    );
    0
}

fn down(args: &[String]) -> i32 {
    let Some(id_text) = args.first() else {
        return usage();
    };
    let mut keep_net = false;
    let mut force = false;
    for arg in &args[1..] {
        match arg.as_str() {
            "--keep-net" => keep_net = true,
            "--force" => force = true,
            _ => return usage(),
        }
    }
    let Some(instance) = instance_name(id_text) else {
        eprintln!("[unknown_tunnel]");
        return 2;
    };
    let socket = client::socket_path();
    let (_host, tunnels) = match fetch_overview(&socket) {
        Ok(overview) => overview,
        Err(code) => return code,
    };
    let Some(row) = tunnels.iter().find(|item| item.instance == instance) else {
        return 0;
    };
    if row.worker_generation.is_none() && row.net_generation.is_none() {
        return 0;
    }
    if row.holders > 0 && !force {
        eprintln!("[in_use]");
        return 5;
    }
    if let Some(generation) = row.worker_generation {
        match client::call(
            &socket,
            Op::WorkerStop {
                instance: instance.clone(),
                generation,
            },
        ) {
            Ok(reply) if reply.ok || reply.code == "not_running" => {}
            Ok(reply) => {
                eprintln!("[{}]", reply.code);
                return 3;
            }
            Err(_) => return 4,
        }
    }
    if !keep_net && let Some(generation) = row.net_generation {
        match client::call(
            &socket,
            Op::NetRevert {
                instance,
                generation,
            },
        ) {
            Ok(reply) if reply.ok || reply.code == "not_running" => {}
            Ok(reply) => {
                eprintln!("[{}]", reply.code);
                return 3;
            }
            Err(_) => return 4,
        }
    }
    0
}

fn status(args: &[String]) -> i32 {
    let Some(id_text) = args.first().filter(|_| args.len() == 1) else {
        return usage();
    };
    let Some(instance) = instance_name(id_text) else {
        eprintln!("[unknown_tunnel]");
        return 2;
    };
    let socket = client::socket_path();
    let state = match client::call(
        &socket,
        Op::TunnelState {
            instance: instance.clone(),
        },
    ) {
        Ok(reply) if reply.ok => reply,
        Ok(reply) => {
            eprintln!("[{}]", reply.code);
            return 3;
        }
        Err(_) => return 4,
    };
    let apps = client::call(
        &socket,
        Op::AppList {
            instance: instance.clone(),
        },
    );
    let Some(ReplyData::Tunnel {
        state,
        restarts,
        holders,
    }) = state.data
    else {
        return 3;
    };
    let holders = match apps {
        Ok(reply) => match reply.data {
            Some(ReplyData::Apps { pids, .. }) => pids.len() as u32,
            _ => holders,
        },
        Err(_) => holders,
    };
    let generation = worker_generation(&socket, &instance)
        .ok()
        .flatten()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned());
    println!("state={state} restarts={restarts} holders={holders} generation={generation}");
    0
}

fn verify(args: &[String]) -> i32 {
    let Some(id_text) = args.first() else {
        return usage();
    };
    let mut country = None;
    let mut custom = Vec::new();
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--country" => {
                let Some(value) = args.get(index + 1) else {
                    return usage();
                };
                country = Some(value.to_ascii_uppercase());
                index += 2;
            }
            "--source" => {
                let Some(value) = args.get(index + 1) else {
                    return usage();
                };
                let Some(source) = sources::parse_source_arg(value) else {
                    return usage();
                };
                custom.push(source);
                index += 2;
            }
            _ => return usage(),
        }
    }
    let Some(instance) = instance_name(id_text) else {
        eprintln!("[unknown_tunnel]");
        return 2;
    };
    let sources = if custom.is_empty() {
        sources::default_sources()
    } else {
        custom
    };
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(format!("/run/user/{}", crate::common::sys::euid()))
        });
    let observations = match collect::collect(
        &client::socket_path(),
        &instance,
        &sources,
        &runtime,
        15_000,
    ) {
        Ok(observations) => observations,
        Err(CollectError::Launch) => return 3,
        Err(CollectError::Timeout) => return 8,
        Err(_) => return 2,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0);
    let decision = geo::decide(&observations, now);
    let mut line = format!(
        "region={} sources={}",
        region_token(&decision),
        sources.len()
    );
    let mut code = 0;
    if let Some(expected) = &country {
        let axis = geo::region_axis(&decision, expected);
        line.push_str(&format!(" axis={}", axis_token(axis)));
        code = match axis {
            crate::profiles::VerificationValue::Verified => 0,
            crate::profiles::VerificationValue::Blocked => 7,
            _ => 8,
        };
        let path = region_path(&instance);
        if let Some(previous) = read_previous(&path) {
            let change = invalidate::on_region_change(expected, &previous, &decision);
            line.push_str(&format!(" invalidation={}", invalidation_token(&change)));
        }
        let _ = write_decision(&path, &decision);
    }
    println!("{line}");
    code
}

fn fetch_overview(
    socket: &Path,
) -> Result<
    (
        crate::controller::protocol::HostRow,
        Vec<crate::controller::protocol::TunnelRow>,
    ),
    i32,
> {
    match client::call(socket, Op::Overview) {
        Ok(reply) if reply.ok => match reply.data {
            Some(ReplyData::Overview { host, tunnels }) => Ok((host, tunnels)),
            _ => {
                eprintln!("[failed]");
                Err(3)
            }
        },
        Ok(reply) => {
            eprintln!("[{}]", reply.code);
            Err(3)
        }
        Err(_) => Err(4),
    }
}

fn worker_generation(socket: &Path, instance: &str) -> Result<Option<u64>, i32> {
    match client::call(
        socket,
        Op::WorkerStatus {
            instance: instance.to_owned(),
        },
    ) {
        Ok(reply) if reply.ok => Ok(match reply.data {
            Some(ReplyData::Status { generation, .. }) => generation,
            _ => None,
        }),
        Ok(reply) => {
            eprintln!("[{}]", reply.code);
            Err(3)
        }
        Err(ClientError::Unavailable) => Err(4),
        Err(_) => Err(4),
    }
}

fn controller(socket: &Path, op: Op) -> i32 {
    match client::call(socket, op) {
        Ok(reply) if reply.ok => 0,
        Ok(reply) => {
            eprintln!("[{}]", reply.code);
            3
        }
        Err(_) => 4,
    }
}

fn write_generation(
    instance: &str,
    generation: u64,
    document: &serde_json::Value,
) -> std::io::Result<()> {
    let dir = client::owner_config_dir(instance);
    fs::create_dir_all(&dir)?;
    let bytes = serde_json::to_vec(document)?;
    write_0600(&dir.join(format!("gen-{generation}.json")), &bytes)
}

fn write_policy(instance: &str, policy: &TunnelPolicy) -> std::io::Result<()> {
    let dir = client::owner_config_dir(instance);
    fs::create_dir_all(&dir)?;
    let bytes = serde_json::to_vec(policy)?;
    write_0600(&dir.join("net.json"), &bytes)
}

fn write_0600(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(tmp, path)
}

fn instance_name(id_text: &str) -> Option<String> {
    let id = Id::new(id_text).ok()?;
    let store = Store::open(crate::sources::cli::store_root()).ok()?;
    let snapshot = store.read_snapshot().ok()?;
    assign::tunnel(&snapshot, &id)
        .ok()
        .map(|resolved| resolved.instance)
}

fn region_token(decision: &RegionDecision) -> String {
    match decision {
        RegionDecision::Agreed { country, sources } => format!("agreed {country} {sources}"),
        RegionDecision::Disagree { countries } => format!("disagree {}", countries.join(",")),
        RegionDecision::Insufficient { fresh } => format!("insufficient {fresh}"),
        RegionDecision::Stale => "stale".to_owned(),
    }
}

fn axis_token(value: crate::profiles::VerificationValue) -> &'static str {
    match value {
        crate::profiles::VerificationValue::Verified => "verified",
        crate::profiles::VerificationValue::Blocked => "blocked",
        crate::profiles::VerificationValue::Partial => "partial",
        crate::profiles::VerificationValue::Error => "unknown",
        crate::profiles::VerificationValue::Unknown => "unknown",
    }
}

fn invalidation_token(value: &Invalidation) -> &'static str {
    match value {
        Invalidation::Keep => "keep",
        Invalidation::RegionUnknown => "region_unknown",
        Invalidation::RegionMismatch { .. } => "region_mismatch",
    }
}

fn region_path(instance: &str) -> std::path::PathBuf {
    let base = std::env::var("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_owned());
            std::path::PathBuf::from(home).join(".local/state")
        });
    base.join("cm").join(format!("region-{instance}.json"))
}

fn read_previous(path: &Path) -> Option<RegionDecision> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_decision(path: &Path, decision: &RegionDecision) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec(decision).unwrap_or_default();
    write_0600(path, &bytes)
}

fn assign_token(error: AssignError) -> &'static str {
    match error {
        AssignError::UnknownApplication => "unknown_application",
        AssignError::UnknownGroup => "unknown_group",
        AssignError::UnknownTunnel => "unknown_tunnel",
        AssignError::UnknownProfile => "unknown_profile",
        AssignError::UnknownSource => "unknown_source",
        AssignError::HostTunnel => "host_tunnel",
        AssignError::OwnerMismatch => "owner_mismatch",
        AssignError::BadInstanceName => "bad_instance_name",
    }
}

fn usage() -> i32 {
    print!(
        "{}",
        t!(
            "cm tunnel up ID [--lifetime pinned|refcount] [--failure block|failover]\ncm tunnel down ID [--keep-net] [--force]\ncm tunnel status ID\ncm tunnel verify ID [--country XX] [--source id=формат=url]\n"
        )
    );
    2
}
