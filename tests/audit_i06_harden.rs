//! C08: a child inherits no descriptors, no capabilities and no way to get them back.
//! C09 (L1 part): a relative program is refused before anything is spawned.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::path::Path;
use std::process::{Command, Stdio};

use cm::controller::drop::{RunAs, spawn_as};
use cm::controller::harden::{ChildLimits, harden_pre_exec, secret_fd};
use cm::controller::protocol::ControlError;

fn inherited() -> File {
    let file = File::open("/proc/self/stat").unwrap();
    // SAFETY: fcntl clears FD_CLOEXEC on a descriptor this test owns.
    unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, 0) };
    file
}

fn run(script: &str, keep: &[i32]) -> String {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", script]).stdin(Stdio::null());
    harden_pre_exec(&mut command, keep, ChildLimits::default());
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn c08_descriptors_do_not_leak() {
    let leak = inherited();
    let kept = inherited();
    let list = "for f in /proc/self/fd/*; do echo ${f##*/}; done";
    let fds: Vec<i32> = run(list, &[])
        .lines()
        .filter_map(|line| line.parse().ok())
        .collect();
    // The shell reads the directory through one descriptor of its own.
    assert!(fds.iter().filter(|fd| **fd >= 3).count() <= 1, "{fds:?}");
    let probe = format!(
        "[ -e /proc/self/fd/{} ] && echo kept; [ -e /proc/self/fd/{} ] && echo leaked; true",
        kept.as_raw_fd(),
        leak.as_raw_fd()
    );
    assert_eq!(run(&probe, &[kept.as_raw_fd()]).trim(), "kept");
}

#[test]
fn c08_no_privileges_and_limits() {
    let status = run(
        "cat /proc/self/status; echo nofile=$(ulimit -n); echo core=$(ulimit -c)",
        &[],
    );
    for line in [
        "NoNewPrivs:\t1",
        "CapAmb:\t0000000000000000",
        "nofile=4096",
        "core=0",
    ] {
        assert!(
            status.lines().any(|candidate| candidate == line),
            "{line}\n{status}"
        );
    }
    // Without CAP_SETPCAP the bounding set cannot be dropped; with it, it must be empty.
    let bounding = status
        .lines()
        .find_map(|line| line.strip_prefix("CapBnd:\t"))
        .unwrap();
    let effective = status
        .lines()
        .find_map(|line| line.strip_prefix("CapEff:\t"))
        .unwrap();
    assert_eq!(effective, "0000000000000000");
    if cm::common::sys::euid() == 0 {
        assert_eq!(bounding, "0000000000000000");
    }
}

#[test]
fn c08_secret_fd_is_sealed() {
    let fd = secret_fd(b"secret-marker").unwrap();
    // SAFETY: plain fcntl queries on a descriptor this test owns.
    let (flags, seals) = unsafe {
        (
            libc::fcntl(fd.as_raw_fd(), libc::F_GETFD),
            libc::fcntl(fd.as_raw_fd(), libc::F_GET_SEALS),
        )
    };
    assert_ne!(flags & libc::FD_CLOEXEC, 0);
    for seal in [
        libc::F_SEAL_WRITE,
        libc::F_SEAL_GROW,
        libc::F_SEAL_SHRINK,
        libc::F_SEAL_SEAL,
    ] {
        assert_ne!(seals & seal, 0);
    }
    // SAFETY: the raw descriptor is moved into the File exactly once.
    let mut file = unsafe { File::from_raw_fd(fd.into_raw_fd()) };
    let mut text = String::new();
    file.read_to_string(&mut text).unwrap();
    assert_eq!(text, "secret-marker");
    file.seek(SeekFrom::Start(0)).unwrap();
    let error = file.write_all(b"x").unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::EPERM));
}

#[test]
fn c09_relative_program_is_refused() {
    let run_as = RunAs {
        uid: 1,
        gid: 1,
        groups: vec![1],
    };
    let result = spawn_as(run_as, Path::new("id"), &[], &BTreeMap::new());
    assert_eq!(result.err(), Some(ControlError::BadArgument));
}
