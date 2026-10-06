//! Instance resource leases with a revisioned file and an exclusive lock.
//!
//! The profile graph is not this file. A crash while allocating leaves the
//! previous revision in place; `reclaim_absent` drops holders that are gone.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use super::instance::InstanceId;

const FILE_MODE: u32 = 0o600;

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Socket,
    Port,
    Fwmark,
    RouteTable,
    TunName,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Lease {
    pub revision: u64,
    pub holder: String,
    pub kind: ResourceKind,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
struct Document {
    revision: u64,
    leases: Vec<Record>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
struct Record {
    holder: String,
    kind: ResourceKind,
    value: String,
}

#[derive(Clone, Debug)]
pub struct LeaseRegistry {
    path: PathBuf,
    lock_path: PathBuf,
}

impl LeaseRegistry {
    pub fn create(dir: &Path) -> Result<Self, LeaseError> {
        let path = dir.join("leases.json");
        let lock_path = dir.join("leases.lock");
        create_private(&lock_path)?;
        let mut file = create_private(&path)?;
        let document = Document {
            revision: 0,
            leases: Vec::new(),
        };
        write_document(&mut file, &document)?;
        file.sync_all().map_err(|_| LeaseError::Io)?;
        Ok(Self { path, lock_path })
    }

    pub fn open(dir: &Path) -> Result<Self, LeaseError> {
        let path = dir.join("leases.json");
        let lock_path = dir.join("leases.lock");
        if !path.is_file() || !lock_path.is_file() {
            return Err(LeaseError::Io);
        }
        Ok(Self { path, lock_path })
    }

    pub fn revision(&self) -> Result<u64, LeaseError> {
        self.with_document(|document| Ok(document.revision))
    }

    pub fn allocate(&self, holder: &str, kind: ResourceKind) -> Result<Lease, LeaseError> {
        let holder = holder_name(holder)?;
        self.with_document_mut(|document| {
            let value = next_value(document, kind)?;
            document.leases.push(Record {
                holder: holder.clone(),
                kind,
                value: value.clone(),
            });
            Ok(Lease {
                revision: document.revision.saturating_add(1),
                holder,
                kind,
                value,
            })
        })
    }

    pub fn release(&self, holder: &str, kind: ResourceKind, value: &str) -> Result<(), LeaseError> {
        let holder = holder_name(holder)?;
        self.with_document_mut(|document| {
            let before = document.leases.len();
            document.leases.retain(|lease| {
                !(lease.holder == holder && lease.kind == kind && lease.value == value)
            });
            if document.leases.len() == before {
                return Err(LeaseError::Conflict);
            }
            Ok(())
        })
    }

    /// Drop leases whose holder is not in `live`. Returns how many were dropped.
    pub fn reclaim_absent(&self, live: &[&str]) -> Result<usize, LeaseError> {
        for holder in live {
            holder_name(holder)?;
        }
        self.with_document_mut(|document| {
            let before = document.leases.len();
            document
                .leases
                .retain(|lease| live.iter().any(|holder| *holder == lease.holder));
            Ok(before - document.leases.len())
        })
    }

    fn with_document<T>(
        &self,
        body: impl FnOnce(&Document) -> Result<T, LeaseError>,
    ) -> Result<T, LeaseError> {
        let lock = self.hold_lock()?;
        let mut file = OpenOptions::new()
            .read(true)
            .open(&self.path)
            .map_err(|_| LeaseError::Io)?;
        let document = read_document(&mut file)?;
        let result = body(&document);
        drop(lock);
        result
    }

    fn with_document_mut<T>(
        &self,
        body: impl FnOnce(&mut Document) -> Result<T, LeaseError>,
    ) -> Result<T, LeaseError> {
        let lock = self.hold_lock()?;
        let mut file = OpenOptions::new()
            .read(true)
            .open(&self.path)
            .map_err(|_| LeaseError::Io)?;
        let mut document = read_document(&mut file)?;
        drop(file);
        let result = body(&mut document);
        let published = if result.is_ok() {
            document.revision = document
                .revision
                .checked_add(1)
                .ok_or(LeaseError::Corrupt)?;
            publish(&self.path, &document)
        } else {
            Ok(())
        };
        drop(lock);
        published?;
        result
    }

    fn hold_lock(&self) -> Result<File, LeaseError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.lock_path)
            .map_err(|_| LeaseError::Io)?;
        lock_exclusive(&file)?;
        Ok(file)
    }
}

