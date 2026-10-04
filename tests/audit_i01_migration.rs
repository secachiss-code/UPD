//! Independent disposable-filesystem checks for the I01 migration executor.
//! These tests never target the host root and use only files owned by this uid.
use cm::migration::transaction::{
    Change, Content, Executor, Observer, Plan, Service, ServiceChange, ServiceState, Services,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct Scratch(PathBuf);
impl Scratch {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("cm-i01-{label}-{}-{nonce}", std::process::id()));
        fs::create_dir(&p).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
        Self(p)
    }
    fn root(&self) -> &Path {
        &self.0
    }
    fn at(&self, logical: &str) -> PathBuf {
        self.0.join(logical.trim_start_matches('/'))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone)]
struct FakeServices {
    path: PathBuf,
    states: BTreeMap<String, ServiceState>,
    fail_set: bool,
    fail_reload: bool,
    set_calls: usize,
    reload_calls: usize,
}
impl FakeServices {
    fn new(path: PathBuf) -> Self {
        let states = fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            path,
            states,
            fail_set: false,
            fail_reload: false,
            set_calls: 0,
            reload_calls: 0,
        }
    }
    fn persist(&self) {
        fs::write(&self.path, serde_json::to_vec(&self.states).unwrap()).unwrap();
    }
    fn seed(&mut self, s: &Service, state: ServiceState) {
        self.states.insert(s.unit.clone(), state);
        self.persist();
    }
}
impl Services for FakeServices {
    fn inspect(&mut self, s: &Service) -> Result<ServiceState, String> {
        Ok(self.states.get(&s.unit).cloned().unwrap_or_default())
    }
    fn set(&mut self, s: &Service, state: &ServiceState) -> Result<(), String> {
        self.set_calls += 1;
        if self.fail_set {
            return Err("injected service set failure".into());
        }
        self.states.insert(s.unit.clone(), state.clone());
        self.persist();
        Ok(())
    }
    fn reload(&mut self) -> Result<(), String> {
        self.reload_calls += 1;
        if self.fail_reload {
            Err("injected reload failure".into())
        } else {
            Ok(())
        }
    }
}
struct NoFault;
impl Observer for NoFault {
    fn boundary(&mut self, _: &str) -> Result<(), String> {
        Ok(())
    }
}
struct Trace(Vec<String>);
impl Observer for Trace {
    fn boundary(&mut self, name: &str) -> Result<(), String> {
        self.0.push(name.into());
        Ok(())
    }
}
struct FailAt(String);
impl Observer for FailAt {
    fn boundary(&mut self, name: &str) -> Result<(), String> {
        if name == self.0 {
            Err(format!("fault:{name}"))
        } else {
            Ok(())
        }
    }
}
struct ExitAt(String);
impl Observer for ExitAt {
    fn boundary(&mut self, name: &str) -> Result<(), String> {
        if name == self.0 {
            std::process::exit(86)
        }
        Ok(())
    }
}

fn tree_signature(path: &Path) -> Vec<(PathBuf, u32, u32, u32, String)> {
    fn walk(path: &Path, base: &Path, out: &mut Vec<(PathBuf, u32, u32, u32, String)>) {
        let Ok(meta) = fs::symlink_metadata(path) else {
            return;
        };
        let value = if meta.file_type().is_symlink() {
            format!("link:{:?}", fs::read_link(path).unwrap())
        } else if meta.is_file() {
            format!("file:{:x?}", fs::read(path).unwrap())
        } else {
            "dir".to_owned()
        };
        out.push((
            path.strip_prefix(base).unwrap_or(path).to_path_buf(),
            meta.mode() & 0o7777,
            meta.uid(),
            meta.gid(),
            value,
        ));
        if meta.is_dir() {
            let mut children: Vec<_> = fs::read_dir(path)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            children.sort();
            for child in children {
                walk(&child, base, out)
            }
        }
    }
    let mut out = Vec::new();
    walk(path, path, &mut out);
    out
}

