//! Поколения экземпляра и ответ на повторный запрос с тем же ключом.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use super::journal::{Journal, TxnStatus};
use super::protocol::ControlError;

pub struct Generations {
    path: PathBuf,
    map: BTreeMap<String, u64>,
}

impl Generations {
    pub fn load(path: &Path) -> Result<Self, ControlError> {
        if !path.exists() {
            return Ok(Self {
                path: path.to_owned(),
                map: BTreeMap::new(),
            });
        }
        if fs::symlink_metadata(path)
            .map_err(|_| ControlError::Failed)?
            .file_type()
            .is_symlink()
        {
            return Err(ControlError::Failed);
        }
        let bytes = fs::read(path).map_err(|_| ControlError::Failed)?;
        let map = serde_json::from_slice(&bytes).map_err(|_| ControlError::Failed)?;
        Ok(Self {
            path: path.to_owned(),
            map,
        })
    }

    pub fn current(&self, instance: &str) -> Option<u64> {
        self.map.get(instance).copied()
    }

    pub fn set(&mut self, instance: &str, generation: u64) -> Result<(), ControlError> {
        let _guard = file_lock();
        self.reload_locked()?;
        self.map.insert(instance.to_owned(), generation);
        self.store()
    }

    pub fn clear(&mut self, instance: &str) -> Result<(), ControlError> {
        let _guard = file_lock();
        self.reload_locked()?;
        self.map.remove(instance);
        self.store()
    }

    fn reload_locked(&mut self) -> Result<(), ControlError> {
        if self.path.exists() {
            self.map = Self::load(&self.path)?.map;
        }
        Ok(())
    }

    fn store(&self) -> Result<(), ControlError> {
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            fs::create_dir_all(parent).map_err(|_| ControlError::Failed)?;
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
            let bytes = serde_json::to_vec(&self.map).map_err(|_| ControlError::Failed)?;
            file.write_all(&bytes).map_err(|_| ControlError::Failed)?;
            file.sync_all().map_err(|_| ControlError::Failed)?;
            let mut perms = file
                .metadata()
                .map_err(|_| ControlError::Failed)?
                .permissions();
            perms.set_mode(0o600);
            file.set_permissions(perms)
                .map_err(|_| ControlError::Failed)?;
        }
        fs::rename(&tmp, &self.path).map_err(|_| ControlError::Failed)?;
        if let Some(parent) = self.path.parent()
            && let Ok(dir) = File::open(parent)
        {
            let _ = dir.sync_all();
        }
        Ok(())
    }
}

pub enum Replay {
    Fresh,
    Stored(String),
}

pub fn replay_or_fresh(journal: &Journal, txn: &str, digest: &str) -> Result<Replay, ControlError> {
    let Some(state) = journal.find(txn)? else {
        return Ok(Replay::Fresh);
    };
    if state.digest != digest {
        return Err(ControlError::Conflict);
    }
    match state.status {
        TxnStatus::Committed => Ok(Replay::Stored(state.reply.unwrap_or_default())),
        TxnStatus::Pending | TxnStatus::Dirty | TxnStatus::Aborted => Err(ControlError::Conflict),
    }
}

pub fn check_start(current: Option<u64>, requested: u64) -> Result<(), ControlError> {
    if current.is_some_and(|current| requested < current) {
        Err(ControlError::GenerationMismatch)
    } else {
        Ok(())
    }
}

pub fn check_running(current: Option<u64>, requested: u64) -> Result<(), ControlError> {
    match current {
        None => Err(ControlError::NotRunning),
        Some(current) if current == requested => Ok(()),
        Some(_) => Err(ControlError::GenerationMismatch),
    }
}

fn file_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}
