//! Атомарная установка кандидата и откат на предыдущую версию. Старые каталоги не удаляются.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::pin::{CoreKind, DeliveryError, Pin};

pub struct Installed {
    pub version: String,
    pub path: PathBuf,
}

pub fn install(
    root: &Path,
    pin: &Pin,
    binary: &[u8],
    probe: &dyn Fn(&Path) -> Result<String, DeliveryError>,
    accept: &dyn Fn(&Path) -> Result<(), DeliveryError>,
) -> Result<Installed, DeliveryError> {
    let dir = kind_dir(root, pin.kind)?;
    let version_dir = dir.join("versions").join(pin.version);
    let tmp_dir = dir.join("versions").join(format!("{}.tmp", pin.version));
    if tmp_dir.exists() {
        fs::remove_dir_all(&tmp_dir).map_err(|_| DeliveryError::Io)?;
    }
    fs::create_dir_all(&tmp_dir).map_err(|_| DeliveryError::Io)?;
    set_mode(&tmp_dir, 0o755)?;
    let name = kind_name(pin.kind);
    let staged = tmp_dir.join(name);
    write_executable(&staged, binary)?;
    sync_dir(&tmp_dir)?;
    let probed = probe(&staged).map_err(|_| DeliveryError::VersionMismatch)?;
    if !probed.contains(pin.version_marker) {
        let _ = fs::remove_dir_all(&tmp_dir);
        return Err(DeliveryError::VersionMismatch);
    }
    if accept(&staged).is_err() {
        let _ = fs::remove_dir_all(&tmp_dir);
        return Err(DeliveryError::ConfigRejected);
    }
    if version_dir.exists() {
        fs::remove_dir_all(&version_dir).map_err(|_| DeliveryError::Io)?;
    }
    fs::rename(&tmp_dir, &version_dir).map_err(|_| DeliveryError::Io)?;
    let relative = PathBuf::from("versions").join(pin.version).join(name);
    if let Some(previous) = fs::read_link(dir.join("current")).ok()
        && previous != relative
    {
        swap_link(&dir, "previous", &previous)?;
    }
    swap_link(&dir, "current", &relative)?;
    Ok(Installed {
        version: pin.version.to_owned(),
        path: dir.join(&relative),
    })
}

pub fn rollback(root: &Path, kind: CoreKind) -> Result<Installed, DeliveryError> {
    let dir = kind_dir(root, kind)?;
    let previous =
        fs::read_link(dir.join("previous")).map_err(|_| DeliveryError::NothingToRollBack)?;
    let current =
        fs::read_link(dir.join("current")).map_err(|_| DeliveryError::NothingToRollBack)?;
    if !link_exists(&dir, &previous) {
        return Err(DeliveryError::NothingToRollBack);
    }
    swap_link(&dir, "current", &previous)?;
    swap_link(&dir, "previous", &current)?;
    let path = dir.join(&previous);
    let version = path
        .parent()
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_owned();
    Ok(Installed { version, path })
}

pub fn current(root: &Path, kind: CoreKind) -> Option<Installed> {
    let dir = root.join(kind_name(kind));
    let target = fs::read_link(dir.join("current")).ok()?;
    let path = dir.join(&target);
    if !path.is_file() {
        return None;
    }
    let version = path
        .parent()
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str())?
        .to_owned();
    Some(Installed { version, path })
}

fn kind_dir(root: &Path, kind: CoreKind) -> Result<PathBuf, DeliveryError> {
    let dir = root.join(kind_name(kind));
    if let Ok(meta) = fs::symlink_metadata(&dir)
        && (meta.file_type().is_symlink() || !meta.is_dir())
    {
        return Err(DeliveryError::Io);
    }
    fs::create_dir_all(&dir).map_err(|_| DeliveryError::Io)?;
    set_mode(&dir, 0o755)?;
    let versions = dir.join("versions");
    fs::create_dir_all(&versions).map_err(|_| DeliveryError::Io)?;
    set_mode(&versions, 0o755)?;
    Ok(dir)
}

fn kind_name(kind: CoreKind) -> &'static str {
    match kind {
        CoreKind::Mihomo => "mihomo",
        CoreKind::Xray => "xray",
    }
}

fn write_executable(path: &Path, bytes: &[u8]) -> Result<(), DeliveryError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o755)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| DeliveryError::Io)?;
    file.write_all(bytes).map_err(|_| DeliveryError::Io)?;
    file.sync_all().map_err(|_| DeliveryError::Io)?;
    let mut perms = file
        .metadata()
        .map_err(|_| DeliveryError::Io)?
        .permissions();
    perms.set_mode(0o755);
    file.set_permissions(perms).map_err(|_| DeliveryError::Io)?;
    Ok(())
}

fn set_mode(path: &Path, mode: u32) -> Result<(), DeliveryError> {
    let mut perms = fs::metadata(path)
        .map_err(|_| DeliveryError::Io)?
        .permissions();
    perms.set_mode(mode);
    fs::set_permissions(path, perms).map_err(|_| DeliveryError::Io)
}

fn sync_dir(path: &Path) -> Result<(), DeliveryError> {
    File::open(path)
        .and_then(|dir| dir.sync_all())
        .map_err(|_| DeliveryError::Io)
}

fn swap_link(dir: &Path, name: &str, target: &Path) -> Result<(), DeliveryError> {
    let tmp = dir.join(format!("{name}.tmp"));
    let _ = fs::remove_file(&tmp);
    std::os::unix::fs::symlink(target, &tmp).map_err(|_| DeliveryError::Io)?;
    fs::rename(&tmp, dir.join(name)).map_err(|_| DeliveryError::Io)
}

fn link_exists(dir: &Path, target: &Path) -> bool {
    dir.join(target).is_file()
}
