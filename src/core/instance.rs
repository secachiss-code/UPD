//! Per-instance directories. The production root is `/var/lib/cm`; tests pass their own.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const SYSTEM_ROOT: &str = "/var/lib/cm";
const DIR_MODE: u32 = 0o700;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstanceId(String);

impl InstanceId {
    pub fn new(value: &str) -> Result<Self, InstanceError> {
        let ok = !value.is_empty()
            && value.len() <= 32
            && !value.starts_with('-')
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if !ok {
            return Err(InstanceError::InvalidId);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstanceDirs {
    pub root: PathBuf,
    pub config: PathBuf,
    pub cache: PathBuf,
    pub run: PathBuf,
}

#[derive(Clone, Debug)]
pub struct InstanceRoot {
    path: PathBuf,
}

impl InstanceRoot {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn system() -> Self {
        Self::new(SYSTEM_ROOT)
    }

    pub fn create(&self, id: &InstanceId) -> Result<InstanceDirs, InstanceError> {
        let dirs = self.dirs(id);
        mkdir_private(self.path.as_path())?;
        mkdir_private(&self.path.join("instances"))?;
        mkdir_private(&dirs.root)?;
        mkdir_private(&dirs.config)?;
        mkdir_private(&dirs.cache)?;
        mkdir_private(&dirs.run)?;
        Ok(dirs)
    }

    pub fn remove(&self, id: &InstanceId) -> Result<(), InstanceError> {
        let root = self.dirs(id).root;
        let meta = match fs::symlink_metadata(&root) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(InstanceError::Io(error.kind())),
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(InstanceError::UnsafePath);
        }
        fs::remove_dir_all(&root).map_err(|error| InstanceError::Io(error.kind()))
    }

    fn dirs(&self, id: &InstanceId) -> InstanceDirs {
        let root = self.path.join("instances").join(id.as_str());
        InstanceDirs {
            config: root.join("config"),
            cache: root.join("cache"),
            run: root.join("run"),
            root,
        }
    }
}

fn mkdir_private(path: &Path) -> Result<(), InstanceError> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(InstanceError::Io(error.kind())),
    }
    let meta = fs::symlink_metadata(path).map_err(|error| InstanceError::Io(error.kind()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(InstanceError::UnsafePath);
    }
    let mut permissions = meta.permissions();
    permissions.set_mode(DIR_MODE);
    fs::set_permissions(path, permissions).map_err(|error| InstanceError::Io(error.kind()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstanceError {
    InvalidId,
    UnsafePath,
    Io(std::io::ErrorKind),
}

impl std::fmt::Display for InstanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidId => "invalid instance id",
            Self::UnsafePath => "instance path is not a private directory",
            Self::Io(_) => "instance directory operation failed",
        })
    }
}

impl std::error::Error for InstanceError {}
