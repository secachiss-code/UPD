//! Per-instance directories. The production root is `/var/lib/cm`; tests pass their own.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const SYSTEM_ROOT: &str = "/var/lib/cm";
const DIR_MODE: u32 = 0o700;
/// Traversable, not listable: the owner reaches its own directories through the manager's.
const SHARED_MODE: u32 = 0o711;

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
    /// Where the manager writes the config the core is started with.
    pub rendered: PathBuf,
    /// Where the manager writes a candidate config for the core's own check.
    pub check: PathBuf,
    /// The core's home directory (`-d`).
    pub state: PathBuf,
}

/// The user a core runs as when the manager itself is privileged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstanceOwner {
    pub uid: u32,
    pub gid: u32,
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

    /// Layout for a core that runs as `owner` under a privileged manager.
    ///
    /// The manager keeps every directory it writes into: the tree down to the instance and
    /// `core/` stay with the manager (mode 0711), so the owner can pass through but cannot
    /// plant or swap an entry there. The owner gets `config/` (its inbox: the manager only
    /// reads it, with checks), `cache/` (the core's home) and `run/` (the API socket).
    pub fn create_owned(
        &self,
        id: &InstanceId,
        owner: InstanceOwner,
    ) -> Result<InstanceDirs, InstanceError> {
        let mut dirs = self.dirs(id);
        mkdir_mode(self.path.as_path(), SHARED_MODE, None)?;
        mkdir_mode(&self.path.join("instances"), SHARED_MODE, None)?;
        mkdir_mode(&dirs.root, SHARED_MODE, None)?;
        mkdir_mode(&dirs.config, DIR_MODE, Some(owner))?;
        mkdir_mode(&dirs.cache, DIR_MODE, Some(owner))?;
        mkdir_mode(&dirs.run, DIR_MODE, Some(owner))?;
        let core = dirs.root.join("core");
        mkdir_mode(&core, SHARED_MODE, None)?;
        let check = core.join("check");
        mkdir_mode(&check, SHARED_MODE, None)?;
        dirs.rendered = core;
        dirs.check = check;
        dirs.state = dirs.cache.clone();
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
            rendered: root.join("config"),
            check: root.join("cache"),
            state: root.clone(),
            root,
        }
    }
}

/// Create or adopt a directory through a descriptor, so a symlink is never followed.
fn mkdir_mode(path: &Path, mode: u32, owner: Option<InstanceOwner>) -> Result<(), InstanceError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(InstanceError::Io(error.kind())),
    }
    let dir = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| InstanceError::UnsafePath)?;
    if let Some(owner) = owner {
        // SAFETY: fchown on a descriptor this function owns; no pointer is passed.
        if unsafe { libc::fchown(dir.as_raw_fd(), owner.uid, owner.gid) } != 0 {
            return Err(InstanceError::Io(std::io::Error::last_os_error().kind()));
        }
    }
    dir.set_permissions(fs::Permissions::from_mode(mode))
        .map_err(|error| InstanceError::Io(error.kind()))
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
