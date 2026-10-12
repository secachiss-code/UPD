//! Процессы туннеля определяются сетью, а не тем, кто их запустил.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: u32,
    pub uid: u32,
    pub exe: Option<PathBuf>,
    pub netns: Option<u64>,
}

pub fn netns_id(link: &str) -> Option<u64> {
    let inner = link.strip_prefix("net:[")?.strip_suffix(']')?;
    if inner.is_empty() || !inner.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    inner.parse().ok()
}

pub fn netns_of_path(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|meta| meta.ino())
}

pub fn scan(proc_root: &Path) -> Vec<ProcInfo> {
    let Ok(entries) = fs::read_dir(proc_root) else {
        return Vec::new();
    };
    let mut procs = Vec::new();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let exe = fs::read_link(path.join("exe")).ok();
        let netns = fs::read_link(path.join("ns/net"))
            .ok()
            .and_then(|target| netns_id(&target.to_string_lossy()));
        procs.push(ProcInfo {
            pid,
            uid: meta.uid(),
            exe,
            netns,
        });
    }
    procs.sort_by_key(|proc| proc.pid);
    procs
}

pub fn holders(procs: &[ProcInfo], netns: u64, uid: u32) -> Vec<u32> {
    let mut pids: Vec<u32> = procs
        .iter()
        .filter(|proc| proc.uid == uid && proc.netns == Some(netns))
        .map(|proc| proc.pid)
        .collect();
    pids.sort_unstable();
    pids
}

pub fn elsewhere(procs: &[ProcInfo], uid: u32, exe: &Path, netns: u64) -> Vec<u32> {
    let mut pids: Vec<u32> = procs
        .iter()
        .filter(|proc| {
            proc.uid == uid
                && proc.exe.as_deref() == Some(exe)
                && matches!(proc.netns, Some(id) if id != netns)
        })
        .map(|proc| proc.pid)
        .collect();
    pids.sort_unstable();
    pids
}
