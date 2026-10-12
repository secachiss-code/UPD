//! `cm host`: смена режима хоста через контроллер. Службу `cm vpn` эта команда не трогает.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use crate::controller::client::{self, ClientError};
use crate::controller::owner::SYSTEM_BASE;
use crate::controller::protocol::{Op, ReplyData};
use crate::net::host::{self, HostStep};
use crate::net::host_plan::HOST_INSTANCE;
use crate::net::query::SystemQuery;
use crate::profiles::HostMode;

pub fn dispatch(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("status") if args.len() == 1 => status(),
        Some("check") => check(&args[1..]),
        Some("set") => set(&args[1..]),
        _ => usage(),
    }
}

fn status() -> i32 {
    let reply = match client::call(&client::socket_path(), Op::HostStatus) {
        Ok(reply) => reply,
        Err(_) => return 4,
    };
    let Some(ReplyData::Host {
        mode,
        generation,
        uplinks,
    }) = reply.data
    else {
        eprintln!("[{}]", reply.code);
        return 3;
    };
    if !reply.ok {
        eprintln!("[{}]", reply.code);
        return 3;
    }
    println!("{}", status_line(&mode, generation, &uplinks));
    0
}

fn set(args: &[String]) -> i32 {
    let Some(mode_text) = args.first().map(String::as_str) else {
        return usage();
    };
    let Some(target) = parse_mode(mode_text) else {
        return usage();
    };
    let config = match config_arg(&args[1..]) {
        Ok(path) => path,
        Err(()) => return usage(),
    };
    if matches!(target, HostMode::Proxy | HostMode::Tunnel) && config.is_none() {
        return usage();
    }
    if matches!(target, HostMode::Off) && config.is_some() {
        return usage();
    }
    let socket = client::socket_path();
    let current = match client::call(&socket, Op::HostStatus) {
        Ok(reply) => reply,
        Err(_) => return 4,
    };
    let Some(ReplyData::Host {
        mode,
        generation: host_generation,
        ..
    }) = current.data
    else {
        eprintln!("[{}]", current.code);
        return 3;
    };
    if !current.ok {
        eprintln!("[{}]", current.code);
        return 3;
    }
    let worker = match client::call(
        &socket,
        Op::WorkerStatus {
            instance: HOST_INSTANCE.to_owned(),
        },
    ) {
        Ok(reply) => reply,
        Err(_) => return 4,
    };
    let worker_generation = match worker.data {
        Some(ReplyData::Status { generation, .. }) if worker.ok => generation,
        _ => {
            eprintln!("[{}]", worker.code);
            return 3;
        }
    };
    let steps = host::transition(parse_mode(&mode).unwrap_or(HostMode::Off), target);
    if steps.is_empty() {
        return 0;
    }
    let next = worker_generation.map_or(1, |generation| generation.saturating_add(1));
    let mut applied = false;
    for step in steps {
        let outcome = match step {
            HostStep::StopWorker => stop_worker(&socket, worker_generation),
            HostStep::RevertNet => {
                let Some(generation) = host_generation else {
                    return 3;
                };
                controller(&socket, Op::HostRevert { generation })
            }
            HostStep::ApplyNet(mode) => {
                let text = match mode {
                    HostMode::Proxy => "proxy",
                    HostMode::Tunnel => "tunnel",
                    HostMode::Off => return usage(),
                };
                if let Err(code) = prepare_and_copy(&socket, next, config.as_deref()) {
                    return code;
                }
                let code = controller(
                    &socket,
                    Op::HostApply {
                        generation: next,
                        mode: text.to_owned(),
                    },
                );
                if code == 0 {
                    applied = true;
                }
                code
            }
            HostStep::StartWorker => controller(
                &socket,
                Op::WorkerStart {
                    instance: HOST_INSTANCE.to_owned(),
                    generation: next,
                },
            ),
        };
        if outcome != 0 {
            if applied && !matches!(step, HostStep::ApplyNet(_)) {
                let _ = client::call(&socket, Op::HostRevert { generation: next });
            }
            return outcome;
        }
    }
    0
}