fn fixture(label: &str) -> (Scratch, Plan, PathBuf, Service, Service) {
    let d = Scratch::new(label);
    let source = d.at("var/lib/upd");
    fs::create_dir_all(source.join("vpn")).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(source.join("vpn"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(source.join("vpn/config.yaml"), b"credentials: sentinel\n").unwrap();
    fs::set_permissions(
        source.join("vpn/config.yaml"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let old = Service {
        unit: "upd-vpn.service".into(),
        user: None,
        global: false,
    };
    let new = Service {
        unit: "cm-vpn.service".into(),
        user: None,
        global: false,
    };
    let states = d.at("fake-services.json");
    let plan = Plan {
        changes: vec![
            Change {
                path: PathBuf::from("/var/lib/cm"),
                content: Content::Tree(PathBuf::from("/var/lib/upd")),
            },
            Change {
                path: PathBuf::from("/var/lib/upd"),
                content: Content::Absent,
            },
        ],
        services: vec![
            ServiceChange {
                service: old.clone(),
                after: ServiceState::default(),
                before_files: true,
            },
            ServiceChange {
                service: new.clone(),
                after: ServiceState {
                    enabled: true,
                    active: true,
                },
                before_files: false,
            },
        ],
        lock_paths: vec![],
    };
    (d, plan, states, old, new)
}

fn manual_policy_fixture(label: &str) -> (Scratch, BTreeMap<PathBuf, (Vec<u8>, u32)>) {
    let d = Scratch::new(label);
    let binary = d.at("usr/local/bin/upd");
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    let pinned = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dist/upd-linux-amd64");
    fs::copy(pinned, &binary).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();

    let conf = d.at("etc/upd.conf");
    fs::create_dir_all(conf.parent().unwrap()).unwrap();
    fs::write(&conf, b"credential: synthetic-raw-config\n").unwrap();
    fs::set_permissions(&conf, fs::Permissions::from_mode(0o600)).unwrap();
    let credential = d.at("var/lib/upd/vpn/config.yaml");
    fs::create_dir_all(credential.parent().unwrap()).unwrap();
    fs::set_permissions(
        credential.parent().unwrap(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::write(&credential, b"password: synthetic-raw-vpn\n").unwrap();
    fs::set_permissions(&credential, fs::Permissions::from_mode(0o600)).unwrap();

    let mut replacements = BTreeMap::new();
    replacements.insert(
        PathBuf::from("/usr/local/bin/cm"),
        (b"synthetic current cm executable".to_vec(), 0o755),
    );
    (d, replacements)
}

#[test]
fn manual_plan_prevalidates_nested_data_without_writing_journal() {
    let (clean, replacements) = manual_policy_fixture("manual-plan-clean");
    let before = tree_signature(clean.root());
    let mut clean_services = FakeServices::new(clean.at("services.json"));
    cm::migration::manual::plan(clean.root(), replacements, &mut clean_services, None).unwrap();
    assert_eq!(before, tree_signature(clean.root()));
    assert!(!clean.at("var/lib/cm-migration").exists());

    for hazard in ["nested-symlink", "group-writable"] {
        let (d, replacements) = manual_policy_fixture(hazard);
        let credential = d.at("var/lib/upd/vpn/config.yaml");
        let original = b"password: synthetic-raw-vpn\n";
        let unsafe_entry = d.at(&format!("var/lib/upd/vpn/{hazard}"));
        if hazard == "nested-symlink" {
            std::os::unix::fs::symlink("missing-synthetic-target", &unsafe_entry).unwrap();
        } else {
            fs::write(&unsafe_entry, b"synthetic writable-entry sentinel").unwrap();
            fs::set_permissions(&unsafe_entry, fs::Permissions::from_mode(0o664)).unwrap();
        }
        let before = tree_signature(d.root());
        let mut services = FakeServices::new(d.at("services.json"));
        assert!(cm::migration::manual::plan(d.root(), replacements, &mut services, None).is_err());
        assert_eq!(before, tree_signature(d.root()));
        assert_eq!(fs::read(&credential).unwrap(), original);
        assert!(!d.at("var/lib/cm-migration").exists());
    }
}

#[test]
fn shared_plan_validator_success_is_read_only() {
    let (d, plan, _, _, _) = fixture("validate-plan-read-only");
    let before = tree_signature(d.root());
    Executor::new(d.root())
        .unwrap()
        .validate_plan(&plan)
        .unwrap();
    assert_eq!(before, tree_signature(d.root()));
    assert!(!d.at("var/lib/cm-migration").exists());
}

#[test]
fn success_copies_bytes_owner_mode_and_service_state() {
    let (d, plan, state_path, old, new) = fixture("success");
    let exec = Executor::new(d.root()).unwrap();
    let before = ServiceState {
        enabled: true,
        active: true,
    };
    let mut sv = FakeServices::new(state_path);
    sv.seed(&old, before.clone());
    exec.execute(&plan, &mut sv, &mut NoFault).unwrap();
    let target = d.at("var/lib/cm/vpn/config.yaml");
    let meta = fs::metadata(&target).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"credentials: sentinel\n");
    assert_eq!(meta.uid(), unsafe { libc::getuid() });
    assert_eq!(meta.mode() & 0o777, 0o600);
    assert!(!d.at("var/lib/upd").exists());
    assert_eq!(sv.inspect(&old).unwrap(), ServiceState::default());
    assert_eq!(sv.inspect(&new).unwrap(), before);
    assert!(!exec.pending().unwrap());
}

#[test]
fn rejects_foreign_ancestor_and_busy_flock_without_mutation() {
    let (d, mut plan, state_path, _, _) = fixture("negative");
    let outside = d.at("outside");
    fs::create_dir(&outside).unwrap();
    let etc = d.at("etc");
    std::os::unix::fs::symlink(&outside, &etc).unwrap();
    plan.changes[0].path = PathBuf::from("/etc/cm.conf");
    plan.changes[0].content = Content::File(b"new config".to_vec(), 0o600);
    let exec = Executor::new(d.root()).unwrap();
    let mut sv = FakeServices::new(state_path);
    let mut no = NoFault;
    assert!(exec.execute(&plan, &mut sv, &mut no).is_err());
    assert!(etc.symlink_metadata().unwrap().file_type().is_symlink());
    assert!(d.at("var/lib/upd/vpn/config.yaml").exists());

    let (d, plan, state_path, _, _) = fixture("busy");
    let lock = d.at("var/lib/cm-migration/transaction.lock");
    fs::create_dir_all(lock.parent().unwrap()).unwrap();
    fs::set_permissions(lock.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(&lock, b"").unwrap();
    fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock)
        .unwrap();
    assert_eq!(
        unsafe {
            libc::flock(
                std::os::fd::AsRawFd::as_raw_fd(&held),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        },
        0
    );
    let exec = Executor::new(d.root()).unwrap();
    let mut sv = FakeServices::new(state_path);
    assert!(exec.execute(&plan, &mut sv, &mut no).is_err());
    assert!(d.at("var/lib/upd/vpn/config.yaml").exists());

    let (d, mut plan, state_path, _, _) = fixture("legacy-busy");
    let legacy_lock = d.at("var/lib/upd/.lock");
    fs::write(&legacy_lock, b"legacy lock sentinel").unwrap();
    fs::set_permissions(&legacy_lock, fs::Permissions::from_mode(0o600)).unwrap();
    plan.lock_paths = vec![PathBuf::from("/var/lib/upd/.lock")];
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&legacy_lock)
        .unwrap();
    assert_eq!(
        unsafe {
            libc::flock(
                std::os::fd::AsRawFd::as_raw_fd(&held),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        },
        0
    );
    let before = tree_signature(&d.at("var/lib/upd"));
    let inode = fs::metadata(&legacy_lock).unwrap().ino();
    let exec = Executor::new(d.root()).unwrap();
    let mut sv = FakeServices::new(state_path);
    assert!(exec.execute(&plan, &mut sv, &mut NoFault).is_err());
    assert_eq!(before, tree_signature(&d.at("var/lib/upd")));
    assert_eq!(fs::metadata(&legacy_lock).unwrap().ino(), inode);
}

#[test]
fn rejects_final_symlink_target_and_overlapping_plan_before_journaling() {
    let d = Scratch::new("final-symlink");
    let outside = d.at("outside.conf");
    fs::write(&outside, b"foreign sentinel").unwrap();
    let target = d.at("etc/cm.conf");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&outside, &target).unwrap();
    let plan = Plan {
        changes: vec![Change {
            path: PathBuf::from("/etc/cm.conf"),
            content: Content::File(b"replace".to_vec(), 0o600),
        }],
        services: vec![],
        lock_paths: vec![],
    };
    let exec = Executor::new(d.root()).unwrap();
    let mut sv = FakeServices::new(d.at("services.json"));
    assert!(exec.execute(&plan, &mut sv, &mut NoFault).is_err());
    assert!(target.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(fs::read(outside).unwrap(), b"foreign sentinel");
    assert!(!d.at("var/lib/cm-migration").exists());

    let d = Scratch::new("overlap");
    let plan = Plan {
        changes: vec![
            Change {
                path: PathBuf::from("/etc/cm"),
                content: Content::File(vec![], 0o600),
            },
            Change {
                path: PathBuf::from("/etc/cm/conf"),
                content: Content::File(vec![], 0o600),
            },
        ],
        services: vec![],
        lock_paths: vec![],
    };
    let exec = Executor::new(d.root()).unwrap();
    let mut sv = FakeServices::new(d.at("services.json"));
    assert!(exec.execute(&plan, &mut sv, &mut NoFault).is_err());
    assert!(!d.at("etc/cm").exists());
    assert!(!d.at("var/lib/cm-migration").exists());
}

#[test]
fn manual_policy_applies_pinned_binary_layout_with_exact_references_and_replacements() {
    let d = Scratch::new("manual-happy");
    let binary = d.at("usr/local/bin/upd");
    fs::create_dir_all(binary.parent().unwrap()).unwrap();
    let pinned = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dist/upd-linux-amd64");
    fs::copy(&pinned, &binary).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let conf = d.at("etc/upd.conf");
    fs::create_dir_all(conf.parent().unwrap()).unwrap();
    fs::write(&conf, b"language: en\nprivate: fixture\n").unwrap();
    fs::set_permissions(&conf, fs::Permissions::from_mode(0o600)).unwrap();
    let data = d.at("var/lib/upd/vpn/config.yaml");
    fs::create_dir_all(data.parent().unwrap()).unwrap();
    fs::set_permissions(data.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(&data, b"credential: fixture-secret\n").unwrap();
    fs::set_permissions(&data, fs::Permissions::from_mode(0o600)).unwrap();
    let mut refs = cm::migration::manual::references(None);
    for (p, b) in cm::migration::manual::references(Some("/etc/pacman.d/mirrorlist")) {
        refs.entry(p).or_insert(b);
    }
    for (path, bytes) in &refs {
        let actual = d.at(&path.to_string_lossy());
        fs::create_dir_all(actual.parent().unwrap()).unwrap();
        fs::write(&actual, bytes).unwrap();
        fs::set_permissions(&actual, fs::Permissions::from_mode(0o644)).unwrap();
    }
    let enabled = Service {
        unit: "upd-auto.timer".into(),
        user: None,
        global: false,
    };
    let inactive_vpn = Service {
        unit: "upd-vpn.service".into(),
        user: None,
        global: false,
    };
    let enabled_link = d.at("etc/systemd/system/timers.target.wants/upd-auto.timer");
    fs::create_dir_all(enabled_link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink("/etc/systemd/system/upd-auto.timer", &enabled_link).unwrap();
    let mut replacements = BTreeMap::new();
    replacements.insert(
        PathBuf::from("/usr/local/bin/cm"),
        (b"current cm binary fixture".to_vec(), 0o755),
    );
    for path in refs.keys() {
        replacements.insert(
            cm::migration::manual::destination(path).unwrap(),
            (
                format!("replacement for {}", path.display()).into_bytes(),
                0o644,
            ),
        );
    }
    let states = d.at("services.json");
    let mut sv = FakeServices::new(states);
    sv.seed(
        &enabled,
        ServiceState {
            enabled: true,
            active: false,
        },
    );
    sv.seed(
        &inactive_vpn,
        ServiceState {
            enabled: true,
            active: false,
        },
    );
    let plan = cm::migration::manual::plan(d.root(), replacements.clone(), &mut sv, None).unwrap();
    let exec = Executor::new(d.root()).unwrap();
    let mut trace = Trace(Vec::new());
    exec.execute(&plan, &mut sv, &mut trace)
        .unwrap_or_else(|error| panic!("{error}; last boundary {:?}", trace.0.last()));
    assert_eq!(
        fs::read(d.at("etc/cm.conf")).unwrap(),
        b"language: en\nprivate: fixture\n"
    );
    assert_eq!(
        fs::metadata(d.at("etc/cm.conf")).unwrap().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::read(d.at("var/lib/cm/vpn/config.yaml")).unwrap(),
        b"credential: fixture-secret\n"
    );
    assert_eq!(
        fs::metadata(d.at("var/lib/cm/vpn/config.yaml"))
            .unwrap()
            .mode()
            & 0o777,
        0o600
    );
    assert!(!d.at("etc/upd.conf").exists());
    assert!(!d.at("var/lib/upd").exists());
    assert!(!binary.exists());
    assert!(!enabled_link.exists());
    for (path, (bytes, _)) in replacements {
        assert_eq!(
            fs::read(d.at(&path.to_string_lossy())).unwrap(),
            bytes,
            "{}",
            path.display()
        );
    }
    let new = Service {
        unit: "cm-auto.timer".into(),
        user: None,
        global: false,
    };
    assert_eq!(
        sv.inspect(&new).unwrap(),
        ServiceState {
            enabled: true,
            active: false
        }
    );
    let enabled_new = d.at("etc/systemd/system/timers.target.wants/cm-auto.timer");
    assert_eq!(
        fs::read_link(enabled_new).unwrap(),
        PathBuf::from("/etc/systemd/system/cm-auto.timer")
    );
    let vpn = Service {
        unit: "cm-vpn.service".into(),
        user: None,
        global: false,
    };
    assert_eq!(
        sv.inspect(&vpn).unwrap(),
        ServiceState {
            enabled: true,
            active: false
        }
    );
    let vpn_link = d.at("etc/systemd/system/multi-user.target.wants/cm-vpn.service");
    assert_eq!(
        fs::read_link(vpn_link).unwrap(),
        PathBuf::from("/etc/systemd/system/cm-vpn.service")
    );
    assert!(exec.already_committed(&mut sv).unwrap());
}

#[test]
fn observer_failures_roll_back_each_durable_boundary() {
    for boundary in [
        "prepared",
        "locked",
        "before-service-before-0",
        "after-service-before-0",
        "quiesced",
        "before-files-0",
        "after-files-0",
        "before-files-1",
        "after-files-1",
        "before-reload",
        "after-reload",
        "before-service-after-1",
        "after-service-after-1",
        "before-commit",
    ] {
        let (d, plan, state_path, old, new) = fixture("boundary");
        let exec = Executor::new(d.root()).unwrap();
        let original = ServiceState {
            enabled: true,
            active: true,
        };
        let mut sv = FakeServices::new(state_path);
        sv.seed(&old, original.clone());
        let result = exec.execute(&plan, &mut sv, &mut FailAt(boundary.into()));
        assert!(
            result.is_err(),
            "boundary {boundary} unexpectedly succeeded"
        );
        assert_eq!(
            fs::read(d.at("var/lib/upd/vpn/config.yaml")).unwrap(),
            b"credentials: sentinel\n",
            "boundary {boundary}"
        );
        assert!(!d.at("var/lib/cm").exists(), "boundary {boundary}");
        assert_eq!(sv.inspect(&old).unwrap(), original, "boundary {boundary}");
        assert_eq!(
            sv.inspect(&new).unwrap(),
            ServiceState::default(),
            "boundary {boundary}"
        );
    }
}

#[test]
fn crash_worker() {
    let Ok(root) = std::env::var("CM_I01_CRASH_ROOT") else {
        return;
    };
    let state = PathBuf::from(std::env::var("CM_I01_SERVICES").unwrap());
    let (plan, _, _) = subprocess_plan(Path::new(&root));
    let exec = Executor::new(Path::new(&root)).unwrap();
    let mut sv = FakeServices::new(state);
    let _ = exec.execute(&plan, &mut sv, &mut ExitAt("after-files-0".into()));
    panic!("fault boundary was not reached");
}

#[test]
fn killed_process_recovers_from_persistent_journal_in_new_process() {
    let (d, _plan, state_path, old, new) = fixture("crash");
    let mut sv = FakeServices::new(state_path.clone());
    let original = ServiceState {
        enabled: true,
        active: true,
    };
    sv.seed(&old, original.clone());
    let exe = std::env::current_exe().unwrap();
    let status = Command::new(exe)
        .args(["--exact", "crash_worker", "--nocapture"])
        .env("CM_I01_CRASH_ROOT", d.root())
        .env("CM_I01_SERVICES", &state_path)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
    assert!(Executor::new(d.root()).unwrap().pending().unwrap());
    assert!(Executor::new(d.root()).unwrap().startup_allowed().is_err());
    assert!(d.at("var/lib/cm/vpn/config.yaml").exists());
    let mut recovered = FakeServices::new(state_path);
    Executor::new(d.root())
        .unwrap()
        .recover(&mut recovered)
        .unwrap();
    Executor::new(d.root()).unwrap().startup_allowed().unwrap();
    assert_eq!(
        fs::read(d.at("var/lib/upd/vpn/config.yaml")).unwrap(),
        b"credentials: sentinel\n"
    );
    assert!(!d.at("var/lib/cm").exists());
    assert_eq!(recovered.inspect(&old).unwrap(), original);
    assert_eq!(recovered.inspect(&new).unwrap(), ServiceState::default());
}

#[test]
fn recovery_refuses_tampered_backup_and_foreign_target_edits() {
    let (d, _plan, state_path, _, _) = fixture("tamper");
    let exec = Executor::new(d.root()).unwrap();
    let mut sv = FakeServices::new(state_path);
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_worker", "--nocapture"])
        .env("CM_I01_CRASH_ROOT", d.root())
        .env("CM_I01_SERVICES", &sv.path)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
    let backup = fs::read_dir(d.at("var/lib/cm-migration"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("blob-old-")
        })
        .unwrap();
    fs::write(backup, b"tampered").unwrap();
    let before = (
        tree_signature(&d.at("var/lib/upd")),
        tree_signature(&d.at("var/lib/cm")),
    );
    assert!(exec.recover(&mut sv).is_err());
    assert_eq!(
        before,
        (
            tree_signature(&d.at("var/lib/upd")),
            tree_signature(&d.at("var/lib/cm"))
        )
    );

    let (d, _, state_path, _, _) = fixture("foreign");
    let mut sv = FakeServices::new(state_path);
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_worker", "--nocapture"])
        .env("CM_I01_CRASH_ROOT", d.root())
        .env("CM_I01_SERVICES", &sv.path)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
    fs::write(d.at("var/lib/cm/foreign.txt"), b"keep me").unwrap();
    let before = (
        tree_signature(&d.at("var/lib/upd")),
        tree_signature(&d.at("var/lib/cm")),
    );
    assert!(Executor::new(d.root()).unwrap().recover(&mut sv).is_err());
    assert_eq!(
        before,
        (
            tree_signature(&d.at("var/lib/upd")),
            tree_signature(&d.at("var/lib/cm"))
        )
    );
}

#[test]
fn incomplete_rollback_can_be_retried_without_losing_backup() {
    let (d, _plan, state_path, old, new) = fixture("retry");
    let mut sv = FakeServices::new(state_path.clone());
    let original = ServiceState {
        enabled: true,
        active: true,
    };
    sv.seed(&old, original.clone());
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_worker", "--nocapture"])
        .env("CM_I01_CRASH_ROOT", d.root())
        .env("CM_I01_SERVICES", &state_path)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
    let exec = Executor::new(d.root()).unwrap();
    let mut failing = FakeServices::new(state_path.clone());
    failing.fail_set = true;
    assert!(exec.recover(&mut failing).is_err());
    assert!(exec.pending().unwrap());
    let mut retry = FakeServices::new(state_path);
    exec.recover(&mut retry).unwrap();
    assert_eq!(
        fs::read(d.at("var/lib/upd/vpn/config.yaml")).unwrap(),
        b"credentials: sentinel\n"
    );
    assert!(!d.at("var/lib/cm").exists());
    assert_eq!(retry.inspect(&old).unwrap(), original);
    assert_eq!(retry.inspect(&new).unwrap(), ServiceState::default());
}

#[test]
fn recovery_cleans_only_verified_partial_staging_prefixes() {
    for (label, staging, expect_success) in [
        ("partial-stage", b"".as_slice(), true),
        (
            "foreign-stage",
            b"unrelated foreign staging data".as_slice(),
            false,
        ),
    ] {
        let (d, _plan, state_path, _, _) = fixture(label);
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "crash_worker", "--nocapture"])
            .env("CM_I01_CRASH_ROOT", d.root())
            .env("CM_I01_SERVICES", &state_path)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(86));
        let temp = d.at("var/lib/cm/vpn/.config.yaml.cm-migration.tmp");
        fs::write(&temp, staging).unwrap();
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600)).unwrap();
        let mut sv = FakeServices::new(state_path);
        let result = Executor::new(d.root()).unwrap().recover(&mut sv);
        if expect_success {
            result.unwrap();
            assert!(!temp.exists());
            assert!(d.at("var/lib/upd/vpn/config.yaml").exists());
        } else {
            assert!(result.is_err());
            assert_eq!(fs::read(&temp).unwrap(), staging);
            assert!(d.at("var/lib/cm/vpn/config.yaml").exists());
        }
    }
}

#[test]
fn recovery_preserves_unrecognized_journal_staging_contents() {
    let (d, _plan, state_path, _, _) = fixture("journal-stage");
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_worker", "--nocapture"])
        .env("CM_I01_CRASH_ROOT", d.root())
        .env("CM_I01_SERVICES", &state_path)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));
    let staged = d.at("var/lib/cm-migration/.journal.json.cm-migration.tmp");
    fs::write(&staged, b"unrelated journal staging contents").unwrap();
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o600)).unwrap();
    let mut sv = FakeServices::new(state_path);
    let err = Executor::new(d.root())
        .unwrap()
        .recover(&mut sv)
        .unwrap_err();
    assert!(err.contains("journal staging"), "{err}");
    assert_eq!(
        fs::read(&staged).unwrap(),
        b"unrelated journal staging contents"
    );
    assert!(d.at("var/lib/cm/vpn/config.yaml").exists());
    assert!(Executor::new(d.root()).unwrap().pending().unwrap());
}

