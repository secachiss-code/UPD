//! `cm controller serve`: unix-сокет, один поток на соединение, не больше 32 соединений.

use std::fs;
use std::io::{BufReader, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::actions::PolkitAuthorizer;
use super::codec::{self, read_frame};
use super::core_ops::{MihomoFactory, Workers};
use super::dispatch::{Deps, OpSlots, handle};
use super::owner::SYSTEM_BASE;
use super::protocol::Reply;
use crate::common::{sys, test_mode};
use crate::core::unit::CORE_BIN;

const MAX_CONNECTIONS: usize = 32;

static STOP: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicUsize = AtomicUsize::new(0);

pub fn dispatch(args: &[String]) -> i32 {
    if args.first().map(String::as_str) == Some("reconcile") {
        return reconcile_command(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("teardown") {
        return teardown_command(&args[1..]);
    }
    if args.first().map(String::as_str) != Some("serve") {
        usage();
        return 2;
    }
    let mut socket = None;
    let mut base = None;
    let mut rest = args[1..].iter();
    while let Some(arg) = rest.next() {
        let value = match arg.as_str() {
            "--socket" | "--base" => match rest.next() {
                Some(value) if !value.starts_with('-') => value.clone(),
                _ => {
                    usage();
                    return 2;
                }
            },
            _ => {
                usage();
                return 2;
            }
        };
        match arg.as_str() {
            "--socket" => socket = Some(value),
            "--base" => base = Some(value),
            _ => unreachable!(),
        }
    }
    let Some(socket) = socket else {
        usage();
        return 2;
    };
    let base = if test_mode() {
        let Some(base) = base else {
            usage();
            return 2;
        };
        PathBuf::from(base)
    } else {
        if sys::euid() != 0 {
            eprintln!("cm controller: permission denied");
            return 4;
        }
        if base.is_some() {
            eprintln!("cm controller serve --socket PATH");
            return 2;
        }
        PathBuf::from(SYSTEM_BASE)
    };
    match serve(Path::new(&socket), &base) {
        Ok(code) => code,
        Err(code) => code,
    }
}

fn reconcile_command(args: &[String]) -> i32 {
    let mut base = None;
    let mut boot = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--boot" => boot = true,
            "--base" => {
                let Some(value) = rest.next().filter(|value| !value.starts_with('-')) else {
                    return 2;
                };
                base = Some(PathBuf::from(value));
            }
            _ => return 2,
        }
    }
    if !boot {
        return 2;
    }
    let base = match super::boot::command_base(base) {
        Ok(base) => base,
        Err(code) => return code,
    };
    let lease_dir = base.join("leases");
    let workers = Arc::new(Workers::new(Arc::new(MihomoFactory {
        binary: core_binary(),
        lease_dir: lease_dir.clone(),
    })));
    let nets = Arc::new(super::net_ops::NetCtx {
        exec: Arc::new(Mutex::new(crate::net::SystemExec)),
        query: Arc::new(Mutex::new(crate::net::query::SystemQuery)),
        lease_dir,
        etc_root: if test_mode() {
            base.join("etc-netns")
        } else {
            PathBuf::from("/etc/netns")
        },
    });
    workers.set_net(Arc::clone(&nets));
    match super::boot::reconcile_all(&base, &workers, &nets, unix_now()) {
        Ok(report) => {
            super::boot::print_report(&report);
            0
        }
        Err(_) => 4,
    }
}

fn serve(socket: &Path, base: &Path) -> Result<i32, i32> {
    STOP.store(false, Ordering::SeqCst);
    install_signals();
    let (listener, remove_socket) = listen(socket)?;
    let _ = listener.set_nonblocking(true);
    let binary = core_binary();
    let lease_dir = base.join("leases");
    let workers = Arc::new(Workers::new(Arc::new(MihomoFactory {
        binary,
        lease_dir: lease_dir.clone(),
    })));
    let nets = Arc::new(super::net_ops::NetCtx {
        exec: Arc::new(Mutex::new(crate::net::SystemExec)),
        query: Arc::new(Mutex::new(crate::net::query::SystemQuery)),
        lease_dir,
        etc_root: if test_mode() {
            base.join("etc-netns")
        } else {
            PathBuf::from("/etc/netns")
        },
    });
    workers.set_net(Arc::clone(&nets));
    let _ = super::boot::reconcile_all(base, &workers, &nets, unix_now());
    let watch = Arc::new(Mutex::new(super::watch::Watch::new()));
    workers.set_watch(Arc::clone(&watch));
    let watch_base = base.to_owned();
    let watch_workers = Arc::clone(&workers);
    let watch_nets = Arc::clone(&nets);
    std::thread::spawn(move || {
        while !STOP.load(Ordering::SeqCst) {
            let now = unix_now().saturating_mul(1000);
            {
                let mut guard = watch.lock().unwrap_or_else(|error| error.into_inner());
                guard.tick(&watch_base, &watch_workers, &watch_nets, now);
            }
            let mut left = super::watch::WATCH_INTERVAL_MS;
            while left > 0 && !STOP.load(Ordering::SeqCst) {
                let step = left.min(200);
                std::thread::sleep(Duration::from_millis(step));
                left -= step;
            }
        }
    });
    let deps = Arc::new(Deps {
        base: base.to_owned(),
        authorizer: Arc::new(PolkitAuthorizer),
        workers: Arc::clone(&workers),
        slots: Arc::new(OpSlots::new()),
        now: unix_now,
        nets,
    });
    loop {
        if STOP.load(Ordering::SeqCst) {
            break;
        }
        let mut pollfd = libc::pollfd {
            fd: listener.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one live pollfd; the 200 ms timeout only waits for the next connection or stop.
        let rc = unsafe { libc::poll(&mut pollfd, 1, 200) };
        if rc <= 0 {
            continue;
        }
        match listener.accept() {
            Ok((stream, _)) => accept_one(stream, Arc::clone(&deps)),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(_) => continue,
        }
    }
    workers.stop_all();
    if remove_socket {
        let _ = fs::remove_file(socket);
    }
    Ok(0)
}

fn accept_one(stream: UnixStream, deps: Arc<Deps>) {
    let live = LIVE.fetch_add(1, Ordering::SeqCst);
    if live >= MAX_CONNECTIONS {
        LIVE.fetch_sub(1, Ordering::SeqCst);
        drop(stream);
        return;
    }
    std::thread::spawn(move || {
        let _guard = ConnectionGuard;
        connection(stream, &deps);
    });
}

fn connection(mut stream: UnixStream, deps: &Deps) {
    let peer = match super::peer::capture(&stream) {
        Ok(peer) => peer,
        Err(error) => {
            let _ = stream.write_all(&codec::encode(&Reply::error("", error)));
            return;
        }
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(cloned);
    loop {
        if STOP.load(Ordering::SeqCst) {
            break;
        }
        match read_frame(&mut reader) {
            Ok(None) => break,
            Ok(Some(frame)) => {
                let reply = handle(&frame, &peer, deps);
                if stream.write_all(&reply).is_err() {
                    break;
                }
            }
            Err(error) => {
                let _ = stream.write_all(&codec::encode(&Reply::error("", error)));
                break;
            }
        }
    }
}

fn listen(path: &Path) -> Result<(UnixListener, bool), i32> {
    if systemd_socket() {
        // SAFETY: LISTEN_PID names this process, so fd 3 is the listener systemd passed and nobody else owns it.
        let listener = unsafe { UnixListener::from_raw_fd(3) };
        return Ok((listener, false));
    }
    let parent = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .ok_or(4)?;
    let meta = fs::symlink_metadata(parent).map_err(|_| 4)?;
    if meta.file_type().is_symlink() || !meta.is_dir() || meta.uid() != sys::euid() {
        return Err(4);
    }
    if let Ok(existing) = fs::symlink_metadata(path) {
        if existing.file_type().is_symlink() || !existing.file_type().is_socket() {
            return Err(4);
        }
        fs::remove_file(path).map_err(|_| 4)?;
    }
    let listener = UnixListener::bind(path).map_err(|_| 4)?;
    let mut perms = fs::metadata(path).map_err(|_| 4)?.permissions();
    perms.set_mode(0o666);
    fs::set_permissions(path, perms).map_err(|_| 4)?;
    Ok((listener, true))
}

fn systemd_socket() -> bool {
    std::env::var("LISTEN_PID")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        == Some(std::process::id())
        && std::env::var("LISTEN_FDS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0)
            >= 1
}

fn core_binary() -> PathBuf {
    if test_mode()
        && let Some(path) = std::env::var_os("CM_CORE_BIN")
    {
        return PathBuf::from(path);
    }
    PathBuf::from(CORE_BIN)
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

fn install_signals() {
    // SAFETY: the handler only stores an atomic flag. That is async-signal-safe.
    // sighandler_t is an integer on this libc, so the function pointer is passed as its address.
    unsafe {
        libc::signal(libc::SIGTERM, handler_addr());
        libc::signal(libc::SIGINT, handler_addr());
    }
}

#[allow(function_casts_as_integer)]
fn handler_addr() -> libc::sighandler_t {
    // libc stores the handler as an integer address.
    on_stop as libc::sighandler_t
}

extern "C" fn on_stop(_signal: i32) {
    STOP.store(true, Ordering::SeqCst);
}

fn teardown_command(args: &[String]) -> i32 {
    let mut offline = false;
    let mut base = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--offline" => offline = true,
            "--base" => {
                let Some(value) = rest.next().filter(|value| !value.starts_with('-')) else {
                    return 2;
                };
                base = Some(PathBuf::from(value));
            }
            _ => return 2,
        }
    }
    if offline {
        return teardown_offline(base);
    }
    if base.is_some() {
        return 2;
    }
    match super::client::call(&super::client::socket_path(), super::protocol::Op::Teardown) {
        Ok(reply) if reply.ok => match reply.data {
            Some(super::protocol::ReplyData::Teardown {
                cores,
                nets,
                host,
                errors,
            }) => {
                print_teardown(&super::teardown::TeardownReport {
                    cores,
                    nets,
                    host,
                    errors,
                });
                0
            }
            _ => 3,
        },
        Ok(reply) => {
            eprintln!("[{}]", reply.code);
            3
        }
        Err(_) => 4,
    }
}

fn teardown_offline(base: Option<PathBuf>) -> i32 {
    let base = match super::boot::command_base(base) {
        Ok(base) => base,
        Err(code) => return code,
    };
    if !test_mode()
        && matches!(
            crate::common::unit_state("cm-controller.service").as_str(),
            "active" | "activating"
        )
    {
        eprintln!("[controller_running]");
        return 3;
    }
    let report = offline_teardown(&base);
    print_teardown(&report);
    0
}

fn offline_teardown(base: &Path) -> super::teardown::TeardownReport {
    let lease_dir = base.join("leases");
    let workers = Arc::new(super::core_ops::Workers::new(Arc::new(
        super::core_ops::MihomoFactory {
            binary: core_binary(),
            lease_dir: lease_dir.clone(),
        },
    )));
    let nets = Arc::new(super::net_ops::NetCtx {
        exec: Arc::new(Mutex::new(crate::net::SystemExec)),
        query: Arc::new(Mutex::new(crate::net::query::SystemQuery)),
        lease_dir,
        etc_root: if test_mode() {
            base.join("etc-netns")
        } else {
            PathBuf::from("/etc/netns")
        },
    });
    workers.set_net(Arc::clone(&nets));
    super::teardown::teardown_all(base, &workers, &nets)
}

fn print_teardown(report: &super::teardown::TeardownReport) {
    if let Ok(text) = serde_json::to_string(report) {
        println!("{text}");
    }
}

/// Кадр снятия, если сокет контроллера есть. Неудача не мешает удалению пакета.
pub fn uninstall_controller_frame() {
    let socket = super::client::socket_path();
    let Ok(meta) = fs::symlink_metadata(&socket) else {
        return;
    };
    if !meta.file_type().is_socket() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = std::process::Command::new(exe)
        .args(["controller", "teardown"])
        .status();
}

/// То же, что `cm controller teardown --offline`, уже после остановки служб.
pub fn uninstall_teardown_offline() {
    let base = if test_mode() {
        std::env::var("CM_CONTROLLER_BASE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(super::owner::SYSTEM_BASE))
    } else {
        PathBuf::from(super::owner::SYSTEM_BASE)
    };
    let _ = offline_teardown(&base);
}

fn usage() {
    eprintln!(
        "cm controller serve --socket PATH [--base DIR]\ncm controller teardown [--offline] [--base DIR]"
    );
}

struct ConnectionGuard;

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}
