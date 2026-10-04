//! Actual production preflight, rooted only in disposable local fixtures.
//! This validates the temporary refusal boundary, never data migration.
use cm::common::contract_fixtures::{EnvGuard, TempDirGuard, isolation_lock};
use cm::migration::{inventory_legacy, preflight_install, preflight_startup};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    mode: u32,
    uid: u32,
    gid: u32,
    bytes_or_link: Vec<u8>,
}

fn inventory(root: &Path) -> BTreeMap<PathBuf, Entry> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Entry>) {
        let meta = fs::symlink_metadata(path).unwrap();
        let bytes_or_link = if meta.file_type().is_symlink() {
            fs::read_link(path)
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
                .to_vec()
        } else if meta.is_file() {
            fs::read(path).unwrap()
        } else {
            Vec::new()
        };
        out.insert(
            path.strip_prefix(root).unwrap().to_path_buf(),
            Entry {
                mode: meta.mode(),
                uid: meta.uid(),
                gid: meta.gid(),
                bytes_or_link,
            },
        );
        if meta.is_dir() {
            for child in fs::read_dir(path).unwrap() {
                visit(root, &child.unwrap().path(), out);
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out);
    out
}

fn put(root: &Path, relative: &str) {
    let p = root.join(relative);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, b"private fixture data\0\xff preserved verbatim\n").unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o640)).unwrap();
}

fn unchanged(root: &Path, blocked: bool) {
    let before = inventory(root);
    let result = preflight_install(root);
    if blocked {
        let message = result.expect_err("legacy or ambiguous layout must be refused");
        assert!(message.contains("blocked"));
        assert!(message.contains("unsupported"));
    } else {
        result.expect("fresh or CM-only layout must pass preflight");
    }
    assert_eq!(
        before,
        inventory(root),
        "preflight changed bytes, paths or permissions"
    );
}

#[test]
fn i01_fresh_and_cm_only_pass_without_any_write() {
    let d = TempDirGuard::new("i01-fresh").unwrap();
    unchanged(d.path(), false);
    for p in [
        "etc/cm.conf",
        "etc/cm/vpn/subs.json",
        "var/lib/cm/vpn/config.yaml",
        "usr/local/bin/cm",
    ] {
        put(d.path(), p);
    }
    unchanged(d.path(), false);
    assert!(inventory_legacy(d.path()).unwrap().is_empty());
}

#[test]
fn i01_old_only_and_all_old_new_conflicts_preserve_full_contents() {
    for pair in [
        ("etc/upd.conf", "etc/cm.conf"),
        ("etc/upd/vpn/subs.json", "etc/cm/vpn/subs.json"),
        ("var/lib/upd/vpn/config.yaml", "var/lib/cm/vpn/config.yaml"),
    ] {
        let d = TempDirGuard::new("i01-conflict").unwrap();
        put(d.path(), pair.0);
        unchanged(d.path(), true);
        put(d.path(), pair.1);
        unchanged(d.path(), true);
        unchanged(d.path(), true); // repeated refusal is stable
    }
}

#[test]
fn i01_foreign_hooks_cron_desktop_cli_and_policy_are_refused_and_preserved() {
    for p in [
        "etc/cron.d/upd",
        "etc/pacman.d/hooks/zz-upd.hook",
        "usr/share/libalpm/hooks/zz-upd.hook",
        "etc/apt/apt.conf.d/99upd",
        "etc/NetworkManager/dispatcher.d/90-upd",
        "usr/lib/NetworkManager/dispatcher.d/90-upd",
        "usr/local/bin/upd",
        "usr/local/bin/upd-cosmic",
        "usr/bin/upd",
        "usr/bin/upd-cosmic",
        "usr/local/share/applications/io.github.upd.desktop",
        "usr/share/applications/io.github.upd.Applet.desktop",
        "etc/xdg/autostart/io.github.upd.desktop",
        "usr/share/polkit-1/actions/io.github.upd.policy",
    ] {
        let d = TempDirGuard::new("i01-foreign").unwrap();
        put(d.path(), p);
        unchanged(d.path(), true);
        assert!(!inventory_legacy(d.path()).unwrap().is_empty());
    }
}

