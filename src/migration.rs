//! Fail-closed startup guards plus an explicit journaled UPD -> CM migration.
//!
//! Production callers must use `/`. An explicit filesystem root lets fixtures
//! exercise the same inventory without environment variables changing the guard.

use std::fs;
use std::path::{Component, Path, PathBuf};

pub mod transaction;
pub mod manual;
mod legacy_v027;

const LEGACY_PATHS: &[&str] = &[
    "/etc/upd.conf",
    "/etc/upd",
    "/var/lib/upd",
    "/run/upd",
    "/run/upd/helper.sock",
    "/etc/cron.d/upd",
    "/etc/pacman.d/hooks/zz-upd.hook",
    "/usr/share/libalpm/hooks/zz-upd.hook",
    "/etc/apt/apt.conf.d/99upd",
    "/etc/NetworkManager/dispatcher.d/90-upd",
    "/usr/lib/NetworkManager/dispatcher.d/90-upd",
    "/usr/local/bin/upd",
    "/usr/local/bin/upd-cosmic",
    "/usr/bin/upd",
    "/usr/bin/upd-cosmic",
    "/usr/local/share/applications/io.github.upd.desktop",
    "/usr/local/share/applications/io.github.upd.Applet.desktop",
    "/usr/share/applications/io.github.upd.desktop",
    "/usr/share/applications/io.github.upd.Applet.desktop",
    "/usr/local/share/icons/hicolor/scalable/apps/io.github.upd.svg",
    "/usr/local/share/icons/hicolor/symbolic/apps/io.github.upd-symbolic.svg",
    "/usr/share/icons/hicolor/scalable/apps/io.github.upd.svg",
    "/usr/share/icons/hicolor/symbolic/apps/io.github.upd-symbolic.svg",
    "/etc/xdg/autostart/io.github.upd.desktop",
    "/etc/xdg/autostart/io.github.upd.Applet.desktop",
    "/usr/share/polkit-1/actions/io.github.upd.policy",
];

const UNIT_DIRS: &[&str] = &[
    "/etc/systemd/system",
    "/run/systemd/system",
    "/usr/lib/systemd/system",
    "/etc/systemd/user",
    "/run/systemd/user",
    "/usr/lib/systemd/user",
];

const LEGACY_UNITS: &[&str] = &[
    "upd-auto.service",
    "upd-auto.timer",
    "upd-net.service",
    "upd-net.timer",
    "upd-vpn.service",
    "upd-helper.socket",
    "upd-helper.service",
    "upd-mirrors.path",
    "upd-mirrors-apply.service",
    "upd-notify.service",
    "upd-notify.timer",
];

fn cannot_verify(path: &Path, reason: impl std::fmt::Display) -> String {
    format!(
        "CM blocked: cannot verify legacy UPD path {}: {reason}. Unrequested automatic UPD -> CM migration is unsupported; use cm migration plan to check the explicit migration path. No configuration, state or installation changes were made.",
        path.display()
    )
}

/// Inspect each component without following symlinks outside a fixture root.
/// A final symlink is an existing entry, including a dangling symlink or mask.
fn present(root: &Path, path: &Path) -> Result<bool, String> {
    let mut actual = root.to_path_buf();
    let mut logical = PathBuf::from("/");
    let relative = path.strip_prefix("/").map_err(|e| cannot_verify(path, e))?;
    let mut parts = relative.components().peekable();
    while let Some(part) = parts.next() {
        let Component::Normal(name) = part else {
            return Err(cannot_verify(path, "invalid inventory path"));
        };
        actual.push(name);
        logical.push(name);
        let metadata = match fs::symlink_metadata(&actual) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(cannot_verify(&logical, error)),
        };
        if parts.peek().is_some() && !metadata.is_dir() {
            return Err(cannot_verify(&logical, "ancestor is a symlink or is not a directory"));
        }
    }
    Ok(true)
}

/// Inventory known legacy entries. Presence is not proof of ownership, and this
/// function never reads credentials, changes files, or invokes external commands.
pub fn inventory_legacy(root: &Path) -> Result<Vec<PathBuf>, String> {
    if !root.is_absolute() || root.components().any(|part| matches!(part, Component::ParentDir)) {
        return Err(cannot_verify(root, "filesystem root must be an absolute path without '..'"));
    }
    let metadata = fs::symlink_metadata(root).map_err(|error| cannot_verify(root, error))?;
    if !metadata.is_dir() {
        return Err(cannot_verify(root, "filesystem root must be a directory, not a symlink"));
    }
    let mut found = Vec::new();
    for path in LEGACY_PATHS {
        if present(root, Path::new(path))? {
            found.push(PathBuf::from(*path));
        }
    }
    for directory in UNIT_DIRS {
        for unit in LEGACY_UNITS {
            for name in [(*unit).to_string(), format!("{unit}.d")] {
                let path = Path::new(directory).join(name);
                if present(root, &path)? {
                    found.push(path);
                }
            }
        }
        // Enabled links can survive after the original unit has disappeared.
        // Inspect arbitrary target names as well as the generated targets.
        let logical_dir = Path::new(directory);
        if !present(root, logical_dir)? {
            continue;
        }
        let actual_dir = root.join(directory.trim_start_matches('/'));
        let metadata = fs::symlink_metadata(&actual_dir).map_err(|error| cannot_verify(logical_dir, error))?;
        if !metadata.is_dir() {
            return Err(cannot_verify(logical_dir, "unit directory is a symlink or is not a directory"));
        }
        for entry in fs::read_dir(&actual_dir).map_err(|error| cannot_verify(logical_dir, error))? {
            let entry = entry.map_err(|error| cannot_verify(logical_dir, error))?;
            let name = entry.file_name();
            let Some(name_text) = name.to_str() else { continue };
            if !name_text.ends_with(".wants") && !name_text.ends_with(".requires") {
                continue;
            }
            for unit in LEGACY_UNITS {
                let path = logical_dir.join(&name).join(unit);
                if present(root, &path)? {
                    found.push(path);
                }
            }
        }
    }
    found.sort();
    found.dedup();
    Ok(found)
}

/// Fail closed before config load/save and every installation mutation.
/// Environment overrides (including CM_STATE_DIR / UPD_STATE_DIR) do not bypass
/// this check. Old/new conflicts and foreign files at legacy paths stay intact.
pub fn preflight_install(root: &Path) -> Result<(), String> {
    preflight_legacy(root, "install")
}

/// Privileged commands can write the legacy config/state through fallback paths,
/// even when their arguments are invalid or they normally only display status.
/// Check before config load/save, independently of all environment overrides.
/// Unprivileged readers and early help/version returns retain their old behavior.
pub fn preflight_startup(root: &Path, command: &str, privileged: bool) -> Result<(), String> {
    if command == "install" {
        return preflight_install(root);
    }
    if !privileged {
        return Ok(());
    }
    preflight_legacy(root, "startup")
}

fn preflight_legacy(root: &Path, boundary: &str) -> Result<(), String> {
    let found = inventory_legacy(root)?;
    if found.is_empty() {
        return Ok(());
    }
    let paths = found.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", ");
    Err(format!(
        "CM {boundary} blocked: legacy or ambiguous UPD entries found: {paths}. Ownership is not established; these files were not changed. Unrequested automatic UPD -> CM migration is unsupported; use cm migration plan to check the explicit supported migration path. No configuration, state or installation changes were made; keep existing UPD data and services intact."
    ))
}
