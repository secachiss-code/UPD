//! Журнал владения: JSON-строки, 0600, дозапись и восстановление после обрыва.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use super::protocol::ControlError;

const COMPACT_AT: u64 = 1024 * 1024;
const KEEP_COMMITTED: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub txn: String,
    pub event: Event,
    pub instance: String,
    pub generation: u64,
    pub digest: String,
    pub step: Option<String>,
    pub reply: Option<String>,
    pub at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    Begin,
    Step,
    Commit,
    Abort,
    Compensated,
    CompensationFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxnStatus {
    Pending,
    Committed,
    Aborted,
    Dirty,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxnState {
    pub txn: String,
    pub instance: String,
    pub generation: u64,
    pub digest: String,
    pub steps_done: Vec<String>,
    pub status: TxnStatus,
    pub reply: Option<String>,
}

pub struct Journal {
    path: PathBuf,
    file: File,
}

impl Journal {
    pub fn open(path: &Path) -> Result<Self, ControlError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            match fs::symlink_metadata(parent) {
                Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
                    return Err(ControlError::Failed);
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fs::create_dir_all(parent).map_err(|_| ControlError::Failed)?;
                    let mut perms = fs::metadata(parent)
                        .map_err(|_| ControlError::Failed)?
                        .permissions();
                    perms.set_mode(0o700);
                    fs::set_permissions(parent, perms).map_err(|_| ControlError::Failed)?;
                }
                Err(_) => return Err(ControlError::Failed),
            }
        }
        if let Ok(meta) = fs::symlink_metadata(path)
            && meta.file_type().is_symlink()
        {
            return Err(ControlError::Failed);
        }
        let file = OpenOptions::new()
            .read(true)
            .create(true)
            .append(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|_| ControlError::Failed)?;
        let mut perms = file
            .metadata()
            .map_err(|_| ControlError::Failed)?
            .permissions();
        perms.set_mode(0o600);
        file.set_permissions(perms)
            .map_err(|_| ControlError::Failed)?;
        let mut journal = Self {
            path: path.to_owned(),
            file,
        };
        journal.truncate_tail()?;
        Ok(journal)
    }

    pub fn append(&mut self, record: &Record) -> Result<(), ControlError> {
        let _guard = lock();
        let mut line = serde_json::to_vec(record).map_err(|_| ControlError::Failed)?;
        line.push(b'\n');
        self.file
            .write_all(&line)
            .map_err(|_| ControlError::Failed)?;
        self.file.sync_data().map_err(|_| ControlError::Failed)?;
        if record.event == Event::Commit {
            self.compact_locked()?;
        }
        Ok(())
    }

    pub fn replay(&self) -> Result<Vec<TxnState>, ControlError> {
        let text = fs::read_to_string(&self.path).map_err(|_| ControlError::Failed)?;
        parse_records(&text)
    }

    pub fn find(&self, txn: &str) -> Result<Option<TxnState>, ControlError> {
        Ok(self.replay()?.into_iter().find(|state| state.txn == txn))
    }

    pub fn compact(&mut self) -> Result<(), ControlError> {
        let _guard = lock();
        self.compact_locked()
    }

    fn compact_locked(&mut self) -> Result<(), ControlError> {
        let len = self
            .file
            .metadata()
            .map_err(|_| ControlError::Failed)?
            .len();
        if len <= COMPACT_AT {
            return Ok(());
        }
        let text = fs::read_to_string(&self.path).map_err(|_| ControlError::Failed)?;
        let states = parse_records(&text)?;
        let mut committed: Vec<&str> = states
            .iter()
            .filter(|state| state.status == TxnStatus::Committed)
            .map(|state| state.txn.as_str())
            .collect();
        let skip = committed.len().saturating_sub(KEEP_COMMITTED);
        committed.drain(..skip);
        let keep_committed: BTreeSet<&str> = committed.into_iter().collect();
        let keep_open: BTreeSet<&str> = states
            .iter()
            .filter(|state| matches!(state.status, TxnStatus::Pending | TxnStatus::Dirty))
            .map(|state| state.txn.as_str())
            .collect();
        let mut kept = String::new();
        for line in text.split_inclusive('\n') {
            if !line.ends_with('\n') {
                continue;
            }
            let Some(record) = serde_json::from_str::<Record>(line.trim_end_matches('\n')).ok()
            else {
                continue;
            };
            if keep_open.contains(record.txn.as_str())
                || keep_committed.contains(record.txn.as_str())
            {
                kept.push_str(line);
            }
        }
        let tmp = tmp_path(&self.path);
        {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&tmp)
                .map_err(|_| ControlError::Failed)?;
            file.write_all(kept.as_bytes())
                .map_err(|_| ControlError::Failed)?;
            file.sync_all().map_err(|_| ControlError::Failed)?;
        }
        fs::rename(&tmp, &self.path).map_err(|_| ControlError::Failed)?;
        if let Some(parent) = self.path.parent()
            && let Ok(dir) = File::open(parent)
        {
            let _ = dir.sync_all();
        }
        self.file = OpenOptions::new()
            .read(true)
            .append(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&self.path)
            .map_err(|_| ControlError::Failed)?;
        Ok(())
    }

    fn truncate_tail(&mut self) -> Result<(), ControlError> {
        let _guard = lock();
        let mut bytes = Vec::new();
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|_| ControlError::Failed)?;
        self.file
            .read_to_end(&mut bytes)
            .map_err(|_| ControlError::Failed)?;
        let mut end = bytes.len();
        if end > 0 && bytes[end - 1] != b'\n' {
            end = bytes[..end]
                .iter()
                .rposition(|byte| *byte == b'\n')
                .map(|pos| pos + 1)
                .unwrap_or(0);
        }
        if end > 0 {
            let text = std::str::from_utf8(&bytes[..end]).unwrap_or("");
            let body = text.trim_end_matches('\n');
            let last = body.rsplit('\n').next().unwrap_or("");
            if last.is_empty() || serde_json::from_str::<Record>(last).is_err() {
                end = body.len().saturating_sub(last.len());
            }
        }
        if end < bytes.len() {
            self.file
                .set_len(end as u64)
                .map_err(|_| ControlError::Failed)?;
            self.file.sync_data().map_err(|_| ControlError::Failed)?;
        }
        self.file
            .seek(SeekFrom::End(0))
            .map_err(|_| ControlError::Failed)?;
        Ok(())
    }
}

