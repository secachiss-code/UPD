//! Исполнение команд сети. Запреты nft ставятся до появления интерфейсов.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::profiles::Ipv6Policy;

use super::commands::{Cmd, Program, create_commands, destroy_commands};
use super::firewall::replace_commands;
use super::plan::{NetError, TunnelNet};

pub trait NetExec {
    fn run(&mut self, cmd: &Cmd) -> Result<(), NetError>;
}

pub struct SystemExec;

impl NetExec for SystemExec {
    fn run(&mut self, cmd: &Cmd) -> Result<(), NetError> {
        let program = match cmd.program {
            Program::Ip => "/usr/bin/ip",
            Program::Nft => "/usr/bin/nft",
            Program::Sysctl => "/usr/bin/sysctl",
        };
        let mut command = Command::new(program);
        command
            .args(&cmd.args)
            .env_clear()
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if cmd.stdin.is_some() {
            command.stdin(Stdio::piped());
        } else {
            command.stdin(Stdio::null());
        }
        let mut child = command.spawn().map_err(|_| NetError::CommandFailed)?;
        if let Some(text) = &cmd.stdin
            && let Some(mut stdin) = child.stdin.take()
        {
            stdin
                .write_all(text.as_bytes())
                .map_err(|_| NetError::CommandFailed)?;
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return if status.success() {
                        Ok(())
                    } else {
                        Err(NetError::CommandFailed)
                    };
                }
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(NetError::Timeout);
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => return Err(NetError::CommandFailed),
            }
        }
    }
}

#[derive(Default)]
pub struct RecordingExec {
    pub ran: Vec<Cmd>,
    pub fail_at: Option<usize>,
}

impl NetExec for RecordingExec {
    fn run(&mut self, cmd: &Cmd) -> Result<(), NetError> {
        self.ran.push(cmd.clone());
        if cmd.program != Program::Nft {
            let seen = self
                .ran
                .iter()
                .filter(|item| item.program != Program::Nft)
                .count();
            if self.fail_at == Some(seen) {
                return Err(NetError::CommandFailed);
            }
        }
        Ok(())
    }
}

pub fn create(
    tunnel: &TunnelNet,
    ipv6: Ipv6Policy,
    all: &[TunnelNet],
    exec: &mut dyn NetExec,
) -> Result<(), NetError> {
    for cmd in replace_commands(all) {
        exec.run(&cmd)?;
    }
    for cmd in create_commands(tunnel, ipv6) {
        if exec.run(&cmd).is_err() {
            let rollback = destroy_commands(tunnel);
            let last = rollback.len().saturating_sub(1);
            for (index, undo) in rollback.iter().enumerate() {
                if exec.run(undo).is_err() && index == last {
                    return Err(NetError::CommandFailed);
                }
            }
            return Err(NetError::CommandFailed);
        }
    }
    Ok(())
}

/// Удаление идемпотентно: каждая команда выполняется, даже если предыдущая отказала.
/// Отказ команды удаления значит, что части сети уже нет (TUN убрали руками, обрыв
/// посреди прошлого удаления), и ошибкой не считается: иначе такую сеть нельзя было бы
/// ни снять, ни создать заново. Оставшийся после настоящего отказа интерфейс найдёт
/// сверка (`OrphanVeth`), а до тех пор он закрыт запретами таблицы.
pub fn destroy(
    tunnel: &TunnelNet,
    remaining: &[TunnelNet],
    exec: &mut dyn NetExec,
) -> Result<(), NetError> {
    for cmd in destroy_commands(tunnel) {
        let _ = exec.run(&cmd);
    }
    for cmd in replace_commands(remaining) {
        exec.run(&cmd)?;
    }
    Ok(())
}