#[test]
fn i01_units_dropins_and_enabled_links_refuse_before_ownership_changes() {
    for dir in [
        "etc/systemd/system",
        "run/systemd/system",
        "usr/lib/systemd/system",
        "etc/systemd/user",
        "run/systemd/user",
        "usr/lib/systemd/user",
    ] {
        let d = TempDirGuard::new("i01-units").unwrap();
        put(d.path(), &format!("{dir}/upd-vpn.service")); // deliberately foreign content
        put(d.path(), &format!("{dir}/upd-helper.service.d/admin.conf"));
        unchanged(d.path(), true);
    }
    for suffix in ["wants", "requires"] {
        let d = TempDirGuard::new("i01-enabled").unwrap();
        let link = d.path().join(format!(
            "etc/systemd/system/custom.target.{suffix}/upd-net.timer"
        ));
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink("/missing-legacy-unit", &link).unwrap();
        unchanged(d.path(), true);
    }
}

#[test]
fn i01_dangling_final_and_ancestor_symlinks_never_follow_outside_root() {
    let outside = TempDirGuard::new("i01-outside").unwrap();
    put(outside.path(), "sentinel");
    let outside_before = inventory(outside.path());
    for relative in ["etc/upd.conf", "var/lib/upd", "etc"] {
        let d = TempDirGuard::new("i01-symlink").unwrap();
        let p = d.path().join(relative);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        symlink(
            if relative == "etc" {
                outside.path()
            } else {
                Path::new("/missing-i01-target")
            },
            &p,
        )
        .unwrap();
        unchanged(d.path(), true);
    }
    assert_eq!(outside_before, inventory(outside.path()));
}

#[test]
fn i01_non_directory_ancestor_and_invalid_root_are_fail_closed() {
    let d = TempDirGuard::new("i01-invalid").unwrap();
    put(d.path(), "etc");
    unchanged(d.path(), true);
    assert!(preflight_install(Path::new("relative-root")).is_err());
    assert!(preflight_install(&d.path().join("missing")).is_err());
    assert!(preflight_install(&d.path().join("etc")).is_err());
    assert!(preflight_install(&d.path().join("../")).is_err());
}

#[test]
fn i01_metadata_permission_error_and_symlink_root_are_refused() {
    assert_ne!(
        unsafe { libc::geteuid() },
        0,
        "permission fixture requires normal user"
    );
    let d = TempDirGuard::new("i01-metadata-error").unwrap();
    put(d.path(), "etc/sentinel");
    let before = inventory(d.path());
    let etc = d.path().join("etc");
    let original_mode = fs::metadata(&etc).unwrap().permissions();
    fs::set_permissions(&etc, fs::Permissions::from_mode(0o000)).unwrap();
    let result = preflight_install(d.path());
    let unchanged_mode = fs::symlink_metadata(&etc).unwrap().permissions().mode() & 0o777 == 0;
    // Restore fixture access before assertions and cleanup, including on failure.
    fs::set_permissions(&etc, original_mode).unwrap();
    assert!(
        result
            .expect_err("unreadable ancestor must be refused")
            .contains("cannot verify")
    );
    assert!(unchanged_mode);
    assert_eq!(before, inventory(d.path()));

    let parent = TempDirGuard::new("i01-symlink-root").unwrap();
    let link = parent.path().join("root");
    symlink(d.path(), &link).unwrap();
    unchanged(&link, true);
    assert_eq!(before, inventory(d.path()));
}

#[test]
fn i01_helper_socket_is_refused_without_disconnecting_or_unlinking() {
    let d = TempDirGuard::new("i01-helper-socket").unwrap();
    let p = d.path().join("run/upd/helper.sock");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    let listener = UnixListener::bind(&p).unwrap();
    unchanged(d.path(), true);
    assert!(listener.local_addr().is_ok());
    assert!(p.exists());
}

#[test]
fn i01_cm_and_legacy_test_environment_cannot_bypass_production_preflight() {
    let _serial = isolation_lock();
    let legacy = TempDirGuard::new("i01-legacy-env").unwrap();
    let clean = TempDirGuard::new("i01-clean-env").unwrap();
    put(legacy.path(), "etc/upd.conf");
    let mut env = EnvGuard::new();
    for key in [
        "CM_STATE_DIR",
        "UPD_STATE_DIR",
        "CM_CONF",
        "UPD_CONF",
        "CM_VPN_HOME",
        "UPD_VPN_HOME",
        "CM_VPN_ETC",
        "UPD_VPN_ETC",
    ] {
        env.set(key, clean.path());
    }
    unchanged(legacy.path(), true);
    unchanged(clean.path(), false);
}

