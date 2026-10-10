//! Каталог владельца вычисляется из uid peer. Тело запроса путь не задаёт.

use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use super::protocol::ControlError;
use crate::core::adapter::{CoreConfig, MAX_CONFIG_BYTES};
use crate::core::instance::InstanceId;

pub const SYSTEM_BASE: &str = "/var/lib/cm/users";

#[derive(Clone)]
pub struct Owned {
    pub uid: u32,
    pub instance: InstanceId,
    pub root: PathBuf,
    pub unit: String,
}

pub fn owned(base: &Path, uid: u32, instance: &str) -> Result<Owned, ControlError> {
    let instance = InstanceId::new(instance).map_err(|_| ControlError::BadInstance)?;
    let root = base.join(format!("u{uid}"));
    let unit = format!("cm-core-u{uid}-{}.service", instance.as_str());
    Ok(Owned {
        uid,
        instance,
        root,
        unit,
    })
}

impl Owned {
    pub fn config_path(&self, generation: u64) -> PathBuf {
        self.root
            .join("instances")
            .join(self.instance.as_str())
            .join("config")
            .join(format!("gen-{generation}.json"))
    }

    pub fn journal_path(&self) -> PathBuf {
        self.root.join("journal.jsonl")
    }

    pub fn generations_path(&self) -> PathBuf {
        self.root.join("generations.json")
    }
}

pub fn read_owned_config(owned: &Owned, generation: u64) -> Result<CoreConfig, ControlError> {
    let bytes = read_owned_file(owned, &owned.config_path(generation), MAX_CONFIG_BYTES)?;
    CoreConfig::from_bytes(bytes).map_err(|_| ControlError::InvalidConfig)
}

/// Файл владельца читается без перехода по ссылке и не дальше `max` байт.
/// Проверки идут по открытому дескриптору: обычный файл, uid владельца, без записи для чужих.
pub fn read_owned_file(owned: &Owned, path: &Path, max: usize) -> Result<Vec<u8>, ControlError> {
    let parent = path.parent().ok_or(ControlError::InvalidConfig)?;
    let parent_meta = fs::symlink_metadata(parent).map_err(|_| ControlError::InvalidConfig)?;
    if parent_meta.file_type().is_symlink() || !parent_meta.is_dir() {
        return Err(ControlError::InvalidConfig);
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| ControlError::InvalidConfig)?;
    let meta = file.metadata().map_err(|_| ControlError::InvalidConfig)?;
    if !meta.is_file()
        || meta.uid() != owned.uid
        || meta.mode() & 0o022 != 0
        || meta.len() > max as u64
    {
        return Err(ControlError::InvalidConfig);
    }
    let mut bytes = Vec::new();
    file.take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ControlError::InvalidConfig)?;
    if bytes.len() > max {
        return Err(ControlError::InvalidConfig);
    }
    Ok(bytes)
}