#[test]
fn committed_plan_repeat_verifies_without_recopying_or_changing_bytes() {
    let (d, plan, state_path, old, _new) = fixture("repeat");
    let exec = Executor::new(d.root()).unwrap();
    let mut sv = FakeServices::new(state_path);
    sv.seed(
        &old,
        ServiceState {
            enabled: true,
            active: true,
        },
    );
    exec.execute(&plan, &mut sv, &mut NoFault).unwrap();
    let before = (
        tree_signature(&d.at("var/lib/upd")),
        tree_signature(&d.at("var/lib/cm")),
    );
    exec.execute(&plan, &mut sv, &mut NoFault).unwrap();
    assert_eq!(
        before,
        (
            tree_signature(&d.at("var/lib/upd")),
            tree_signature(&d.at("var/lib/cm"))
        )
    );
}

#[test]
fn failed_service_and_reload_actions_leave_recoverable_journal() {
    for fail_reload in [false, true] {
        let (d, plan, state_path, old, new) = fixture("service-failure");
        let exec = Executor::new(d.root()).unwrap();
        let original = ServiceState {
            enabled: true,
            active: true,
        };
        let mut sv = FakeServices::new(state_path.clone());
        sv.seed(&old, original.clone());
        if fail_reload {
            sv.fail_reload = true;
        } else {
            sv.fail_set = true;
        }
        assert!(exec.execute(&plan, &mut sv, &mut NoFault).is_err());
        assert!(exec.pending().unwrap());
        sv.fail_set = false;
        sv.fail_reload = false;
        exec.recover(&mut sv).unwrap();
        assert!(!exec.pending().unwrap());
        assert_eq!(
            fs::read(d.at("var/lib/upd/vpn/config.yaml")).unwrap(),
            b"credentials: sentinel\n"
        );
        assert!(!d.at("var/lib/cm").exists());
        assert_eq!(sv.inspect(&old).unwrap(), original);
        assert_eq!(sv.inspect(&new).unwrap(), ServiceState::default());
    }
}

