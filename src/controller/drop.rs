//! Сброс привилегий до uid вызывающего. Группы считаются в родителе, до fork.

use std::collections::BTreeMap;
use std::ffi::CString;
use std::os::fd::RawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use super::harden::{self, ChildLimits};
use super::protocol::ControlError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunAs {
    pub uid: u32,
    pub gid: u32,
    pub groups: Vec<u32>,
}

pub fn run_as_for(uid: u32, gid: u32) -> Result<RunAs, ControlError> {
    let name = user_name(uid).ok_or(ControlError::Failed)?;
    let c_name = CString::new(name).map_err(|_| ControlError::Failed)?;
    let mut count = 64i32;
    let mut groups = vec![0 as libc::gid_t; count as usize];
    // SAFETY: getgrouplist writes into `groups` and `count`; the name is a live C string.
    let mut rc =
        unsafe { libc::getgrouplist(c_name.as_ptr(), gid, groups.as_mut_ptr(), &mut count) };
    if rc == -1 {
        if count <= 0 || count > 4096 {
            return Err(ControlError::Failed);
        }
        groups.resize(count as usize, 0);
        // SAFETY: the buffer now has the size the previous call requested.
        rc = unsafe { libc::getgrouplist(c_name.as_ptr(), gid, groups.as_mut_ptr(), &mut count) };
        if rc == -1 {
            return Err(ControlError::Failed);
        }
    }
    groups.truncate(count.max(0) as usize);
    Ok(RunAs { uid, gid, groups })
}

pub fn drop_pre_exec(
    command: &mut Command,
    run_as: RunAs,
    keep_fds: &[RawFd],
    limits: ChildLimits,
) {
    install(command, run_as, keep_fds, limits, false, None);
}

/// `setsid`, затем те же шаги, что у [`drop_pre_exec`]. Один `pre_exec`: std не гарантирует порядок нескольких.
pub fn worker_pre_exec(
    command: &mut Command,
    run_as: RunAs,
    keep_fds: &[RawFd],
    limits: ChildLimits,
) {
    install(command, run_as, keep_fds, limits, true, None);
}

pub fn spawn_as(
    run_as: RunAs,
    program: &Path,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> Result<Child, ControlError> {
    if !program.is_absolute() {
        return Err(ControlError::BadArgument);
    }
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        if key.is_empty() || key.contains('=') || key.contains('\0') || value.contains('\0') {
            return Err(ControlError::BadArgument);
        }
        command.env(key, value);
    }
    drop_pre_exec(&mut command, run_as, &[], ChildLimits::default());
    command.spawn().map_err(|_| ControlError::Failed)
}

/// Вход в netns и только потом сброс привилегий. С `resolv` потомок получает свой mount
/// namespace, где этот файл закрывает `/etc/resolv.conf`: одного `setns` мало, приложение
/// читало бы DNS хоста. Путь с NUL внутри — отказ до запуска.
pub fn drop_into_netns_pre_exec(
    command: &mut Command,
    netns: RawFd,
    resolv: Option<&Path>,
    run_as: RunAs,
    keep_fds: &[RawFd],
    limits: ChildLimits,
) -> Result<(), ControlError> {
    let resolv = match resolv {
        Some(path) => Some(
            CString::new(std::os::unix::ffi::OsStrExt::as_bytes(path.as_os_str()))
                .map_err(|_| ControlError::BadArgument)?,
        ),
        None => None,
    };
    install(
        command,
        run_as,
        keep_fds,
        limits,
        false,
        Some(Netns { fd: netns, resolv }),
    );
    Ok(())
}

struct Netns {
    fd: RawFd,
    resolv: Option<CString>,
}

/// Только системные вызовы: выполняется в потомке между fork и exec.
///
/// # Safety
/// Вызывать только в `pre_exec`; `netns.fd` — открытый дескриптор сетевого пространства имён.
unsafe fn enter_netns(netns: &Netns) -> std::io::Result<()> {
    // SAFETY: setns takes an fd and a flag; the fd stays open in the parent until spawn returns.
    if unsafe { libc::setns(netns.fd, libc::CLONE_NEWNET) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let Some(resolv) = &netns.resolv else {
        return Ok(());
    };
    // SAFETY: unshare takes a flag only and affects this child.
    if unsafe { libc::unshare(libc::CLONE_NEWNS) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the path literals are NUL-terminated; null source, type and data are valid for a
    // propagation change. Slave keeps the bind below from reaching the host mount table.
    let slave = unsafe {
        libc::mount(
            std::ptr::null(),
            c"/".as_ptr(),
            std::ptr::null(),
            libc::MS_REC | libc::MS_SLAVE,
            std::ptr::null(),
        )
    };
    if slave != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: both paths are live NUL-terminated strings allocated before fork.
    let bind = unsafe {
        libc::mount(
            resolv.as_ptr(),
            c"/etc/resolv.conf".as_ptr(),
            std::ptr::null(),
            libc::MS_BIND,
            std::ptr::null(),
        )
    };
    if bind != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn install(
    command: &mut Command,
    run_as: RunAs,
    keep_fds: &[RawFd],
    limits: ChildLimits,
    new_session: bool,
    netns: Option<Netns>,
) {
    let keep = harden::sorted_keep(keep_fds);
    let groups: Vec<libc::gid_t> = run_as
        .groups
        .iter()
        .map(|gid| *gid as libc::gid_t)
        .collect();
    let uid = run_as.uid;
    let gid = run_as.gid;
    // SAFETY: runs in the forked child before exec. set*id, setsid and the harden syscalls are async-signal-safe.
    // The group list was allocated in the parent; the child only reads it.
    unsafe {
        command.pre_exec(move || {
            if let Some(netns) = &netns {
                enter_netns(netns)?;
            }
            if new_session && libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::setgroups(groups.len(), groups.as_ptr()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::setresgid(gid, gid, gid) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::setresuid(uid, uid, uid) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if uid != 0 && libc::setresuid(0, 0, 0) == 0 {
                libc::_exit(126);
            }
            harden::apply_child(&keep, limits)
        });
    }
}

fn user_name(uid: u32) -> Option<String> {
    // SAFETY: passwd is a plain C struct; the all-zero pattern is valid.
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0u8; 16 * 1024];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: passwd, buffer and result outlive the call; pw_name points into `buf` on success.
    let rc = unsafe {
        libc::getpwuid_r(
            uid,
            &mut pw,
            buf.as_mut_ptr().cast(),
            buf.len(),
            &mut result,
        )
    };
    if rc != 0 || result.is_null() {
        return None;
    }
    // SAFETY: getpwuid_r succeeded, so pw_name is a NUL-terminated string inside `buf`.
    let name = unsafe { std::ffi::CStr::from_ptr(pw.pw_name) };
    Some(name.to_string_lossy().into_owned())
}
