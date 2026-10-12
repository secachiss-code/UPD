//! Каталоги данных и сокетов сессии выводятся из профиля. Контроллер их не создаёт.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataState {
    #[default]
    Existing,
    Isolated,
    Fresh,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcPolicy {
    #[default]
    Shared,
    Private,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateDirs {
    pub root: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

pub fn data_dirs(
    home: &Path,
    profile: &str,
    state: DataState,
    session: &str,
) -> Option<PrivateDirs> {
    let root = match state {
        DataState::Existing => return None,
        DataState::Isolated => home.join(".local/share/cm/data").join(profile),
        DataState::Fresh => home
            .join(".local/share/cm/data")
            .join(profile)
            .join(format!("fresh-{session}")),
    };
    Some(PrivateDirs {
        config: root.join("config"),
        data: root.join("data"),
        cache: root.join("cache"),
        state: root.join("state"),
        root,
    })
}

pub fn runtime_dir(runtime: &Path, session: &str) -> PathBuf {
    runtime.join("cm").join(session)
}

pub fn env_overrides(dirs: &PrivateDirs) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("XDG_CONFIG_HOME".to_owned(), path_text(&dirs.config)),
        ("XDG_DATA_HOME".to_owned(), path_text(&dirs.data)),
        ("XDG_CACHE_HOME".to_owned(), path_text(&dirs.cache)),
        ("XDG_STATE_HOME".to_owned(), path_text(&dirs.state)),
    ])
}

pub fn prepare(dirs: &PrivateDirs) -> std::io::Result<()> {
    for path in [
        &dirs.root,
        &dirs.config,
        &dirs.data,
        &dirs.cache,
        &dirs.state,
    ] {
        ensure_dir(path)?;
    }
    Ok(())
}

pub fn prepare_runtime(path: &Path) -> std::io::Result<()> {
    ensure_dir(path)
}

pub fn check_dir(path: &Path, uid: u32) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    meta.file_type().is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0
}

fn ensure_dir(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(std::io::Error::other("not a directory"));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent()
                && parent != Path::new("")
                && parent != path
            {
                ensure_dir(parent)?;
            }
            fs::create_dir(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
