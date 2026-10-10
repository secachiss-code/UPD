//! Каталог личностей пользователя.

use super::model::{BrowserIdentityProfile, ModelError, validate_id};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

const DIRECTORY_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    Unsafe,
    Exists,
    NotFound,
    BadId,
    Io,
    Invalid,
}

impl StoreError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unsafe => "Unsafe",
            Self::Exists => "Exists",
            Self::NotFound => "NotFound",
            Self::BadId => "BadId",
            Self::Io => "Io",
            Self::Invalid => "Invalid",
        }
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unsafe => t!("каталог личностей небезопасен"),
            Self::Exists => t!("личность уже существует"),
            Self::NotFound => t!("личность не найдена"),
            Self::BadId => t!("неверный идентификатор личности"),
            Self::Io => t!("не удалось записать личность"),
            Self::Invalid => t!("неверные данные личности"),
        })
    }
}

pub struct IdentityStore {
    root: PathBuf,
}

pub fn root() -> PathBuf {
    if let Some(value) = nonempty_env("CM_IDENTITY_ROOT") {
        return PathBuf::from(value);
    }
    if let Some(value) = nonempty_env("XDG_DATA_HOME") {
        return PathBuf::from(value).join("cm").join("identities");
    }
    match nonempty_env("HOME") {
        Some(home) => PathBuf::from(home).join(".local/share/cm/identities"),
        None => PathBuf::from(".local/share/cm/identities"),
    }
}

impl IdentityStore {
    pub fn open(root: PathBuf) -> Result<Self, StoreError> {
        prepare_root(&root)?;
        Ok(Self { root })
    }

    pub fn create(&self, profile: &BrowserIdentityProfile) -> Result<(), StoreError> {
        let dir = self.checked_dir(&profile.id)?;
        match fs::symlink_metadata(&dir) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(StoreError::Unsafe),
            Ok(_) => return Err(StoreError::Exists),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(StoreError::Io),
        }
        create_private_dir(&dir)?;
        create_private_dir(&dir.join("profile"))?;
        let bytes = serde_json::to_vec_pretty(profile).map_err(|_| StoreError::Invalid)?;
        write_private(&dir.join("identity.json"), &bytes)
    }

    pub fn load(&self, id: &str) -> Result<BrowserIdentityProfile, StoreError> {
        let path = self.checked_dir(id)?.join("identity.json");
        let text = fs::read_to_string(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                StoreError::NotFound
            } else {
                StoreError::Io
            }
        })?;
        BrowserIdentityProfile::from_json(&text).map_err(|error| match error {
            ModelError::BadId => StoreError::BadId,
            ModelError::UnsupportedSchema | ModelError::Invalid => StoreError::Invalid,
        })
    }

    pub fn save(&self, profile: &BrowserIdentityProfile) -> Result<(), StoreError> {
        let dir = self.checked_dir(&profile.id)?;
        if !dir.is_dir() {
            return Err(StoreError::NotFound);
        }
        let bytes = serde_json::to_vec_pretty(profile).map_err(|_| StoreError::Invalid)?;
        write_private(&dir.join("identity.json"), &bytes)
    }

    pub fn list(&self) -> Result<Vec<String>, StoreError> {
        let mut ids = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(|_| StoreError::Io)? {
            let entry = entry.map_err(|_| StoreError::Io)?;
            let meta = fs::symlink_metadata(entry.path()).map_err(|_| StoreError::Io)?;
            if meta.file_type().is_symlink() || !meta.is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if validate_id(&name).is_err() || !entry.path().join("identity.json").is_file() {
                continue;
            }
            ids.push(name);
        }
        ids.sort();
        Ok(ids)
    }

    pub fn remove(&self, id: &str, purge: bool) -> Result<(), StoreError> {
        let dir = self.checked_dir(id)?;
        let meta = fs::symlink_metadata(&dir).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                StoreError::NotFound
            } else {
                StoreError::Io
            }
        })?;
        if meta.file_type().is_symlink() {
            return Err(StoreError::Unsafe);
        }
        if !meta.is_dir() {
            return Err(StoreError::NotFound);
        }
        if purge {
            return fs::remove_dir_all(&dir).map_err(|_| StoreError::Io);
        }
        remove_if_present(&dir.join("identity.json"))?;
        remove_if_present(&dir.join("state.json"))
    }

    pub fn dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    pub fn profile_dir(&self, id: &str) -> PathBuf {
        self.dir(id).join("profile")
    }

    fn checked_dir(&self, id: &str) -> Result<PathBuf, StoreError> {
        validate_id(id).map_err(|_| StoreError::BadId)?;
        Ok(self.dir(id))
    }
}

pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let mut temporary_name = path.file_name().ok_or(StoreError::Io)?.to_os_string();
    temporary_name.push(".tmp");
    let temporary = path.with_file_name(temporary_name);
    if fs::symlink_metadata(&temporary).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(StoreError::Unsafe);
    }
    {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(FILE_MODE)
            .open(&temporary)
            .map_err(|_| StoreError::Io)?;
        file.write_all(bytes).map_err(|_| StoreError::Io)?;
        file.sync_all().map_err(|_| StoreError::Io)?;
    }
    fs::set_permissions(&temporary, fs::Permissions::from_mode(FILE_MODE))
        .map_err(|_| StoreError::Io)?;
    fs::rename(&temporary, path).map_err(|_| StoreError::Io)?;
    if let Some(parent) = path.parent()
        && let Ok(directory) = File::open(parent)
    {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn prepare_root(root: &Path) -> Result<(), StoreError> {
    let uid = crate::common::sys::euid();
    match fs::symlink_metadata(root) {
        Ok(meta) if meta.file_type().is_symlink() => return Err(StoreError::Unsafe),
        Ok(meta) if !meta.is_dir() || meta.uid() != uid => return Err(StoreError::Unsafe),
        Ok(_) => fs::set_permissions(root, fs::Permissions::from_mode(DIRECTORY_MODE))
            .map_err(|_| StoreError::Io)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(root).map_err(|_| StoreError::Io)?;
            let meta = fs::symlink_metadata(root).map_err(|_| StoreError::Io)?;
            if meta.file_type().is_symlink() || !meta.is_dir() || meta.uid() != uid {
                return Err(StoreError::Unsafe);
            }
            fs::set_permissions(root, fs::Permissions::from_mode(DIRECTORY_MODE))
                .map_err(|_| StoreError::Io)?;
        }
        Err(_) => return Err(StoreError::Io),
    }
    for entry in fs::read_dir(root).map_err(|_| StoreError::Io)? {
        let entry = entry.map_err(|_| StoreError::Io)?;
        let meta = fs::symlink_metadata(entry.path()).map_err(|_| StoreError::Io)?;
        if meta.file_type().is_symlink() || (meta.is_dir() && meta.uid() != uid) {
            return Err(StoreError::Unsafe);
        }
    }
    Ok(())
}

fn create_private_dir(path: &Path) -> Result<(), StoreError> {
    fs::DirBuilder::new()
        .mode(DIRECTORY_MODE)
        .create(path)
        .map_err(|_| StoreError::Io)?;
    fs::set_permissions(path, fs::Permissions::from_mode(DIRECTORY_MODE))
        .map_err(|_| StoreError::Io)?;
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<(), StoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(StoreError::Io),
    }
}

fn nonempty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}
