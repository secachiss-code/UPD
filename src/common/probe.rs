//! Bounded command capture. Pipe readers live in this stack frame, never in detached threads.
use std::collections::VecDeque;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Output, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

type ProbeScope = Option<(Instant, Arc<AtomicBool>, Option<CaptureError>)>;

thread_local! {
    static SCOPE: std::cell::RefCell<ProbeScope> = const { std::cell::RefCell::new(None) };
}
/// A whole read-only job shares one deadline. Legacy optional probes cannot hide a limit failure.
pub fn with_probe_scope<T>(
    timeout: Duration,
    cancel: Arc<AtomicBool>,
    job: impl FnOnce() -> T,
) -> Result<T, CaptureError> {
    struct Restore(Option<(Instant, Arc<AtomicBool>, Option<CaptureError>)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SCOPE.with(|s| *s.borrow_mut() = self.0.take());
        }
    }
    if cancel.load(Ordering::Acquire) {
        return Err(CaptureError::Cancelled("background job".into()));
    }
    let deadline = Instant::now() + timeout;
    let _restore = Restore(SCOPE.with(|s| s.replace(Some((deadline, cancel.clone(), None)))));
    let value = job();
    let error = SCOPE.with(|s| s.borrow_mut().as_mut().and_then(|(_, _, e)| e.take()));
    match error {
        Some(e) => Err(e),
        None if cancel.load(Ordering::Acquire) => {
            Err(CaptureError::Cancelled("background job".into()))
        }
        None if Instant::now() >= deadline => Err(CaptureError::Timeout("background job".into())),
        None => Ok(value),
    }
}
pub(super) fn record_probe_error(error: CaptureError) {
    if matches!(
        error,
        CaptureError::Timeout(_) | CaptureError::Cancelled(_) | CaptureError::Overflow { .. }
    ) {
        SCOPE.with(|s| {
            if let Some((_, _, e)) = s.borrow_mut().as_mut()
                && e.is_none()
            {
                *e = Some(error);
            }
        });
    }
}

#[derive(Debug)]
pub enum CaptureError {
    Timeout(String),
    Cancelled(String),
    Overflow {
        program: String,
        stream: &'static str,
        limit: u64,
    },
    Io {
        program: String,
        source: io::Error,
    },
}
impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout(p) => write!(f, "{p}: command timed out"),
            Self::Cancelled(p) => write!(f, "{p}: command cancelled"),
            Self::Overflow {
                program,
                stream,
                limit,
            } => write!(f, "{program}: {stream} exceeds {limit} bytes"),
            Self::Io { program, source } => write!(f, "{program}: {source}"),
        }
    }
}
impl std::error::Error for CaptureError {}
impl From<CaptureError> for String {
    fn from(e: CaptureError) -> Self {
        e.to_string()
    }
}

pub struct CapturePolicy {
    pub deadline: Instant,
    pub cancellation: Option<Arc<AtomicBool>>,
    pub stdout_max: Option<u64>,
    pub stderr_max: u64,
    /// Background probes own a new process group. Installation commands retain their runner's group.
    pub own_group: bool,
}
impl CapturePolicy {
    pub fn background(stdout_max: Option<u64>) -> Self {
        Self {
            deadline: Instant::now() + Duration::from_secs(30),
            cancellation: None,
            stdout_max,
            stderr_max: super::OUT_MAX,
            own_group: true,
        }
    }
    pub fn interactive(stdout_max: Option<u64>) -> Self {
        Self {
            deadline: Instant::now() + Duration::from_secs(30 * 60),
            own_group: false,
            ..Self::background(stdout_max)
        }
    }
}

