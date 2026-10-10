//! Запуск браузера строго по плану.

use super::engine::{self, Family};
use super::guard::{self, GuardError};
use super::model::validate_id;
use super::plan::{self, LaunchPlan};
use super::store::{IdentityStore, StoreError};
use super::tzdata::SYSTEM_TZDIR;
use super::validate::{self, Violation};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

#[derive(Debug, PartialEq, Eq)]
pub enum LaunchError {
    RunAsRoot,
    AlreadyRunning,
    Store(StoreError),
    Engine(engine::EngineError),
    Invalid(Vec<Violation>),
    Guard(GuardError),
    Spawn,
}

impl LaunchError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::RunAsRoot => "RunAsRoot",
            Self::AlreadyRunning => "AlreadyRunning",
            Self::Store(error) => error.code(),
            Self::Engine(error) => error.code(),
            Self::Invalid(items) => items.first().map(Violation::code).unwrap_or("Invalid"),
            Self::Guard(error) => error.code(),
            Self::Spawn => "Spawn",
        }
    }
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunAsRoot => f.write_str(t!("запуск личности от root запрещён")),
            Self::AlreadyRunning => f.write_str(t!("личность уже запущена")),
            Self::Store(error) => write!(f, "{error}"),
            Self::Engine(error) => write!(f, "{error}"),
            Self::Invalid(items) => match items.first() {
                Some(item) => write!(f, "{item}"),
                None => f.write_str(t!("личность не согласована")),
            },
            Self::Guard(error) => write!(f, "{error}"),
            Self::Spawn => f.write_str(t!("не удалось запустить браузер")),
        }
    }
}

pub fn launch(
    store: &IdentityStore,
    id: &str,
    url: Option<&str>,
    lab: bool,
) -> Result<i32, LaunchError> {
    if crate::common::sys::euid() == 0 && !crate::common::test_mode() {
        return Err(LaunchError::RunAsRoot);
    }
    validate_id(id).map_err(|_| LaunchError::Store(StoreError::BadId))?;
    let dir = store.dir(id);
    if !dir.is_dir() {
        return Err(LaunchError::Store(StoreError::NotFound));
    }
    let _lock = lock_identity(&dir)?;
    let mut profile = store.load(id)?;
    let detected = engine::detect(&profile.browser)?;
    if detected != profile.engine {
        // Only another brand or major is a change of the identity. A patch release
        // updates the snapshot without spending a generation and a history slot.
        let identity_changed =
            detected.brand != profile.engine.brand || detected.major != profile.engine.major;
        let note = format!("engine {} {}", detected.brand.as_str(), detected.major);
        profile.engine = detected;
        if identity_changed {
            profile.record(crate::common::now(), &note);
        }
        store.save(&profile)?;
    }
    let report = validate::validate(&profile, &profile.engine, Path::new(SYSTEM_TZDIR), lab);
    if !report.errors.is_empty() {
        return Err(LaunchError::Invalid(report.errors));
    }
    let family = profile.engine.family;
    guard::check_before_launch(
        store,
        &mut profile,
        family,
        guard::bypass_policy(),
        &guard::pid_alive,
    )?;
    let profile_dir = store.profile_dir(id);
    let parent = std::env::vars().collect::<BTreeMap<_, _>>();
    let plan = match family {
        Family::Chromium => plan::chromium_plan(&profile, &profile_dir, &parent, url, lab),
        Family::Gecko => plan::gecko_plan(&profile, &profile_dir, &parent, url, lab),
    };
    for file in &plan.files {
        super::store::write_private(&file.path, file.contents.as_bytes())?;
    }
    let generation = profile.generation;
    spawn_and_wait(&plan, &dir, generation, &profile_dir, family)
}

fn spawn_and_wait(
    plan: &LaunchPlan,
    dir: &Path,
    generation: u32,
    profile_dir: &Path,
    family: Family,
) -> Result<i32, LaunchError> {
    let log_path = dir.join("browser.log");
    let log = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
        .map_err(|_| LaunchError::Spawn)?;
    fs::set_permissions(&log_path, fs::Permissions::from_mode(0o600))
        .map_err(|_| LaunchError::Spawn)?;
    let stderr = log.try_clone().map_err(|_| LaunchError::Spawn)?;
    let mut command = Command::new(&plan.program);
    command.args(&plan.args).env_clear();
    for (key, value) in &plan.env {
        command.env(key, value);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));
    // SAFETY: pre_exec runs only in the forked child before exec. setsid takes no pointers and only creates a new session for that child.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let signals = StopSignals::install();
    let mut child = command.spawn().map_err(|_| LaunchError::Spawn)?;
    let pid = child.id();
    let started = crate::common::now();
    if let Err(error) = guard::record_launch(dir, generation, pid, started) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(LaunchError::Store(error));
    }
    // Ctrl+C, a closed terminal or a session end must not leave the browser running
    // without CM: the next launch would read that as a bypass. The stop is passed on
    // to the browser's session, and the exit mark is written after it has finished.
    let mut forwarded = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(_) => return Err(LaunchError::Spawn),
        }
        if signals.requested() && !forwarded {
            forwarded = true;
            // SAFETY: kill takes no pointers. The negative pid addresses the session
            // that setsid created for this child, so only the browser is signalled.
            unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGTERM) };
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    drop(signals);
    guard::record_exit(dir, profile_dir, family, crate::common::now())?;
    Ok(exit_code(status))
}

impl From<StoreError> for LaunchError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<engine::EngineError> for LaunchError {
    fn from(error: engine::EngineError) -> Self {
        Self::Engine(error)
    }
}

impl From<GuardError> for LaunchError {
    fn from(error: GuardError) -> Self {
        Self::Guard(error)
    }
}

fn exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    128 + status.signal().unwrap_or(0)
}

static STOP_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_stop_signal(_signal: libc::c_int) {
    STOP_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
}

const STOP_SIGNALS: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

/// Handlers for the time the browser runs; the previous ones return on drop.
struct StopSignals {
    previous: [libc::sighandler_t; 3],
}

impl StopSignals {
    fn install() -> Self {
        STOP_REQUESTED.store(false, std::sync::atomic::Ordering::SeqCst);
        let handler = on_stop_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        let mut previous = [libc::SIG_DFL; 3];
        for (slot, signal) in previous.iter_mut().zip(STOP_SIGNALS) {
            // SAFETY: the handler only stores into an atomic, which is async-signal-safe.
            *slot = unsafe { libc::signal(signal, handler) };
        }
        Self { previous }
    }

    fn requested(&self) -> bool {
        STOP_REQUESTED.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Drop for StopSignals {
    fn drop(&mut self) {
        for (previous, signal) in self.previous.iter().zip(STOP_SIGNALS) {
            // SAFETY: restores the handler that was in place before install.
            unsafe { libc::signal(signal, *previous) };
        }
    }
}

struct RunLock {
    file: std::fs::File,
}

impl Drop for RunLock {
    fn drop(&mut self) {
        // Closing the fd releases the flock.
        let _ = &self.file;
    }
}

fn lock_identity(dir: &Path) -> Result<RunLock, LaunchError> {
    use std::os::fd::AsRawFd;
    let path = dir.join("run.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|_| LaunchError::Store(StoreError::Io))?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .map_err(|_| LaunchError::Store(StoreError::Io))?;
    // SAFETY: flock on an open fd borrowed for the call; it accesses no memory.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EWOULDBLOCK)
            || error.raw_os_error() == Some(libc::EAGAIN)
        {
            return Err(LaunchError::AlreadyRunning);
        }
        return Err(LaunchError::Store(StoreError::Io));
    }
    Ok(RunLock { file })
}