fn create_private(path: &Path) -> Result<File, LeaseError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(path)
        .map_err(|_| LeaseError::Io)?;
    let mut permissions = file.metadata().map_err(|_| LeaseError::Io)?.permissions();
    permissions.set_mode(FILE_MODE);
    file.set_permissions(permissions)
        .map_err(|_| LeaseError::Io)?;
    Ok(file)
}

fn holder_name(holder: &str) -> Result<String, LeaseError> {
    InstanceId::new(holder)
        .map(|id| id.as_str().to_owned())
        .map_err(|_| LeaseError::InvalidHolder)
}

fn next_value(document: &Document, kind: ResourceKind) -> Result<String, LeaseError> {
    let used: Vec<&str> = document
        .leases
        .iter()
        .filter(|lease| lease.kind == kind)
        .map(|lease| lease.value.as_str())
        .collect();
    let candidate = match kind {
        ResourceKind::Port => (0..256)
            .map(|offset| (20_000 + offset).to_string())
            .find(|value| !used.contains(&value.as_str())),
        ResourceKind::Fwmark => (0..256)
            .map(|offset| (256 + offset).to_string())
            .find(|value| !used.contains(&value.as_str())),
        ResourceKind::RouteTable => (0..256)
            .map(|offset| (100 + offset).to_string())
            .find(|value| !used.contains(&value.as_str())),
        ResourceKind::TunName => (0..100)
            .map(|offset| format!("cmtun{offset}"))
            .find(|value| !used.contains(&value.as_str())),
        ResourceKind::Socket => (0..256)
            .map(|offset| format!("sock{offset}"))
            .find(|value| !used.contains(&value.as_str())),
    };
    candidate.ok_or(LeaseError::Exhausted)
}

fn read_document(file: &mut File) -> Result<Document, LeaseError> {
    file.seek(SeekFrom::Start(0)).map_err(|_| LeaseError::Io)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|_| LeaseError::Io)?;
    serde_json::from_slice(&bytes).map_err(|_| LeaseError::Corrupt)
}

fn write_document(file: &mut File, document: &Document) -> Result<(), LeaseError> {
    let bytes = serde_json::to_vec(document).map_err(|_| LeaseError::Corrupt)?;
    file.seek(SeekFrom::Start(0)).map_err(|_| LeaseError::Io)?;
    file.set_len(0).map_err(|_| LeaseError::Io)?;
    file.write_all(&bytes).map_err(|_| LeaseError::Io)?;
    Ok(())
}

fn publish(path: &Path, document: &Document) -> Result<(), LeaseError> {
    let parent = path.parent().ok_or(LeaseError::Io)?;
    let temp = parent.join(format!(".leases-{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&temp)
        .map_err(|_| LeaseError::Io)?;
    write_document(&mut file, document)?;
    file.sync_all().map_err(|_| LeaseError::Io)?;
    fs::rename(&temp, path).map_err(|_| LeaseError::Io)?;
    if let Ok(dir) = File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// A hung holder must not stall every allocation: give up after this long.
const LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn lock_exclusive(file: &File) -> Result<(), LeaseError> {
    let deadline = std::time::Instant::now() + LOCK_TIMEOUT;
    loop {
        // SAFETY: flock on an fd owned by `file`; LOCK_NB never blocks.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EWOULDBLOCK) {
            return Err(LeaseError::Io);
        }
        if std::time::Instant::now() >= deadline {
            return Err(LeaseError::Busy);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeaseError {
    InvalidHolder,
    Exhausted,
    Conflict,
    Corrupt,
    Busy,
    Io,
}

impl std::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidHolder => "invalid lease holder",
            Self::Exhausted => "lease pool is exhausted",
            Self::Conflict => "lease update conflicted",
            Self::Corrupt => "lease table is corrupt",
            Self::Busy => "lease table is locked by another holder",
            Self::Io => "lease table operation failed",
        })
    }
}

impl std::error::Error for LeaseError {}