struct OwnedChild {
    child: Child,
    group: Option<i32>,
    reaped: bool,
}
impl OwnedChild {
    fn stop(&mut self) {
        // The leader is still unreaped, so its PID (and thus this PGID) cannot be reused.
        if let Some(group) = self.group.take() {
            // SAFETY: sending a signal accesses no memory; the target group is a child we have not reaped yet.
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
        let _ = self.child.kill();
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.reaped {
            self.stop();
            let _ = self.child.wait();
        }
    }
}
fn nonblocking(fd: i32) -> io::Result<()> {
    // SAFETY: fcntl on an open fd borrowed for the call; F_GETFL/F_SETFL/F_GETFD do not access memory.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    // SAFETY: fcntl on an open fd borrowed for the call; F_GETFL/F_SETFL/F_GETFD do not access memory.
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn drain<R: Read>(
    pipe: &mut Option<R>,
    count: &mut u64,
    max: u64,
    mut store: impl FnMut(&[u8]),
) -> io::Result<bool> {
    let mut buf = [0; 8192];
    for _ in 0..32 {
        let Some(reader) = pipe.as_mut() else { break };
        match reader.read(&mut buf) {
            Ok(0) => {
                *pipe = None;
                break;
            }
            Ok(n) => {
                *count = count.saturating_add(n as u64);
                if *count > max {
                    return Ok(true);
                }
                store(&buf[..n]);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(false)
}

pub fn capture_with_policy(
    cmd: &mut Command,
    policy: CapturePolicy,
) -> Result<Output, CaptureError> {
    let name = cmd.get_program().to_string_lossy().into_owned();
    if policy
        .cancellation
        .as_ref()
        .is_some_and(|c| c.load(Ordering::Acquire))
    {
        return Err(CaptureError::Cancelled(name));
    }
    if Instant::now() >= policy.deadline {
        return Err(CaptureError::Timeout(name));
    }
    let io_error = |source| CaptureError::Io {
        program: name.clone(),
        source,
    };
    cmd.stdout(if policy.stdout_max.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stderr(Stdio::piped());
    if policy.own_group {
        cmd.process_group(0).stdin(Stdio::null());
    }
    let child = cmd.spawn().map_err(io_error)?;
    let mut owner = OwnedChild {
        child,
        group: None,
        reaped: false,
    };
    if policy.own_group {
        let pid = owner.child.id() as i32;
        // SAFETY: takes no pointers and accesses no memory.
        if unsafe { libc::getpgid(pid) } != pid {
            return Err(io_error(io::Error::other(
                "probe process group was not established",
            )));
        }
        owner.group = Some(pid);
    }
    let mut stdout_pipe = owner.child.stdout.take();
    let mut stderr_pipe = owner.child.stderr.take();
    for fd in [
        stdout_pipe.as_ref().map(AsRawFd::as_raw_fd),
        stderr_pipe.as_ref().map(AsRawFd::as_raw_fd),
    ]
    .into_iter()
    .flatten()
    {
        nonblocking(fd).map_err(io_error)?;
    }
    let mut stdout = Vec::new();
    let mut tail = VecDeque::new();
    let (mut out_count, mut err_count) = (0, 0);
    let mut exited_at = None;
    loop {
        if policy
            .cancellation
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Acquire))
        {
            return Err(CaptureError::Cancelled(name));
        }
        if Instant::now() >= policy.deadline {
            return Err(CaptureError::Timeout(name));
        }
        if drain(
            &mut stdout_pipe,
            &mut out_count,
            policy.stdout_max.unwrap_or(0),
            |b| stdout.extend_from_slice(b),
        )
        .map_err(io_error)?
        {
            return Err(CaptureError::Overflow {
                program: name,
                stream: "stdout",
                limit: policy.stdout_max.unwrap_or(0),
            });
        }
        if drain(&mut stderr_pipe, &mut err_count, policy.stderr_max, |b| {
            tail.extend(b);
            while tail.len() > super::ERR_TAIL {
                tail.pop_front();
            }
        })
        .map_err(io_error)?
        {
            return Err(CaptureError::Overflow {
                program: name,
                stream: "stderr",
                limit: policy.stderr_max,
            });
        }
        // SAFETY: plain C struct; the all-zero bit pattern is a valid value.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: info is a live out-pointer; WNOWAIT leaves the child for Child::wait.
        let rc = unsafe {
            libc::waitid(
                libc::P_PID,
                owner.child.id(),
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if rc < 0 {
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(io_error(e));
            }
        }
        // SAFETY: waitid filled info (or it stayed zeroed), so the SIGCHLD fields are initialized.
        if unsafe { info.si_pid() } != 0 {
            exited_at.get_or_insert_with(Instant::now);
        }
        if let Some(exited) = exited_at
            && ((stdout_pipe.is_none() && stderr_pipe.is_none())
                || exited.elapsed() >= Duration::from_millis(500))
        {
            owner.stop();
            let status = owner.child.wait().map_err(io_error)?;
            owner.reaped = true;
            return Ok(Output {
                status,
                stdout,
                stderr: tail.into_iter().collect(),
            });
        }
        let mut fds: Vec<_> = [
            stdout_pipe.as_ref().map(AsRawFd::as_raw_fd),
            stderr_pipe.as_ref().map(AsRawFd::as_raw_fd),
        ]
        .into_iter()
        .flatten()
        .map(|fd| libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
        // Once the leader exits, descendants may hold pipes; the bounded drain above closes them.
        // SAFETY: the pollfd pointer and count describe live entries for the whole call.
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, 20) };
        if rc < 0 {
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(io_error(e));
            }
        }
    }
}

pub fn capture(cmd: &mut Command, stdout_max: Option<u64>) -> Result<Output, CaptureError> {
    let mut policy = CapturePolicy::background(stdout_max);
    SCOPE.with(|s| {
        if let Some((deadline, cancel, _)) = s.borrow().as_ref() {
            policy.deadline = policy.deadline.min(*deadline);
            policy.cancellation = Some(cancel.clone());
        }
    });
    capture_with_policy(cmd, policy)
}
pub fn capture_interactive(
    cmd: &mut Command,
    stdout_max: Option<u64>,
) -> Result<Output, CaptureError> {
    capture_with_policy(cmd, CapturePolicy::interactive(stdout_max))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn shell(script: &str, timeout: Duration, out: u64, err: u64) -> Result<Output, CaptureError> {
        let mut p = CapturePolicy::background(Some(out));
        p.deadline = Instant::now() + timeout;
        p.stderr_max = err;
        capture_with_policy(Command::new("sh").args(["-c", script]), p)
    }
    #[test]
    fn sleeping_probe_times_out_and_preserves_caller_group() {
        // SAFETY: takes no pointers and accesses no memory.
        let group = unsafe { libc::getpgrp() };
        let start = Instant::now();
        assert!(matches!(
            shell("sleep 30", Duration::from_millis(80), 100, 100),
            Err(CaptureError::Timeout(_))
        ));
        assert!(start.elapsed() < Duration::from_secs(2));
        // SAFETY: takes no pointers and accesses no memory.
        assert_eq!(unsafe { libc::getpgrp() }, group);
    }
    #[test]
    fn both_streams_have_typed_limits() {
        assert!(matches!(
            shell("yes", Duration::from_secs(2), 1000, 1000),
            Err(CaptureError::Overflow {
                stream: "stdout",
                ..
            })
        ));
        assert!(matches!(
            shell("yes >&2", Duration::from_secs(2), 1000, 1000),
            Err(CaptureError::Overflow {
                stream: "stderr",
                ..
            })
        ));
        let output = shell(
            "head -c 200000 /dev/zero >&2; printf abc",
            Duration::from_secs(2),
            3,
            200000,
        )
        .unwrap();
        assert_eq!(output.stdout, b"abc");
        assert_eq!(output.stderr.len(), super::super::ERR_TAIL);
    }
    #[test]
    fn exited_leader_cannot_leave_background_work() {
        let temp = super::super::contract_fixtures::TempDirGuard::new("probe-descendant").unwrap();
        let marker = temp.path().join("survived");
        let script = format!("(sleep 2; touch '{}') & printf done", marker.display());
        let start = Instant::now();
        let output = shell(&script, Duration::from_secs(4), 1000, 1000).unwrap();
        assert_eq!(output.stdout, b"done");
        assert!(start.elapsed() < Duration::from_secs(1));
        std::thread::sleep(Duration::from_millis(2100));
        assert!(!marker.exists());
    }
    #[test]
    fn cancellation_and_scope_failure_are_visible() {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker = std::thread::spawn(move || {
            with_probe_scope(Duration::from_secs(3), worker_cancel, || {
                super::super::out("sh", &["-c", "sleep 30"])
            })
        });
        std::thread::sleep(Duration::from_millis(50));
        cancel.store(true, Ordering::Release);
        assert!(matches!(
            worker.join().unwrap(),
            Err(CaptureError::Cancelled(_))
        ));
    }
    #[test]
    fn setup_failure_drops_owned_child_and_group() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30"]).process_group(0);
        let child = command.spawn().unwrap();
        let pid = child.id() as i32;
        let owner = OwnedChild {
            child,
            group: Some(pid),
            reaped: false,
        };
        assert!(nonblocking(-1).is_err());
        drop(owner);
        // SAFETY: signal 0 only checks whether the pid exists; no memory is accessed.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    }
}
