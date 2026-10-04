//! Durable filesystem/service transaction shared by production and fault fixtures.
//! The caller supplies a validated plan; all backups stay in a private fixed journal.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::os::{
    fd::AsRawFd,
    unix::fs::{symlink, MetadataExt, OpenOptionsExt, PermissionsExt},
};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

pub const JOURNAL: &str = "/var/lib/cm-migration";
type Result<T> = std::result::Result<T, String>;
pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Service {
    pub unit: String,
    pub user: Option<String>,
    pub global: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceState {
    pub enabled: bool,
    pub active: bool,
}
pub trait Services {
    fn inspect(&mut self, service: &Service) -> Result<ServiceState>;
    fn set(&mut self, service: &Service, state: &ServiceState) -> Result<()>;
    fn reload(&mut self) -> Result<()>;
    fn user_sessions(&mut self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    fn assert_unmanaged(&mut self, _paths: &[PathBuf]) -> Result<()> {
        Ok(())
    }
}
/// Test seams observe durable boundaries. An error rolls back; a killed process
/// leaves its journal for a different process to recover.
pub trait Observer {
    fn boundary(&mut self, name: &str) -> Result<()>;
}
pub struct NoFault;
impl Observer for NoFault {
    fn boundary(&mut self, _: &str) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum Content {
    File(Vec<u8>, u32),
    Tree(PathBuf),
    Link(PathBuf),
    Absent,
}
#[derive(Clone, Debug)]
pub struct Change {
    pub path: PathBuf,
    pub content: Content,
}
#[derive(Clone, Debug)]
pub struct ServiceChange {
    pub service: Service,
    pub after: ServiceState,
    pub before_files: bool,
}
#[derive(Clone, Debug)]
pub struct Plan {
    pub changes: Vec<Change>,
    pub services: Vec<ServiceChange>,
    pub lock_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
enum Kind {
    Directory,
    File { backup: String, sha256: String },
    Link(PathBuf),
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Node {
    relative: PathBuf,
    mode: u32,
    uid: u32,
    gid: u32,
    kind: Kind,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
struct Snapshot {
    nodes: Vec<Node>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
    before: Snapshot,
    after: Snapshot,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct SavedService {
    service: Service,
    before: ServiceState,
    after: ServiceState,
    before_files: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
enum Phase {
    Prepared,
    Applying,
    RollingBack,
    Committed,
    RolledBack,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Journal {
    version: u32,
    phase: Phase,
    entries: Vec<Entry>,
    services: Vec<SavedService>,
    parents: Vec<PathBuf>,
    lock_paths: Vec<PathBuf>,
    created_locks: Vec<PathBuf>,
    step: String,
    #[serde(default)]
    mutations_started: bool,
}

pub struct Executor {
    root: PathBuf,
    owner: u32,
}
struct HeldLocks {
    files: Vec<fs::File>,
}
impl HeldLocks {
    fn hold(&mut self, path: &Path, create: bool) -> Result<bool> {
        let existed = fs::symlink_metadata(path).is_ok();
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(create)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(err)?;
        if !file.metadata().map_err(err)?.is_file() {
            return Err("lock is not a regular file".into());
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(format!("migration busy: {}", path.display()));
        }
        self.files.push(file);
        Ok(!existed)
    }
}

impl Executor {
    pub fn new(root: &Path) -> Result<Self> {
        if !root.is_absolute() || root.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err("invalid migration root".into());
        }
        let meta = fs::symlink_metadata(root).map_err(err)?;
        if !meta.is_dir() || meta.mode() & 0o022 != 0 {
            return Err("unsafe migration root".into());
        }
        Ok(Self {
            root: root.to_owned(),
            owner: meta.uid(),
        })
    }
    pub fn owner(&self) -> u32 {
        self.owner
    }
    pub fn path(&self, logical: &Path) -> Result<PathBuf> {
        if !logical.is_absolute()
            || logical == Path::new("/")
            || logical
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err("unsafe transaction path".into());
        }
        let mut actual = self.root.clone();
        let components: Vec<_> = logical
            .strip_prefix("/")
            .map_err(err)?
            .components()
            .collect();
        for (index, component) in components.iter().enumerate() {
            actual.push(component.as_os_str());
            if index + 1 < components.len() {
                match fs::symlink_metadata(&actual) {
                    Ok(m) if m.is_dir() && m.uid() == self.owner && m.mode() & 0o022 == 0 => {}
                    Ok(_) => return Err(format!("unsafe ancestor: {}", logical.display())),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(err(e)),
                }
            }
        }
        Ok(actual)
    }
    fn journal_dir(&self) -> Result<PathBuf> {
        self.path(Path::new(JOURNAL))
    }
    fn check_private(&self, path: &Path) -> Result<()> {
        let m = fs::symlink_metadata(path).map_err(err)?;
        if !m.is_dir() || m.uid() != self.owner || m.mode() & 0o077 != 0 {
            return Err("unsafe migration journal directory".into());
        }
        Ok(())
    }
    fn save(&self, journal: &Journal) -> Result<()> {
        let dir = self.journal_dir()?;
        self.check_private(&dir)?;
        // Journal replacement uses unique O_EXCL staging files. A previous
        // unknown fixed-name temporary is NEVER deleted or overwritten merely
        // because its owner/mode look right. Backups and the durable journal
        // remain available for inspection if such a conflict is found.
        match fs::symlink_metadata(temporary(&dir.join("journal.json"))?) {
            Ok(_) => return Err("unrecognized journal staging file; left untouched".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(err(e)),
        }
        crate::common::atomic_write(
            &dir.join("journal.json"),
            &serde_json::to_vec(journal).map_err(err)?,
            0o600,
        )
        .map_err(err)
    }
    fn load(&self) -> Result<Option<Journal>> {
        let dir = self.journal_dir()?;
        match fs::symlink_metadata(&dir) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(err(e)),
            Ok(_) => self.check_private(&dir)?,
        }
        let path = dir.join("journal.json");
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path);
        let mut file = match file {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(err(e)),
        };
        let m = file.metadata().map_err(err)?;
        if !m.is_file() || m.uid() != self.owner || m.mode() & 0o077 != 0 || m.len() > 32 << 20 {
            return Err("unsafe journal file".into());
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(err)?;
        let j: Journal = serde_json::from_slice(&bytes).map_err(err)?;
        if j.version != 1 {
            return Err("unsupported migration journal version".into());
        }
        for e in &j.entries {
            self.path(&e.path)?;
            if e.path.starts_with(JOURNAL) {
                return Err("journal targets itself".into());
            }
            for snapshot in [&e.before, &e.after] {
                for node in &snapshot.nodes {
                    if node.relative.is_absolute()
                        || node
                            .relative
                            .components()
                            .any(|c| !matches!(c, Component::Normal(_)))
                    {
                        return Err("invalid snapshot path".into());
                    }
                    if let Kind::File { backup, .. } = &node.kind {
                        if Path::new(backup).components().count() != 1
                            || !backup.starts_with("blob-")
                        {
                            return Err("invalid backup reference".into());
                        }
                    }
                }
            }
        }
        Ok(Some(j))
    }
    /// Inspect metadata/digests only. Never follows symlinks or reads special files.
    fn snapshot(&self, path: &Path, store: bool, prefix: &str) -> Result<Snapshot> {
        let actual = self.path(path)?;
        let mut nodes = Vec::new();
        self.walk(&actual, Path::new(""), &mut nodes, store, prefix)?;
        Ok(Snapshot { nodes })
    }
    fn walk(
        &self,
        actual: &Path,
        relative: &Path,
        nodes: &mut Vec<Node>,
        store: bool,
        prefix: &str,
    ) -> Result<()> {
        let meta = match fs::symlink_metadata(actual) {
            Ok(m) => m,
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound && relative.as_os_str().is_empty() =>
            {
                return Ok(());
            }
            Err(e) => return Err(err(e)),
        };
        if meta.uid() != self.owner || (!meta.file_type().is_symlink() && meta.mode() & 0o022 != 0)
        {
            return Err("foreign owner or writable migration entry".into());
        }
        let kind = if meta.is_dir() {
            Kind::Directory
        } else if meta.is_file() {
            let mut file = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(actual)
                .map_err(err)?;
            let opened = file.metadata().map_err(err)?;
            if opened.ino() != meta.ino() || opened.dev() != meta.dev() {
                return Err("migration source changed during open".into());
            }
            let backup = format!("blob-{prefix}-{}", nodes.len());
            let mut hasher = Sha256::new();
            let mut output = if store {
                Some(
                    fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(self.journal_dir()?.join(&backup))
                        .map_err(err)?,
                )
            } else {
                None
            };
            let mut buffer = [0u8; 65536];
            loop {
                let n = file.read(&mut buffer).map_err(err)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buffer[..n]);
                if let Some(out) = &mut output {
                    out.write_all(&buffer[..n]).map_err(err)?;
                }
            }
            if let Some(out) = output {
                out.sync_all().map_err(err)?;
            }
            Kind::File {
                backup,
                sha256: hex(&hasher.finalize()),
            }
        } else if meta.file_type().is_symlink() {
            Kind::Link(fs::read_link(actual).map_err(err)?)
        } else {
            return Err("special file in migration data; quiesce the legacy runtime first".into());
        };
        let directory = matches!(kind, Kind::Directory);
        nodes.push(Node {
            relative: relative.to_owned(),
            mode: meta.mode() & 0o7777,
            uid: meta.uid(),
            gid: meta.gid(),
            kind,
        });
        if directory {
            let mut children: Vec<_> = fs::read_dir(actual)
                .map_err(err)?
                .collect::<std::io::Result<Vec<_>>>()
                .map_err(err)?;
            children.sort_by_key(|c| c.file_name());
            for child in children {
                self.walk(
                    &child.path(),
                    &relative.join(child.file_name()),
                    nodes,
                    store,
                    prefix,
                )?;
            }
        }
        Ok(())
    }
    fn equivalent(a: &Snapshot, b: &Snapshot) -> bool {
        a.nodes.len() == b.nodes.len()
            && a.nodes
                .iter()
                .zip(&b.nodes)
                .all(|(a, b)| Self::same_node(a, b))
    }
    fn same_node(a: &Node, b: &Node) -> bool {
        a.relative == b.relative
            && a.mode == b.mode
            && a.uid == b.uid
            && a.gid == b.gid
            && match (&a.kind, &b.kind) {
                (Kind::Directory, Kind::Directory) => true,
                (Kind::Link(a), Kind::Link(b)) => a == b,
                (Kind::File { sha256: a, .. }, Kind::File { sha256: b, .. }) => a == b,
                _ => false,
            }
    }
    fn verify_blobs(&self, j: &Journal) -> Result<()> {
        for e in &j.entries {
            for s in [&e.before, &e.after] {
                for n in &s.nodes {
                    if let Kind::File { backup, sha256 } = &n.kind {
                        let path = self.journal_dir()?.join(backup);
                        let meta = fs::symlink_metadata(&path).map_err(err)?;
                        if !meta.is_file() || meta.uid() != self.owner || meta.mode() & 0o077 != 0 {
                            return Err("unsafe backup".into());
                        }
                        let bytes = fs::read(&path).map_err(err)?;
                        if hex(&Sha256::digest(bytes)) != *sha256 {
                            return Err("backup digest mismatch; recovery retained".into());
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn apply_snapshot(&self, logical: &Path, snapshot: &Snapshot) -> Result<()> {
        let actual = self.path(logical)?;
        match fs::symlink_metadata(&actual) {
            Ok(m) if m.is_dir() => fs::remove_dir_all(&actual).map_err(err)?,
            Ok(_) => fs::remove_file(&actual).map_err(err)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(err(e)),
        }
        if let Some(parent) = actual.parent().filter(|p| p.exists()) {
            sync(parent)?;
        }
        if snapshot.nodes.is_empty() {
            return Ok(());
        }
        if let Some(parent) = actual.parent() {
            fs::create_dir_all(parent).map_err(err)?;
        }
        for n in &snapshot.nodes {
            let target = node_path(&actual, &n.relative);
            match &n.kind {
                Kind::Directory => {
                    fs::create_dir(&target).map_err(err)?;
                    chown(&target, n.uid, n.gid)?;
                    fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).map_err(err)?;
                    sync(&target)?;
                }
                Kind::File { backup, .. } => {
                    let bytes = fs::read(self.journal_dir()?.join(backup)).map_err(err)?;
                    atomic(&target, &bytes, n.mode, n.uid, n.gid)?;
                }
                Kind::Link(link) => {
                    symlink(link, &target).map_err(err)?;
                    chown(&target, n.uid, n.gid)?;
                }
            }
            sync(target.parent().ok_or("missing parent")?)?;
        }
        for n in snapshot
            .nodes
            .iter()
            .rev()
            .filter(|n| matches!(n.kind, Kind::Directory))
        {
            let target = node_path(&actual, &n.relative);
            set_meta(&target, n)?;
            sync(&target)?;
        }
        Ok(())
    }
    pub fn pending(&self) -> Result<bool> {
        Ok(self.load()?.is_some_and(|j| {
            matches!(
                j.phase,
                Phase::Prepared | Phase::Applying | Phase::RollingBack
            )
        }))
    }
    pub fn startup_allowed(&self) -> Result<()> {
        let dir = self.journal_dir()?;
        match fs::symlink_metadata(&dir) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(err(e)),
        }
        self.check_private(&dir)?;
        let path = dir.join("transaction.lock");
        match fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
        {
            Ok(file) => {
                if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } != 0 {
                    return Err("migration in progress; command refused".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(err(e)),
        }
        if self.pending()? {
            return Err("unfinished migration; run cm migration recover".into());
        }
        Ok(())
    }
    /// Validate filesystem generations without creating a journal, backup, or lock.
    pub fn validate_plan(&self, plan: &Plan) -> Result<()> {
        for (i, c) in plan.changes.iter().enumerate() {
            self.path(&c.path)?;
            if c.path.starts_with(JOURNAL) {
                return Err("transaction targets journal".into());
            }
            for other in &plan.changes[..i] {
                if c.path.starts_with(&other.path) || other.path.starts_with(&c.path) {
                    return Err("overlapping transaction paths".into());
                }
            }
            let original = self.snapshot(&c.path, false, "inspect")?;
            if original
                .nodes
                .first()
                .is_some_and(|n| matches!(n.kind, Kind::Link(_)))
                && !matches!(c.content, Content::Absent)
            {
                return Err("symlink migration target refused".into());
            }
            if let Content::Tree(src) = &c.content {
                let s = self.snapshot(src, false, "source")?;
                if s.nodes.is_empty() {
                    return Err("migration data source missing".into());
                }
                if s.nodes.iter().any(|n| matches!(n.kind, Kind::Link(_))) {
                    return Err("symlink in data source".into());
                }
            }
            for n in &original.nodes {
                if matches!(n.kind, Kind::File { .. })
                    && fs::symlink_metadata(temporary(&node_path(
                        &self.path(&c.path)?,
                        &n.relative,
                    ))?)
                    .is_ok()
                {
                    return Err("migration staging path already exists".into());
                }
            }
        }
        Ok(())
    }
    /// Same API in production and fixtures; plan generation itself performs no writes.
    pub fn execute(
        &self,
        plan: &Plan,
        services: &mut dyn Services,
        observer: &mut dyn Observer,
    ) -> Result<()> {
        if let Some(j) = self.load()? {
            return match j.phase { Phase::Committed => self.verify_committed(&j,services), Phase::RolledBack => Err("previous migration rolled back; archive its private journal before a new attempt".into()), _ => Err("unfinished migration: run cm migration recover".into()) };
        }
        // Validate the full filesystem plan BEFORE creating a journal or any lock.
        self.validate_plan(plan)?;
        let dir = self.journal_dir()?;
        if !dir.exists() {
            fs::create_dir_all(&dir).map_err(err)?;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(err)?;
        }
        self.check_private(&dir)?;
        // Refuse a partially prepared backup rather than overwriting it.
        if fs::read_dir(&dir)
            .map_err(err)?
            .any(|e| e.is_ok_and(|e| e.file_name() != "transaction.lock"))
        {
            return Err(
                "incomplete preparation: private backup retained; manual inspection required"
                    .into(),
            );
        }
        let mut locks = HeldLocks { files: Vec::new() };
        locks.hold(&dir.join("transaction.lock"), true)?;
        let mut journal = Journal {
            version: 1,
            phase: Phase::Prepared,
            entries: vec![],
            services: vec![],
            parents: vec![],
            lock_paths: plan.lock_paths.clone(),
            created_locks: vec![],
            step: "prepared".into(),
            mutations_started: false,
        };
        for (i, c) in plan.changes.iter().enumerate() {
            let before = self.snapshot(&c.path, true, &format!("old-{i}"))?;
            let after = match &c.content {
                Content::Absent => Snapshot::default(),
                Content::Tree(src) => self.snapshot(src, true, &format!("new-{i}"))?,
                Content::Link(target) => Snapshot {
                    nodes: vec![Node {
                        relative: PathBuf::new(),
                        mode: 0o777,
                        uid: self.owner,
                        gid: unsafe { libc::getegid() },
                        kind: Kind::Link(target.clone()),
                    }],
                },
                Content::File(bytes, mode) => {
                    let backup = format!("blob-new-{i}-0");
                    let mut f = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(dir.join(&backup))
                        .map_err(err)?;
                    f.write_all(bytes).map_err(err)?;
                    f.sync_all().map_err(err)?;
                    Snapshot {
                        nodes: vec![Node {
                            relative: PathBuf::new(),
                            mode: *mode,
                            uid: self.owner,
                            gid: unsafe { libc::getegid() },
                            kind: Kind::File {
                                backup,
                                sha256: hex(&Sha256::digest(bytes)),
                            },
                        }],
                    }
                }
            };
            let mut parent = c.path.parent();
            while let Some(p) = parent.filter(|p| *p != Path::new("/")) {
                if fs::symlink_metadata(self.path(p)?).is_ok() {
                    break;
                }
                if !journal.parents.contains(&p.to_owned()) {
                    journal.parents.push(p.to_owned());
                }
                parent = p.parent();
            }
            journal.entries.push(Entry {
                path: c.path.clone(),
                before,
                after,
            });
        }
        for e in &journal.entries {
            for n in &e.after.nodes {
                if matches!(n.kind, Kind::File { .. })
                    && fs::symlink_metadata(temporary(&node_path(
                        &self.path(&e.path)?,
                        &n.relative,
                    ))?)
                    .is_ok()
                {
                    return Err("migration staging path conflict".into());
                }
            }
        }
        for c in &plan.services {
            journal.services.push(SavedService {
                service: c.service.clone(),
                before: services.inspect(&c.service)?,
                after: c.after.clone(),
                before_files: c.before_files,
            });
        }
        sync(&dir)?;
        self.save(&journal)?;
        let result: Result<()> = (|| {
            observer.boundary("prepared")?;
            for p in &plan.lock_paths {
                let actual = self.path(p)?;
                if !actual.parent().is_some_and(|p| p.is_dir()) {
                    return Err("lock parent absent; unsupported layout".into());
                }
                if fs::symlink_metadata(&actual).is_err() {
                    journal.created_locks.push(p.clone());
                    self.save(&journal)?;
                } else {
                    let m = fs::symlink_metadata(&actual).map_err(err)?;
                    if !m.is_file() || m.uid() != self.owner || m.mode() & 0o022 != 0 {
                        return Err("foreign migration lock".into());
                    }
                }
                locks.hold(&actual, true)?;
            }
            // Reject any changed generation, including a non-cooperative writer.
            for e in &journal.entries {
                if !Self::equivalent(
                    &self.snapshot_ignoring_new_locks(&e.path, &journal)?,
                    &e.before,
                ) {
                    return Err("source/target generation changed while taking locks".into());
                }
            }
            observer.boundary("locked")?;
            journal.phase = Phase::Applying;
            journal.mutations_started = true;
            self.save(&journal)?;
            for index in 0..journal.services.len() {
                if journal.services[index].before_files {
                    journal.step = format!("service-before-{index}");
                    self.save(&journal)?;
                    observer.boundary(&format!("before-{}", journal.step))?;
                    services.set(
                        &journal.services[index].service,
                        &journal.services[index].after,
                    )?;
                    observer.boundary(&format!("after-{}", journal.step))?;
                }
            }
            observer.boundary("quiesced")?;
            for index in 0..journal.entries.len() {
                journal.step = format!("files-{index}");
                self.save(&journal)?;
                observer.boundary(&format!("before-{}", journal.step))?;
                self.apply_snapshot(&journal.entries[index].path, &journal.entries[index].after)?;
                observer.boundary(&format!("after-{}", journal.step))?;
            }
            journal.step = "reload".into();
            self.save(&journal)?;
            observer.boundary("before-reload")?;
            services.reload()?;
            observer.boundary("after-reload")?;
            for index in 0..journal.services.len() {
                if !journal.services[index].before_files {
                    journal.step = format!("service-after-{index}");
                    self.save(&journal)?;
                    observer.boundary(&format!("before-{}", journal.step))?;
                    services.set(
                        &journal.services[index].service,
                        &journal.services[index].after,
                    )?;
                    observer.boundary(&format!("after-{}", journal.step))?;
                }
            }
            observer.boundary("before-commit")?;
            for p in &journal.created_locks {
                let actual = self.path(p)?;
                if fs::symlink_metadata(&actual).is_ok() {
                    fs::remove_file(&actual).map_err(err)?;
                    if let Some(parent) = actual.parent() {
                        sync(parent)?;
                    }
                }
            }
            journal.phase = Phase::Committed;
            journal.step = "committed".into();
            self.save(&journal)?;
            Ok(())
        })();
        match result {
            Ok(()) => Ok(()),
            Err(failure) => match self.rollback(&mut journal, services) {
                Ok(()) => Err(format!(
                    "migration failed ({failure}); original installation restored"
                )),
                Err(recovery) => Err(format!(
                    "migration failed ({failure}); recovery incomplete ({recovery}); backups retained, run cm migration recover"
                )),
            },
        }
    }
    fn snapshot_ignoring_new_locks(&self, path: &Path, j: &Journal) -> Result<Snapshot> {
        for lock in &j.created_locks {
            match fs::symlink_metadata(self.path(lock)?) {
                Ok(m)
                    if m.is_file()
                        && m.uid() == self.owner
                        && m.mode() & 0o777 == 0o600
                        && m.len() == 0 => {}
                Ok(_) => return Err("created lock changed; recovery refused".into()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(err(e)),
            }
        }
        let mut s = self.snapshot(path, false, "current")?;
        s.nodes.retain(|n| {
            !j.created_locks
                .iter()
                .any(|p| node_path(path, &n.relative) == *p)
        });
        Ok(s)
    }
    fn verify_committed(&self, j: &Journal, services: &mut dyn Services) -> Result<()> {
        for e in &j.entries {
            if !Self::equivalent(&self.snapshot(&e.path, false, "verify")?, &e.after) {
                return Err(
                    "completed migration has changed; refusing to replay credentials".into(),
                );
            }
        }
        for s in &j.services {
            if services.inspect(&s.service)? != s.after {
                return Err("completed migration service state changed; refusing replay".into());
            }
        }
        Ok(())
    }
    pub fn already_committed(&self, services: &mut dyn Services) -> Result<bool> {
        match self.load()? {
            Some(j) if j.phase == Phase::Committed => {
                self.verify_committed(&j, services)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    fn rollback(&self, j: &mut Journal, services: &mut dyn Services) -> Result<()> {
        self.verify_blobs(j)?;
        if !j.mutations_started {
            for e in &j.entries {
                if !Self::equivalent(&self.snapshot_ignoring_new_locks(&e.path, j)?, &e.before) {
                    return Err(
                        "external generation changed before migration; left untouched".into(),
                    );
                }
            }
            // Busy/generation refusals happened before any service or data
            // mutation. Do not restore stale captured service states over a
            // legitimate operation that started during preparation.
            for p in &j.created_locks {
                let path = self.path(p)?;
                match fs::remove_file(&path) {
                    Ok(()) => {
                        sync(path.parent().ok_or("missing lock parent")?)?;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(err(e)),
                }
            }
            j.phase = Phase::RolledBack;
            j.step = "compensated-before-mutations".into();
            return self.save(j);
        }
        // A power loss during file write can leave a partial staging file. Only
        // a prefix of one journaled backup at its exact staging path is ours.
        let mut temporaries = Vec::new();
        let mut candidates = Vec::new();
        for e in &j.entries {
            for n in e.before.nodes.iter().chain(&e.after.nodes) {
                if let Kind::File { backup, .. } = &n.kind {
                    let path = temporary(&node_path(&self.path(&e.path)?, &n.relative))?;
                    if !candidates.contains(&path) {
                        candidates.push(path.clone());
                    }
                    match fs::symlink_metadata(&path) {
                        Ok(m) if m.is_file() && m.uid() == self.owner && m.mode() & 0o022 == 0 => {
                            let bytes = fs::read(&path).map_err(err)?;
                            let expected =
                                fs::read(self.journal_dir()?.join(backup)).map_err(err)?;
                            if expected.starts_with(&bytes) {
                                if !temporaries.contains(&path) {
                                    temporaries.push(path);
                                }
                            }
                        }
                        Ok(_) => return Err("foreign staging file; recovery refused".into()),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(err(e)),
                    }
                }
            }
        }
        for path in candidates {
            if fs::symlink_metadata(&path).is_ok() && !temporaries.contains(&path) {
                return Err("foreign staging contents; recovery refused".into());
            }
        }
        // Check EVERY path before the first compensation. A foreign edit is never deleted.
        for e in &j.entries {
            let current = self.snapshot_ignoring_new_locks(&e.path, j)?;
            if current.nodes.iter().any(|n| {
                let actual = self.path(&e.path).map(|p| node_path(&p, &n.relative));
                if actual.is_ok_and(|p| temporaries.contains(&p)) {
                    return false;
                }
                !e.before.nodes.iter().chain(&e.after.nodes).any(|expected| {
                    Self::same_node(n, expected)
                        || (matches!(n.kind, Kind::Directory)
                            && matches!(expected.kind, Kind::Directory)
                            && n.relative == expected.relative
                            && n.uid == expected.uid
                            && n.gid == expected.gid
                            && n.mode == 0o700)
                })
            }) {
                return Err(
                    "concurrent foreign change; recovery refused without deleting it".into(),
                );
            }
        }
        for path in temporaries {
            fs::remove_file(&path).map_err(err)?;
            sync(path.parent().ok_or("missing staging parent")?)?;
        }
        j.phase = Phase::RollingBack;
        j.step = "rollback".into();
        self.save(j)?;
        for s in j.services.iter().rev().filter(|s| !s.before_files) {
            services.set(&s.service, &ServiceState::default())?;
        }
        for e in j.entries.iter().rev() {
            // Unchanged roots must keep their existing lock-file inodes. This is
            // also what makes a busy-lock refusal a read-only refusal of data.
            if !Self::equivalent(&self.snapshot_ignoring_new_locks(&e.path, j)?, &e.before) {
                self.apply_snapshot(&e.path, &e.before)?;
            }
        }
        let mut parents = j.parents.clone();
        parents.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
        for p in parents {
            match fs::remove_dir(self.path(&p)?) {
                Ok(()) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(e) => return Err(err(e)),
            }
        }
        for p in &j.created_locks {
            match fs::remove_file(self.path(p)?) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(err(e)),
            }
        }
        services.reload()?;
        for s in j.services.iter().rev() {
            services.set(&s.service, &s.before)?;
        }
        j.phase = Phase::RolledBack;
        j.step = "compensated".into();
        self.save(j)
    }
    pub fn recover(&self, services: &mut dyn Services) -> Result<()> {
        let Some(mut j) = self.load()? else {
            return Err("no migration journal".into());
        };
        if matches!(j.phase, Phase::Committed | Phase::RolledBack) {
            return Ok(());
        }
        let mut locks = HeldLocks { files: vec![] };
        locks.hold(&self.journal_dir()?.join("transaction.lock"), true)?;
        // Existing legacy locks preserve inode identity. A removed legacy tree is
        // already behind the journal gate; never recreate it merely to take a lock.
        for p in &j.lock_paths {
            let actual = self.path(p)?;
            if fs::symlink_metadata(&actual).is_ok() {
                locks.hold(&actual, false)?;
            }
        }
        self.rollback(&mut j, services)
    }
}
fn sync(path: &Path) -> Result<()> {
    fs::File::open(path).and_then(|f| f.sync_all()).map_err(err)
}
fn chown(path: &Path, uid: u32, gid: u32) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(err)?;
    if unsafe { libc::lchown(name.as_ptr(), uid, gid) } != 0 {
        return Err(err(std::io::Error::last_os_error()));
    }
    Ok(())
}
fn set_meta(path: &Path, n: &Node) -> Result<()> {
    chown(path, n.uid, n.gid)?;
    fs::set_permissions(path, fs::Permissions::from_mode(n.mode)).map_err(err)
}
fn node_path(base: &Path, relative: &Path) -> PathBuf {
    if relative.as_os_str().is_empty() {
        base.to_owned()
    } else {
        base.join(relative)
    }
}
fn temporary(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or("missing staging filename")?
        .to_str()
        .ok_or("non-UTF8 staging filename")?;
    Ok(path
        .parent()
        .ok_or("missing staging parent")?
        .join(format!(".{name}.cm-migration.tmp")))
}
fn atomic(path: &Path, bytes: &[u8], mode: u32, uid: u32, gid: u32) -> Result<()> {
    let parent = path.parent().ok_or("missing parent")?;
    let temp = temporary(path)?;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temp)
        .map_err(err)?;
    let result = (|| {
        f.write_all(bytes).map_err(err)?;
        if unsafe { libc::fchown(f.as_raw_fd(), uid, gid) } != 0 {
            return Err(err(std::io::Error::last_os_error()));
        }
        f.set_permissions(fs::Permissions::from_mode(mode))
            .map_err(err)?;
        f.sync_all().map_err(err)?;
        fs::rename(&temp, path).map_err(err)?;
        sync(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
