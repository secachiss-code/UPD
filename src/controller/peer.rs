//! Личность процесса на другом конце сокета. uid берётся только из `SO_PEERCRED`.

use std::fs;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;

use super::protocol::ControlError;

pub struct PeerIdentity {
    pub uid: u32,
    pub gid: u32,
    pub pid: i32,
    /// Поле 22 `/proc/<pid>/stat`.
    pub start_time: u64,
    /// Путь из строки `0::` файла `/proc/<pid>/cgroup`.
    pub cgroup: String,
    /// `N` из сегмента `session-N.scope`.
    pub session: Option<String>,
    /// `st_ino` файла `/proc/<pid>/ns/net`.
    pub netns_inode: u64,
    pidfd: OwnedFd,
}

pub fn capture(stream: &UnixStream) -> Result<PeerIdentity, ControlError> {
    let cred = peercred(stream)?;
    if cred.pid <= 0 {
        return Err(ControlError::PeerChanged);
    }
    let pidfd = pidfd_open(cred.pid)?;
    let identity = read_identity(cred, pidfd)?;
    if !identity.alive() {
        return Err(ControlError::PeerChanged);
    }
    Ok(identity)
}

impl PeerIdentity {
    /// `POLLIN` на pidfd означает, что процесс уже завершился.
    pub fn alive(&self) -> bool {
        let mut pollfd = libc::pollfd {
            fd: self.pidfd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        loop {
            // SAFETY: one live pollfd; timeout 0 does not block and does not follow a pointer into Rust memory.
            let rc = unsafe { libc::poll(&mut pollfd, 1, 0) };
            if rc < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return false;
            }
            return rc == 0 || (pollfd.revents & libc::POLLIN) == 0;
        }
    }

    pub fn verify(&self) -> Result<(), ControlError> {
        if !self.alive() {
            return Err(ControlError::PeerChanged);
        }
        let stat = fs::read_to_string(format!("/proc/{}/stat", self.pid))
            .map_err(|_| ControlError::PeerChanged)?;
        match parse_start_time(&stat) {
            Some(start) if start == self.start_time => Ok(()),
            _ => Err(ControlError::PeerChanged),
        }
    }
}

impl std::fmt::Debug for PeerIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerIdentity")
            .field("uid", &self.uid)
            .field("pid", &self.pid)
            .field("session", &self.session)
            .finish()
    }
}

/// Поле 22 `/proc/pid/stat`: двадцатое поле после последней `)`.
pub fn parse_start_time(stat: &str) -> Option<u64> {
    let tail = stat.rsplit_once(')')?.1;
    tail.split_whitespace().nth(19)?.parse().ok()
}

pub fn parse_cgroup(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("0::") {
            if path.is_empty() {
                return None;
            }
            return Some(path.to_owned());
        }
    }
    None
}

pub fn session_of(cgroup: &str) -> Option<String> {
    for segment in cgroup.split('/') {
        let Some(rest) = segment.strip_prefix("session-") else {
            continue;
        };
        let Some(id) = rest.strip_suffix(".scope") else {
            continue;
        };
        if !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()) {
            return Some(id.to_owned());
        }
    }
    None
}

struct Cred {
    uid: u32,
    gid: u32,
    pid: i32,
}

fn peercred(stream: &UnixStream) -> Result<Cred, ControlError> {
    let (uid, gid, pid) =
        crate::core::mihomo::api::peer_cred(stream).ok_or(ControlError::PeerChanged)?;
    Ok(Cred { uid, gid, pid })
}

fn pidfd_open(pid: i32) -> Result<OwnedFd, ControlError> {
    // SAFETY: pidfd_open returns a new file descriptor or -1; no pointer is followed.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0u32) };
    if fd < 0 {
        return Err(ControlError::PeerChanged);
    }
    // SAFETY: the syscall returned a fresh fd that this process owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
}

fn read_identity(cred: Cred, pidfd: OwnedFd) -> Result<PeerIdentity, ControlError> {
    let pid = cred.pid;
    let stat =
        fs::read_to_string(format!("/proc/{pid}/stat")).map_err(|_| ControlError::PeerChanged)?;
    let start_time = parse_start_time(&stat).ok_or(ControlError::PeerChanged)?;
    let cgroup_text = fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap_or_default();
    let cgroup = parse_cgroup(&cgroup_text).unwrap_or_default();
    let session = session_of(&cgroup);
    let netns =
        fs::metadata(format!("/proc/{pid}/ns/net")).map_err(|_| ControlError::PeerChanged)?;
    Ok(PeerIdentity {
        uid: cred.uid,
        gid: cred.gid,
        pid,
        start_time,
        cgroup,
        session,
        netns_inode: netns.ino(),
        pidfd,
    })
}