fn stop_worker(socket: &Path, generation: Option<u64>) -> i32 {
    let Some(generation) = generation else {
        return 0;
    };
    match client::call(
        socket,
        Op::WorkerStop {
            instance: HOST_INSTANCE.to_owned(),
            generation,
        },
    ) {
        Ok(reply) if reply.ok || reply.code == "not_running" => 0,
        Ok(reply) => {
            eprintln!("[{}]", reply.code);
            3
        }
        Err(_) => 4,
    }
}

fn prepare_and_copy(socket: &Path, generation: u64, config: Option<&Path>) -> Result<(), i32> {
    controller(
        socket,
        Op::InstancePrepare {
            instance: HOST_INSTANCE.to_owned(),
        },
    )
    .pipe_err()?;
    let Some(config) = config else {
        return Ok(());
    };
    copy_config(generation, config).map_err(|_| 2)
}

fn copy_config(generation: u64, source: &Path) -> std::io::Result<()> {
    let dir = client::owner_config_dir(HOST_INSTANCE);
    fs::create_dir_all(&dir)?;
    let bytes = fs::read(source)?;
    let dest = dir.join(format!("gen-{generation}.json"));
    let tmp = dir.join(format!("gen-{generation}.json.tmp"));
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    fs::rename(tmp, dest)
}

fn controller(socket: &Path, op: Op) -> i32 {
    match client::call(socket, op) {
        Ok(reply) if reply.ok => 0,
        Ok(reply) => {
            eprintln!("[{}]", reply.code);
            3
        }
        Err(ClientError::Unavailable) => 4,
        Err(_) => 4,
    }
}

fn config_arg(args: &[String]) -> Result<Option<std::path::PathBuf>, ()> {
    if args.is_empty() {
        return Ok(None);
    }
    if args.len() == 2 && args[0] == "--config" && !args[1].starts_with('-') {
        return Ok(Some(std::path::PathBuf::from(&args[1])));
    }
    Err(())
}

fn parse_mode(text: &str) -> Option<HostMode> {
    match text {
        "off" => Some(HostMode::Off),
        "proxy" => Some(HostMode::Proxy),
        "tunnel" => Some(HostMode::Tunnel),
        _ => None,
    }
}

fn status_line(mode: &str, generation: Option<u64>, uplinks: &[String]) -> String {
    let generation = generation
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let uplinks = if uplinks.is_empty() {
        "-".to_owned()
    } else {
        uplinks.join(",")
    };
    format!("mode={mode} generation={generation} uplinks={uplinks}")
}

fn check(args: &[String]) -> i32 {
    let json = match args {
        [] => false,
        [flag] if flag == "--json" => true,
        _ => return usage(),
    };
    let mut query = SystemQuery;
    let facts = host::collect_facts(
        &mut query,
        Path::new("/run"),
        Path::new("/proc"),
        &client::socket_path(),
        &host_state_file(),
        Path::new("/usr/bin/nft").is_file(),
        crate::common::unit_state(crate::vpn::SERVICE) == "active",
    );
    let lines = host::check_lines(&facts);
    if json {
        println!("{}", host::render_json(&lines, &facts));
    } else {
        print!("{}", host::render_text(&lines, &facts));
    }
    host::status_code(&lines)
}

fn host_state_file() -> std::path::PathBuf {
    let base = if crate::common::test_mode() {
        std::env::var("CM_CONTROLLER_BASE").unwrap_or_else(|_| SYSTEM_BASE.to_owned())
    } else {
        SYSTEM_BASE.to_owned()
    };
    std::path::PathBuf::from(base).join("host.json")
}

fn usage() -> i32 {
    print!(
        "{}",
        t!("cm host status\ncm host check [--json]\ncm host set off|proxy|tunnel --config ФАЙЛ\n")
    );
    2
}

trait Code {
    fn pipe_err(self) -> Result<(), i32>;
}

impl Code for i32 {
    fn pipe_err(self) -> Result<(), i32> {
        if self == 0 { Ok(()) } else { Err(self) }
    }
}
