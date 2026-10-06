//! Serialize launches across the applet and application entry point.
use serde::{Deserialize, Serialize};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize)]
struct Terminal {
    pid: u32,
    started: String,
}

fn identity(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<_> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    if matches!(*fields.first()?, "Z" | "X") {
        return None;
    }
    Some(fields.get(19)?.to_string())
}
fn directory() -> PathBuf {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").filter(|value| !value.is_empty()) {
        PathBuf::from(runtime).join("cm")
    } else if let Some(cache) = std::env::var_os("XDG_CACHE_HOME").filter(|value| !value.is_empty())
    {
        PathBuf::from(cache).join("cm")
    } else {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into())).join(".cache/cm")
    }
}

pub fn open(force_new: bool, spawn: impl FnOnce() -> Result<u32, String>) -> Result<(), String> {
    open_in(&directory(), force_new, spawn, focus)
}
fn open_in(
    directory: &Path,
    force_new: bool,
    spawn: impl FnOnce() -> Result<u32, String>,
    focus: impl FnOnce(u32),
) -> Result<(), String> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
        .map_err(|e| e.to_string())?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(directory.join("tui-launch.lock"))
        .map_err(|e| e.to_string())?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let path = directory.join("tui-terminal.json");
    let mut terminals = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<Terminal>>(&bytes).ok())
        .unwrap_or_default();
    terminals.retain(|terminal| identity(terminal.pid).as_deref() == Some(&terminal.started));
    if !force_new {
        if let Some(terminal) = terminals.last() {
            focus(terminal.pid);
            return Ok(());
        }
    }
    let pid = spawn()?;
    if let Some(started) = identity(pid) {
        terminals.push(Terminal { pid, started });
        let data = serde_json::to_vec(&terminals).map_err(|e| e.to_string())?;
        cm::common::atomic_write(&path, &data, 0o600).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// X11 terminals can be raised by PID. Wayland compositors control activation;
/// if no supported focus mechanism exists, keep the open TUI without duplicating it.
fn focus(pid: u32) {
    let mut policy = cm::common::CapturePolicy::background(Some(64 << 10));
    policy.deadline = Instant::now() + Duration::from_millis(500);
    let Ok(output) =
        cm::common::capture_with_policy(std::process::Command::new("wmctrl").arg("-lp"), policy)
    else {
        return;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let pid = pid.to_string();
    for line in text.lines() {
        let columns: Vec<_> = line.split_whitespace().collect();
        if columns.len() >= 4 && columns[2] == pid {
            let _ = crate::launch::spawn(
                std::process::Command::new("wmctrl").args(["-ia", columns[0]]),
            );
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cm::common::contract_fixtures::TempDirGuard;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    fn repeat_launch_focuses_live_terminal_and_force_opens_another() {
        let dir = TempDirGuard::new("tui-deduplicate").unwrap();
        let pid = std::process::id();
        let launched = AtomicUsize::new(0);
        let focused = AtomicUsize::new(0);
        let spawn = || {
            launched.fetch_add(1, Ordering::Relaxed);
            Ok(pid)
        };
        open_in(dir.path(), false, spawn, |_| {
            panic!("first launch cannot focus")
        })
        .unwrap();
        open_in(dir.path(), false, spawn, |seen| {
            assert_eq!(seen, pid);
            focused.fetch_add(1, Ordering::Relaxed);
        })
        .unwrap();
        assert_eq!(launched.load(Ordering::Relaxed), 1);
        assert_eq!(focused.load(Ordering::Relaxed), 1);
        open_in(dir.path(), true, spawn, |_| {
            panic!("forced launch cannot focus")
        })
        .unwrap();
        assert_eq!(launched.load(Ordering::Relaxed), 2);
    }
    #[test]
    fn reused_pid_and_failed_launch_do_not_suppress_next_launch() {
        let dir = TempDirGuard::new("tui-stale-terminal").unwrap();
        let path = dir.path().join("tui-terminal.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&vec![Terminal {
                pid: std::process::id(),
                started: "wrong-start-time".into(),
            }])
            .unwrap(),
        )
        .unwrap();
        assert!(
            open_in(
                dir.path(),
                false,
                || Err("missing terminal".into()),
                |_| panic!("stale PID cannot focus")
            )
            .is_err()
        );
        open_in(
            dir.path(),
            false,
            || Ok(std::process::id()),
            |_| panic!("stale PID cannot focus"),
        )
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<Vec<Terminal>>(&std::fs::read(path).unwrap()).unwrap()[0]
                .started,
            identity(std::process::id()).unwrap()
        );
    }
}
