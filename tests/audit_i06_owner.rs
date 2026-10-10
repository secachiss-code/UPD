//! C07: the owner's directory, config and unit name follow from the peer uid alone.

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use cm::common::contract_fixtures::TempDirGuard;
use cm::common::sys::euid;
use cm::controller::owner::{owned, read_owned_config, read_owned_file};
use cm::controller::protocol::ControlError;
use cm::core::adapter::MAX_CONFIG_BYTES;
use cm::core::instance::InstanceError;
use cm::core::{
    InstanceId, InstanceOwner, InstanceRoot, NetPrivileges, render_core_unit, render_user_core_unit,
};

fn egid() -> u32 {
    // SAFETY: getegid has no arguments and cannot fail.
    unsafe { libc::getegid() }
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().mode() & 0o777
}

fn config_file(base: &Path, bytes: &[u8], file_mode: u32) -> PathBuf {
    let owner = owned(base, euid(), "browser").unwrap();
    let path = owner.config_path(1);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(file_mode)).unwrap();
    path
}

#[test]
fn c07_paths_follow_uid_and_instance() {
    let owner = owned(Path::new("/b"), 1000, "browser").unwrap();
    assert_eq!(owner.root, Path::new("/b/u1000"));
    assert_eq!(owner.unit, "cm-core-u1000-browser.service");
    assert_eq!(
        owner.config_path(7),
        Path::new("/b/u1000/instances/browser/config/gen-7.json")
    );
    assert_eq!(owner.journal_path(), Path::new("/b/u1000/journal.jsonl"));
    assert_eq!(
        owner.generations_path(),
        Path::new("/b/u1000/generations.json")
    );
    for bad in ["../x", "", "a/b", "A"] {
        assert_eq!(
            owned(Path::new("/b"), 1000, bad).err(),
            Some(ControlError::BadInstance)
        );
    }
}

#[test]
fn c07_owned_layout_modes() {
    let dir = TempDirGuard::new("cm-i06-c07-layout").unwrap();
    let root = dir.path().join("u1");
    let id = InstanceId::new("browser").unwrap();
    let owner = InstanceOwner {
        uid: euid(),
        gid: egid(),
    };
    for _ in 0..2 {
        let dirs = InstanceRoot::new(&root).create_owned(&id, owner).unwrap();
        let instance = root.join("instances/browser");
        for shared in [
            root.clone(),
            root.join("instances"),
            instance.clone(),
            instance.join("core"),
            instance.join("core/check"),
        ] {
            assert_eq!(mode(&shared), 0o711, "{}", shared.display());
        }
        for private in ["config", "cache", "run"] {
            assert_eq!(mode(&instance.join(private)), 0o700, "{private}");
        }
        assert_eq!(dirs.rendered, instance.join("core"));
        assert_eq!(dirs.check, instance.join("core/check"));
        assert_eq!(dirs.state, instance.join("cache"));
    }
    // The legacy layout keeps its paths.
    let legacy = InstanceRoot::new(dir.path().join("legacy"))
        .create(&id)
        .unwrap();
    assert_eq!(legacy.rendered, legacy.config);
    assert_eq!(legacy.check, legacy.cache);
    assert_eq!(legacy.state, legacy.root);
    assert_eq!(mode(&legacy.root), 0o700);
}

#[test]
fn c07_owned_layout_refuses_a_planted_symlink() {
    let dir = TempDirGuard::new("cm-i06-c07-symlink").unwrap();
    let root = dir.path().join("u1");
    let instance = root.join("instances/browser");
    fs::create_dir_all(&instance).unwrap();
    let target = dir.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
    symlink(&target, instance.join("config")).unwrap();
    let owner = InstanceOwner {
        uid: euid(),
        gid: egid(),
    };
    let result = InstanceRoot::new(&root).create_owned(&InstanceId::new("browser").unwrap(), owner);
    assert_eq!(result.err(), Some(InstanceError::UnsafePath));
    assert_eq!(mode(&target), 0o755);
}

#[test]
fn c07_config_is_read_only_when_it_is_the_owners_plain_file() {
    let dir = TempDirGuard::new("cm-i06-c07-read").unwrap();
    let base = dir.path();
    let owner = owned(base, euid(), "browser").unwrap();
    assert_eq!(
        read_owned_config(&owner, 1).err(),
        Some(ControlError::InvalidConfig)
    );
    let path = config_file(base, b"{\"mode\":\"direct\"}", 0o600);
    assert_eq!(
        read_owned_config(&owner, 1).unwrap().as_bytes(),
        b"{\"mode\":\"direct\"}"
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    assert_eq!(
        read_owned_config(&owner, 1).err(),
        Some(ControlError::InvalidConfig)
    );
    fs::write(&path, vec![b' '; MAX_CONFIG_BYTES + 1]).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        read_owned_config(&owner, 1).err(),
        Some(ControlError::InvalidConfig)
    );
    // Another uid computes another directory and never sees this file.
    let stranger = owned(base, euid() + 1, "browser").unwrap();
    assert_eq!(
        read_owned_config(&stranger, 1).err(),
        Some(ControlError::InvalidConfig)
    );
    // The same file under an owner with another uid is refused by the fstat check.
    let mut wrong = owner.clone();
    wrong.uid = euid() + 1;
    fs::write(&path, b"{}").unwrap();
    assert_eq!(
        read_owned_config(&wrong, 1).err(),
        Some(ControlError::InvalidConfig)
    );
}

#[test]
fn c07_symlinks_are_not_followed() {
    let dir = TempDirGuard::new("cm-i06-c07-links").unwrap();
    let base = dir.path();
    let owner = owned(base, euid(), "browser").unwrap();
    let path = config_file(base, b"{}", 0o600);
    let real = dir.path().join("real.json");
    fs::rename(&path, &real).unwrap();
    symlink(&real, &path).unwrap();
    assert_eq!(
        read_owned_config(&owner, 1).err(),
        Some(ControlError::InvalidConfig)
    );
    fs::remove_file(&path).unwrap();
    let config_dir = path.parent().unwrap().to_owned();
    let moved = dir.path().join("moved");
    fs::rename(&config_dir, &moved).unwrap();
    fs::rename(&real, moved.join("gen-1.json")).unwrap();
    symlink(&moved, &config_dir).unwrap();
    assert_eq!(
        read_owned_config(&owner, 1).err(),
        Some(ControlError::InvalidConfig)
    );
    // The bounded reader does not block on a FIFO or follow a link to a device.
    fs::remove_file(&config_dir).unwrap();
    fs::create_dir(&config_dir).unwrap();
    let device = config_dir.join("env.json");
    symlink("/dev/zero", &device).unwrap();
    assert_eq!(
        read_owned_file(&owner, &device, 4096).err(),
        Some(ControlError::InvalidConfig)
    );
}

#[test]
fn c07_user_unit_differs_only_by_user_and_paths() {
    let id = InstanceId::new("browser").unwrap();
    let user = render_user_core_unit(
        Path::new("/var/lib/cm/users/u1000"),
        1000,
        &id,
        NetPrivileges::default(),
    );
    for line in [
        "User=1000",
        "ReadWritePaths=-/var/lib/cm/users/u1000/instances/browser",
        "RuntimeDirectory=cm-core-u1000-browser",
    ] {
        assert!(user.lines().any(|candidate| candidate == line), "{line}");
    }
    let system = render_core_unit(&id, NetPrivileges::default());
    assert!(!system.contains("User="));
    assert!(system.contains("RuntimeDirectory=cm-core-browser\n"));
}