fn parse_records(text: &str) -> Result<Vec<TxnState>, ControlError> {
    let mut lines: Vec<&str> = Vec::new();
    for line in text.split_inclusive('\n') {
        if line.ends_with('\n') {
            lines.push(line.trim_end_matches('\n'));
        }
    }
    let mut states: Vec<TxnState> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let record = match serde_json::from_str::<Record>(line) {
            Ok(record) => record,
            Err(_) if index + 1 == lines.len() => continue,
            Err(_) => return Err(ControlError::Failed),
        };
        apply_record(&mut states, record)?;
    }
    Ok(states)
}

fn apply_record(states: &mut Vec<TxnState>, record: Record) -> Result<(), ControlError> {
    if record.event == Event::Begin {
        states.push(TxnState {
            txn: record.txn,
            instance: record.instance,
            generation: record.generation,
            digest: record.digest,
            steps_done: Vec::new(),
            status: TxnStatus::Pending,
            reply: None,
        });
        return Ok(());
    }
    let Some(state) = states.iter_mut().find(|state| state.txn == record.txn) else {
        return Err(ControlError::Failed);
    };
    match record.event {
        Event::Begin => {}
        Event::Step => {
            if let Some(step) = record.step {
                state.steps_done.push(step);
            }
        }
        Event::Commit => {
            state.status = TxnStatus::Committed;
            state.reply = record.reply;
        }
        Event::Abort | Event::Compensated => state.status = TxnStatus::Aborted,
        Event::CompensationFailed => state.status = TxnStatus::Dirty,
    }
    Ok(())
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}