#[test]
fn locked_veto_before_mutations_does_not_call_services_or_replace_source_inodes() {
    let (d, mut plan, state_path, old, _new) = fixture("locked-veto");
    let lock = d.at("var/lib/upd/.lock");
    fs::write(&lock, b"legacy lock").unwrap();
    fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
    plan.lock_paths = vec![PathBuf::from("/var/lib/upd/.lock")];
    let before = tree_signature(&d.at("var/lib/upd"));
    let inode = fs::metadata(&lock).unwrap().ino();
    let mut sv = FakeServices::new(state_path);
    sv.seed(
        &old,
        ServiceState {
            enabled: true,
            active: true,
        },
    );
    sv.fail_set = true;
    sv.fail_reload = true;
    let error = Executor::new(d.root())
        .unwrap()
        .execute(&plan, &mut sv, &mut FailAt("locked".into()))
        .unwrap_err();
    assert!(error.contains("original installation restored"), "{error}");
    assert_eq!(sv.set_calls, 0);
    assert_eq!(sv.reload_calls, 0);
    assert_eq!(before, tree_signature(&d.at("var/lib/upd")));
    assert_eq!(inode, fs::metadata(&lock).unwrap().ino());
    assert!(!Executor::new(d.root()).unwrap().pending().unwrap());
}

