//! Подготовка дочернего процесса: дескрипторы, пределы, запрет новых привилегий.
//! В `pre_exec` только async-signal-safe вызовы и уже выделенная память.

use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::Command;

#[derive(Clone, Copy, Debug)]
pub struct ChildLimits {
    pub nofile: u64,
    pub core: u64,
}

impl Default for ChildLimits {
    fn default() -> Self {
        Self {
            nofile: 4096,
            core: 0,
        }
    }
}

pub fn harden_pre_exec(command: &mut Command, keep_fds: &[RawFd], limits: ChildLimits) {
    let keep = sorted_keep(keep_fds);
    // SAFETY: the closure runs in the forked child before exec and only calls async-signal-safe syscalls.
    unsafe {
        command.pre_exec(move || apply_child(&keep, limits));
    }
}

/// Секрет в запечатанном memfd. В argv и env он не попадает.
pub fn secret_fd(bytes: &[u8]) -> Result<OwnedFd, super::protocol::ControlError> {
    let name =
        std::ffi::CString::new("cm-secret").map_err(|_| super::protocol::ControlError::Failed)?;
    // SAFETY: memfd_create returns a new fd or -1; the name points at a live C string.
    let fd =
        unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
    if fd < 0 {
        return Err(super::protocol::ControlError::Failed);
    }
    // SAFETY: memfd_create returned a fresh fd owned by this process.
    let mut file = unsafe { File::from_raw_fd(fd) };
    if file.write_all(bytes).is_err() {
        return Err(super::protocol::ControlError::Failed);
    }
    // SAFETY: lseek on the memfd we own; SEEK_SET does not follow a user pointer.
    if unsafe { libc::lseek(file.as_raw_fd(), 0, libc::SEEK_SET) } < 0 {
        return Err(super::protocol::ControlError::Failed);
    }
    let seals = libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
    // SAFETY: F_ADD_SEALS takes an integer flag, not a pointer, on the fd we own.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, seals) } != 0 {
        return Err(super::protocol::ControlError::Failed);
    }
    Ok(file.into())
}

pub(crate) fn sorted_keep(keep_fds: &[RawFd]) -> Vec<RawFd> {
    let mut keep: Vec<RawFd> = keep_fds.iter().copied().filter(|fd| *fd >= 3).collect();
    keep.sort_unstable();
    keep.dedup();
    keep
}

pub(crate) fn apply_child(keep: &[RawFd], limits: ChildLimits) -> io::Result<()> {
    close_except(keep);
    set_limit(libc::RLIMIT_NOFILE as u32, limits.nofile)?;
    set_limit(libc::RLIMIT_CORE as u32, limits.core)?;
    // SAFETY: prctl with integer arguments; no pointer into Rust memory.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: clears the ambient set of this child only.
    unsafe { libc::prctl(libc::PR_CAP_AMBIENT_CLEAR_ALL, 0, 0, 0, 0) };
    for cap in 0..=63 {
        // SAFETY: drops one bounding-set capability in this child. EINVAL/EPERM are ignored:
        // the number may not exist, and after setuid the caller may no longer hold CAP_SETPCAP.
        let rc = unsafe { libc::prctl(libc::PR_CAPBSET_DROP, cap, 0, 0, 0) };
        if rc != 0 {
            let err = io::Error::last_os_error();
            let code = err.raw_os_error();
            if code != Some(libc::EINVAL) && code != Some(libc::EPERM) {
                return Err(err);
            }
        }
    }
    Ok(())
}

fn close_except(keep: &[RawFd]) {
    let mut cursor = 3u32;
    for fd in keep {
        let fd = *fd as u32;
        if fd > cursor {
            close_range(cursor, fd - 1);
        }
        if fd >= cursor {
            cursor = fd.saturating_add(1);
        }
    }
    close_range(cursor, u32::MAX);
}

fn close_range(first: u32, last: u32) {
    if first > last {
        return;
    }
    // SAFETY: close_range closes descriptors in this child only; flags 0 does not unshare.
    unsafe {
        libc::syscall(libc::SYS_close_range, first, last, 0u32);
    }
}

fn set_limit(resource: u32, value: u64) -> io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: value as _,
        rlim_max: value as _,
    };
    // SAFETY: setrlimit reads one rlimit on the stack and applies it to this child.
    if unsafe { libc::setrlimit(resource as _, &limit) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