#[test]
fn c17a_clean_root_and_unprivileged_readers_pass_without_writes() {
    let clean = TempDirGuard::new("c17a-clean-startup").unwrap();
    let before = inventory(clean.path());
    preflight_startup(clean.path(), "lang", true).expect("clean root startup is allowed");
    assert_eq!(before, inventory(clean.path()));

    let cm_only = TempDirGuard::new("c17a-cm-only-startup").unwrap();
    put(cm_only.path(), "etc/cm.conf");
    put(cm_only.path(), "var/lib/cm/vpn/config.yaml");
    let before = inventory(cm_only.path());
    preflight_startup(cm_only.path(), "status", true).expect("CM-only root startup is allowed");
    assert_eq!(before, inventory(cm_only.path()));

    let legacy = TempDirGuard::new("c17a-user-reader").unwrap();
    put(legacy.path(), "etc/upd.conf");
    let before = inventory(legacy.path());
    preflight_startup(legacy.path(), "lang", false).expect("unprivileged reader is allowed");
    assert_eq!(before, inventory(legacy.path()));
}

#[test]
fn c17a_privileged_startup_refuses_every_dispatch_before_any_write() {
    let commands = [
        "tui", "update", "check", "list", "status", "mirrors", "news", "snapshots",
        "restart", "clean", "merge", "vpn", "aur", "lang", "install", "uninstall",
        "reconcile", "auto", "net", "notify", "helper", "gen-files", "unknown",
    ];
    for command in commands {
        let d = TempDirGuard::new("c17a-root-dispatch").unwrap();
        put(d.path(), "etc/upd.conf");
        let before = inventory(d.path());
        let error = preflight_startup(d.path(), command, true)
            .expect_err("privileged dispatch beside legacy state must refuse");
        assert!(error.contains("blocked") && error.contains("UPD"), "{command}: {error}");
        assert_eq!(before, inventory(d.path()), "{command} changed fixture contents");
    }
}

#[test]
fn c17a_var_lib_only_legacy_and_unprivileged_install_refuse_without_writes() {
    let d = TempDirGuard::new("c17a-state-only-legacy").unwrap();
    put(d.path(), "var/lib/upd/vpn/state");
    let before = inventory(d.path());
    assert!(preflight_startup(d.path(), "status", true).is_err());
    assert_eq!(before, inventory(d.path()));
    assert!(preflight_startup(d.path(), "install", false).is_err());
    assert_eq!(before, inventory(d.path()));
}

#[test]
fn c17a_cm_and_upd_environment_overrides_do_not_bypass_root_guard() {
    let _serial = isolation_lock();
    let d = TempDirGuard::new("c17a-env-guard").unwrap();
    let elsewhere = TempDirGuard::new("c17a-env-target").unwrap();
    put(d.path(), "etc/upd.conf");
    let before = inventory(d.path());
    let mut env = EnvGuard::new();
    for key in ["CM_CONF", "UPD_CONF", "CM_STATE_DIR", "UPD_STATE_DIR"] {
        env.set(key, elsewhere.path());
    }
    for command in ["lang", "status", "unknown"] {
        assert!(preflight_startup(d.path(), command, true).is_err(), "{command}");
    }
    assert_eq!(before, inventory(d.path()));
    assert!(inventory(elsewhere.path()).len() == 1, "environment target should stay empty");
}

#[test]
fn c17a_unsafe_legacy_ancestor_is_refused_for_root_startup() {
    let outside = TempDirGuard::new("c17a-outside").unwrap();
    put(outside.path(), "sentinel");
    let d = TempDirGuard::new("c17a-unsafe-root").unwrap();
    symlink(outside.path().join("missing"), d.path().join("etc")).unwrap();
    let before = inventory(d.path());
    let outside_before = inventory(outside.path());
    assert!(preflight_startup(d.path(), "status", true).is_err());
    assert_eq!(before, inventory(d.path()));
    assert_eq!(outside_before, inventory(outside.path()));
}