#[test]
fn manual_policy_rejects_foreign_and_package_binary_without_touching_sentinels() {
    for legacy_path in ["usr/local/bin/upd", "usr/bin/upd"] {
        let d = Scratch::new("manual-policy");
        let binary = d.at(legacy_path);
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::write(&binary, b"foreign executable sentinel").unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        let before = tree_signature(d.root());
        let mut services = FakeServices::new(d.at("services.json"));
        let result = cm::migration::manual::plan(d.root(), BTreeMap::new(), &mut services, None);
        assert!(
            result.is_err(),
            "{} unexpectedly accepted",
            binary.display()
        );
        assert_eq!(before, tree_signature(d.root()));
    }
}

// A recovery-only copy of fixture plan, kept local so the child can rebuild it.
fn subprocess_plan(root: &Path) -> (Plan, PathBuf, Service) {
    let old = Service {
        unit: "upd-vpn.service".into(),
        user: None,
        global: false,
    };
    let new = Service {
        unit: "cm-vpn.service".into(),
        user: None,
        global: false,
    };
    let state = root.join("fake-services.json");
    (
        Plan {
            changes: vec![
                Change {
                    path: PathBuf::from("/var/lib/cm"),
                    content: Content::Tree(PathBuf::from("/var/lib/upd")),
                },
                Change {
                    path: PathBuf::from("/var/lib/upd"),
                    content: Content::Absent,
                },
            ],
            services: vec![
                ServiceChange {
                    service: old.clone(),
                    after: ServiceState::default(),
                    before_files: true,
                },
                ServiceChange {
                    service: new,
                    after: ServiceState {
                        enabled: true,
                        active: true,
                    },
                    before_files: false,
                },
            ],
            lock_paths: vec![],
        },
        state,
        old,
    )
}
