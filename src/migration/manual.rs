//! Exact, opt-in UPD 0.2.7 manual-layout policy. Package/foreign layouts fail closed.
use super::transaction::{
    Change, Content, Executor, Plan, Service, ServiceChange, ServiceState, Services,
};
use super::{inventory_legacy, legacy_v027 as legacy};
use sha2::{Digest, Sha256};
use std::os::unix::fs::MetadataExt;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
type Result<T> = std::result::Result<T, String>;

pub fn references(watch: Option<&str>) -> BTreeMap<PathBuf, Vec<u8>> {
    let bin = "/usr/local/bin/upd";
    let mut map = BTreeMap::new();
    for (name, body) in legacy::system_units(bin, watch) {
        map.insert(
            Path::new("/etc/systemd/system").join(name),
            format!("# Managed by upd\n{body}").into_bytes(),
        );
    }
    for (name, body) in legacy::user_units(bin) {
        map.insert(
            Path::new("/etc/systemd/user").join(name),
            format!("# Managed by upd\n{body}").into_bytes(),
        );
    }
    for (path, body) in [
        ("/etc/pacman.d/hooks/zz-upd.hook", legacy::pacman_hook(bin)),
        ("/etc/apt/apt.conf.d/99upd", legacy::apt_hook(bin)),
        (
            "/etc/NetworkManager/dispatcher.d/90-upd",
            legacy::nm_dispatcher().into(),
        ),
        (
            "/usr/share/polkit-1/actions/io.github.upd.policy",
            legacy::polkit_policy(),
        ),
        (
            "/etc/cron.d/upd",
            format!("# upd\n*/15 * * * * root {bin} net\n17 */6 * * * root {bin} auto\n"),
        ),
    ] {
        map.insert(PathBuf::from(path), body.into_bytes());
    }
    for (path, body) in [
        (
            "/usr/local/share/applications/io.github.upd.desktop",
            include_str!("io.github.upd.desktop"),
        ),
        (
            "/usr/local/share/applications/io.github.upd.Applet.desktop",
            include_str!("io.github.upd.Applet.desktop"),
        ),
        (
            "/usr/local/share/icons/hicolor/scalable/apps/io.github.upd.svg",
            include_str!("io.github.upd.svg"),
        ),
        (
            "/usr/local/share/icons/hicolor/symbolic/apps/io.github.upd-symbolic.svg",
            include_str!("io.github.upd-symbolic.svg"),
        ),
    ] {
        map.insert(PathBuf::from(path), body.as_bytes().to_vec());
    }
    map
}

/// Map complete owned files only. No replacement inside credentials/subscription URLs.
pub fn destination(path: &Path) -> Result<PathBuf> {
    let s = path.to_str().ok_or("non-UTF8 managed path")?;
    Ok(PathBuf::from(
        s.replace("io.github.upd", "io.github.cm")
            .replace("zz-upd", "zz-cm")
            .replace("90-upd", "90-cm")
            .replace("99upd", "99cm")
            .replace("/upd", "/cm"),
    ))
}
fn system(unit: &str) -> Service {
    Service {
        unit: unit.into(),
        user: None,
        global: false,
    }
}
fn user(unit: &str) -> Service {
    Service {
        unit: unit.into(),
        user: None,
        global: true,
    }
}

/// replacement files come from the SAME current generators as ordinary install.
/// The policy may be inspected offline with synthetic files; real binary digests
/// are pinned, and tests cannot replace the production identity through env vars.
pub fn plan(
    root: &Path,
    replacements: BTreeMap<PathBuf, (Vec<u8>, u32)>,
    services: &mut dyn Services,
    watch: Option<&str>,
) -> Result<Plan> {
    if watch.is_some_and(|p| p != "/etc/pacman.d/mirrorlist") {
        return Err("unsupported legacy mirror watch path".into());
    }
    let fsx = Executor::new(root)?;
    let found = inventory_legacy(root)?;
    if found.is_empty() {
        return Err("no legacy installation to migrate".into());
    }
    let cli = fsx.path(Path::new("/usr/local/bin/upd"))?;
    check_file(&cli, fsx.owner())?;
    if digest(&cli)? != legacy::CLI_SHA256 {
        return Err("unsupported or foreign UPD binary; expected pinned manual UPD 0.2.7".into());
    }
    for target in [
        "/etc/cm.conf",
        "/etc/cm",
        "/var/lib/cm",
        "/run/cm",
        "/usr/local/bin/cm",
        "/usr/bin/cm",
    ] {
        if fs::symlink_metadata(fsx.path(Path::new(target))?).is_ok() {
            return Err("old/new conflict: existing CM installation left unchanged".into());
        }
    }
    if !fsx.path(Path::new("/etc/upd.conf"))?.is_file()
        || !fsx.path(Path::new("/var/lib/upd"))?.is_dir()
    {
        return Err("unsupported incomplete manual layout".into());
    }
    let mut reference = references(watch);
    // Both documented legacy mirror watch variants are known, regardless of distro.
    for (p, b) in references(Some("/etc/pacman.d/mirrorlist")) {
        reference.entry(p).or_insert(b);
    }
    services.assert_unmanaged(&found)?;
    let mut changes = Vec::new();
    let mut unit_services = Vec::new();
    let mut retire = Vec::new();
    for path in found {
        let actual = fsx.path(&path)?;
        if path == Path::new("/run/upd/helper.sock") {
            return Err("legacy helper socket remains; quiesce the legacy runtime first".into());
        }
        if path == Path::new("/run/upd") {
            retire.push(Change {
                path,
                content: Content::Absent,
            });
            continue;
        }
        if matches!(
            path.to_str(),
            Some("/etc/upd.conf" | "/etc/upd" | "/var/lib/upd")
        ) {
            let target = destination(&path)?;
            if fs::symlink_metadata(fsx.path(&target)?).is_ok() {
                return Err("old/new data conflict".into());
            }
            changes.push(Change {
                path: target,
                content: Content::Tree(path.clone()),
            });
            retire.push(Change {
                path,
                content: Content::Absent,
            });
            continue;
        }
        if path == Path::new("/usr/local/bin/upd") {
            retire.push(Change {
                path,
                content: Content::Absent,
            });
            continue;
        }
        if path == Path::new("/usr/local/bin/upd-cosmic") {
            check_file(&actual, fsx.owner())?;
            if digest(&actual)? != legacy::GUI_SHA256 {
                return Err("foreign/unsupported UPD GUI binary".into());
            }
        } else if let Some(expected) = reference.get(&path) {
            check_file(&actual, fsx.owner())?;
            if fs::read(&actual).map_err(|e| e.to_string())? != *expected {
                return Err(format!(
                    "foreign or edited legacy entry: {}",
                    path.display()
                ));
            }
        } else if actual
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
            && path.to_string_lossy().contains(".wants/")
        {
            let unit = path.file_name().ok_or("missing unit")?.to_string_lossy();
            let link = fs::read_link(&actual).map_err(|e| e.to_string())?;
            let expected = Path::new("/etc/systemd/system").join(unit.as_ref());
            let user_expected = Path::new("/etc/systemd/user").join(unit.as_ref());
            let resolved = if link.is_absolute() {
                link.clone()
            } else {
                normalize(&path.parent().ok_or("bad link")?.join(&link))?
            };
            if (resolved != expected && resolved != user_expected)
                || !reference.contains_key(&resolved)
            {
                return Err("foreign legacy enablement link".into());
            }
            retire.push(Change {
                path,
                content: Content::Absent,
            });
            continue;
        } else {
            return Err(format!(
                "unsupported/package/foreign legacy entry: {}",
                path.display()
            ));
        }
        let target = destination(&path)?;
        if !replacements.contains_key(&target) {
            return Err(format!(
                "missing current replacement for {}",
                target.display()
            ));
        }
        if path.parent() == Some(Path::new("/etc/systemd/system"))
            || path.parent() == Some(Path::new("/etc/systemd/user"))
        {
            let unit = path.file_name().ok_or("missing unit")?.to_string_lossy();
            let s = if path.parent() == Some(Path::new("/etc/systemd/user")) {
                user(&unit)
            } else {
                system(&unit)
            };
            let state = services.inspect(&s)?;
            if state.active && (s.unit.ends_with(".service") || s.unit == "upd-helper.socket") {
                return Err(
                    "legacy helper/VPN/operation active; migrate only after quiescence".into(),
                );
            }
            unit_services.push((s, state));
        }
        retire.push(Change {
            path,
            content: Content::Absent,
        });
    }
    for (path, (bytes, mode)) in replacements {
        let actual = fsx.path(&path)?;
        if fs::symlink_metadata(&actual).is_ok() {
            return Err(format!("CM target already exists: {}", path.display()));
        }
        changes.push(Change {
            path,
            content: Content::File(bytes, mode),
        });
    }
    let mut service_changes = Vec::new();
    let sessions = services.user_sessions()?;
    if !sessions.is_empty() && unit_services.iter().any(|(s, _)| s.global) {
        return Err("log out user sessions before migrating global user units; per-user overrides are unsupported".into());
    }
    for (old, state) in unit_services {
        let new = Service {
            unit: old.unit.replacen("upd-", "cm-", 1),
            ..old.clone()
        };
        if services.inspect(&new)? != ServiceState::default() {
            return Err("existing CM service state conflict".into());
        }
        if let Some(target) = enablement_path(&new) {
            let actual = fsx.path(&target)?;
            if fs::symlink_metadata(actual).is_ok() {
                return Err("existing CM enablement link conflict".into());
            }
            if state.enabled {
                let unit_path = Path::new(if new.global {
                    "/etc/systemd/user"
                } else {
                    "/etc/systemd/system"
                })
                .join(&new.unit);
                changes.push(Change {
                    path: target,
                    content: Content::Link(unit_path),
                });
            }
        }
        service_changes.push(ServiceChange {
            service: old,
            after: ServiceState::default(),
            before_files: true,
        });
        service_changes.push(ServiceChange {
            service: new,
            after: state,
            before_files: false,
        });
    }
    // Stop old admissions before writing; retire old sources only AFTER every new
    // file/data tree is installed, so interruption never destroys the only copy.
    changes.extend(retire);
    let mut lock_paths = Vec::new();
    for p in [
        "/var/lib/upd/.lock",
        "/var/lib/upd/vpn/.vpn-config.lock",
        "/etc/.upd.conf.lock",
        "/var/lib/upd/.vpn-subs.lock",
    ] {
        let path = PathBuf::from(p);
        if fsx.path(path.parent().ok_or("bad lock path")?)?.is_dir() {
            lock_paths.push(path);
        }
    }
    let plan = Plan {
        changes,
        services: service_changes,
        lock_paths,
    };
    fsx.validate_plan(&plan)?;
    Ok(plan)
}
fn enablement_path(s: &Service) -> Option<PathBuf> {
    let target = if s.unit == "cm-vpn.service" {
        "multi-user.target"
    } else if s.unit.ends_with(".timer") {
        "timers.target"
    } else if s.unit.ends_with(".socket") {
        "sockets.target"
    } else if s.unit.ends_with(".path") {
        "paths.target"
    } else {
        return None;
    };
    Some(
        Path::new(if s.global {
            "/etc/systemd/user"
        } else {
            "/etc/systemd/system"
        })
        .join(format!("{target}.wants"))
        .join(&s.unit),
    )
}
fn normalize(path: &Path) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::ParentDir => {
                if !out.pop() {
                    return Err("invalid link".into());
                }
            }
            std::path::Component::CurDir => {}
            _ => out.push(c.as_os_str()),
        }
    }
    Ok(out)
}
fn check_file(path: &Path, owner: u32) -> Result<()> {
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !m.is_file() || m.uid() != owner || m.mode() & 0o022 != 0 {
        return Err("unsafe owner/type/mode on managed legacy file".into());
    }
    Ok(())
}
fn digest(path: &Path) -> Result<String> {
    Ok(super::transaction::hex(&Sha256::digest(
        fs::read(path).map_err(|e| e.to_string())?,
    )))
}

/// Inspect /proc by executable device/inode and exact deleted paths, never by
/// process name or stale PID. The production CLI performs an early read-only
/// audit and repeats it after acquiring operation/config locks and stopping
/// timer admission.
pub fn no_legacy_process(root: &Path) -> Result<()> {
    if root != Path::new("/") {
        return Err("production process audit requires the host filesystem root".into());
    }
    audit_processes(
        &[
            PathBuf::from("/usr/local/bin/upd"),
            PathBuf::from("/usr/local/bin/upd-cosmic"),
            PathBuf::from("/var/lib/upd/vpn/bin/mihomo"),
        ],
        Path::new("/proc"),
    )
}

fn audit_processes(managed: &[PathBuf], proc_root: &Path) -> Result<()> {
    let mut identities = Vec::new();
    for path in managed {
        match fs::metadata(path) {
            Ok(metadata) => identities.push((metadata.dev(), metadata.ino())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "cannot establish legacy executable identity for {}: {error}",
                    path.display()
                ));
            }
        }
    }

    for entry in fs::read_dir(proc_root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry
            .file_name()
            .to_string_lossy()
            .bytes()
            .all(|b| b.is_ascii_digit())
        {
            continue;
        }

        let pid = entry.path();
        let exe = pid.join("exe");
        let target = match fs::read_link(&exe) {
            Ok(target) => target,
            Err(error) if safely_missing_exe(&pid, &error)? => continue,
            Err(error) => {
                return Err(format!(
                    "cannot establish legacy process quiescence: {error}"
                ));
            }
        };

        // /proc/PID/exe retains the original pathname with this suffix after
        // unlink/atomic replacement. This catches a running legacy image even
        // when the installed path no longer identifies its inode.
        if managed
            .iter()
            .any(|path| is_managed_proc_target(&target, path))
        {
            return Err("legacy executable still running; migration refused".into());
        }

        match fs::metadata(&exe) {
            Ok(metadata) => check_executable_identity(&metadata, &identities)?,
            Err(error) if safely_missing_exe(&pid, &error)? => {}
            Err(error) => {
                return Err(format!(
                    "cannot establish legacy process quiescence: {error}"
                ));
            }
        }
    }
    Ok(())
}

fn check_executable_identity(metadata: &fs::Metadata, identities: &[(u64, u64)]) -> Result<()> {
    if identities.contains(&(metadata.dev(), metadata.ino())) {
        return Err("legacy executable still running; migration refused".into());
    }
    // After every hardlink has been removed, neither the installed pathname
    // nor its current inode can identify a running legacy image. Do not assume
    // an unlinked (including anonymous) executable is unrelated to migration.
    if metadata.nlink() == 0 {
        return Err("unidentified unlinked executable; cannot establish legacy quiescence".into());
    }
    Ok(())
}

fn is_managed_proc_target(target: &Path, managed: &Path) -> bool {
    if target == managed {
        return true;
    }
    let deleted = format!("{} (deleted)", managed.display());
    target == Path::new(&deleted)
}

/// Missing `exe` is ignorable only for a vanished PID, a zombie, or a kernel
/// thread. Other live processes must expose an executable we can inspect.
fn safely_missing_exe(pid: &Path, error: &std::io::Error) -> Result<bool> {
    if !matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ESRCH)) {
        return Ok(false);
    }
    match fs::metadata(pid) {
        Err(pid_error) if matches!(pid_error.raw_os_error(), Some(libc::ENOENT | libc::ESRCH)) => {
            Ok(true)
        }
        Err(pid_error) => Err(format!(
            "cannot establish legacy process quiescence for {}: {pid_error}",
            pid.display()
        )),
        Ok(_) => proc_stat_is_unexecutable(pid),
    }
}

fn proc_stat_is_unexecutable(pid: &Path) -> Result<bool> {
    const PF_KTHREAD: u64 = 0x0020_0000;
    let stat_path = pid.join("stat");
    let stat = match fs::read(&stat_path) {
        Ok(stat) => stat,
        Err(error)
            if matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ESRCH))
                && matches!(
                    fs::metadata(pid),
                    Err(pid_error) if matches!(pid_error.raw_os_error(), Some(libc::ENOENT | libc::ESRCH))
                ) =>
        {
            return Ok(true);
        }
        Err(error) => {
            return Err(format!(
                "cannot establish legacy process state from {}: {error}",
                stat_path.display()
            ));
        }
    };
    // comm is parenthesized but may itself contain spaces or ')'. The final
    // ')' marks the start of the fixed-format fields (state is field 3).
    let close = stat
        .iter()
        .rposition(|byte| *byte == b')')
        .ok_or_else(|| format!("malformed process stat: {}", stat_path.display()))?;
    let fields: Vec<&[u8]> = stat[close + 1..]
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|field| !field.is_empty())
        .collect();
    let state = fields
        .first()
        .ok_or_else(|| format!("malformed process stat: {}", stat_path.display()))?;
    if *state == b"Z" {
        return Ok(true);
    }
    let flags = fields
        .get(6)
        .and_then(|field| std::str::from_utf8(field).ok())
        .and_then(|field| field.parse::<u64>().ok())
        .ok_or_else(|| format!("malformed process stat: {}", stat_path.display()))?;
    Ok(flags & PF_KTHREAD != 0)
}

pub struct Systemd;
fn guard_active_legacy_admission(service: &Service, observed: &ServiceState) -> Result<()> {
    let admission = (service.unit.starts_with("upd-") && service.unit.ends_with(".service"))
        || service.unit == "upd-helper.socket";
    if admission && observed.active {
        return Err("active legacy service appeared; migration refused".into());
    }
    Ok(())
}

impl Systemd {
    fn args(service: &Service) -> Result<Vec<String>> {
        if !service
            .unit
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.'))
            || !(service.unit.starts_with("upd-") || service.unit.starts_with("cm-"))
        {
            return Err("invalid migration service".into());
        }
        let mut args = Vec::new();
        if service.global {
            args.push("--global".into());
        }
        if let Some(user) = &service.user {
            if !user
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            {
                return Err("invalid service user".into());
            }
            args.extend(["--user".into(), "-M".into(), format!("{user}@")]);
        }
        Ok(args)
    }
    fn command(args: &[String]) -> Result<(String, bool)> {
        let mut command = Command::new("/usr/bin/systemctl");
        command.args(args).env("LC_ALL", "C");
        let out = crate::common::capture(&mut command, Some(65536)).map_err(|e| e.to_string())?;
        Ok((
            String::from_utf8(out.stdout)
                .map_err(|_| "invalid systemctl output")?
                .trim()
                .into(),
            out.status.success(),
        ))
    }
    fn run(service: &Service, operation: &str) -> Result<()> {
        let mut args = Self::args(service)?;
        args.extend([operation.into(), service.unit.clone()]);
        if !Self::command(&args)?.1 {
            return Err(format!("systemctl {operation} {} failed", service.unit));
        }
        Ok(())
    }
}
impl Services for Systemd {
    fn inspect(&mut self, s: &Service) -> Result<ServiceState> {
        let mut args = Self::args(s)?;
        args.extend(["is-enabled".into(), s.unit.clone()]);
        let (enabled, _) = Self::command(&args)?;
        if s.global && enabled.is_empty() && Path::new("/etc/systemd/user").join(&s.unit).exists() {
            return Err("global service enablement inspection failed".into());
        }
        let enabled = match enabled.as_str() {
            "enabled" => true,
            "disabled" | "static" | "not-found" | "" => false,
            _ => return Err("unsupported/masked/indirect service enablement".into()),
        };
        if s.global {
            return Ok(ServiceState {
                enabled,
                active: false,
            });
        }
        let mut args = Self::args(s)?;
        args.extend([
            "show".into(),
            "--property=LoadState,ActiveState,FragmentPath,DropInPaths,NeedDaemonReload".into(),
            s.unit.clone(),
        ]);
        let (text, success) = Self::command(&args)?;
        if !success {
            return Err("cannot inspect systemd service".into());
        }
        let values: BTreeMap<_, _> = text.lines().filter_map(|l| l.split_once('=')).collect();
        if values.get("LoadState") == Some(&"not-found") {
            return Ok(ServiceState::default());
        }
        if values.get("LoadState") != Some(&"loaded")
            || values.get("DropInPaths").is_none_or(|s| !s.is_empty())
        {
            return Err("foreign effective unit or drop-in".into());
        }
        if s.unit.starts_with("upd-") && values.get("NeedDaemonReload") != Some(&"no") {
            return Err(
                "legacy manager cache differs from disk; reload and recheck before migration"
                    .into(),
            );
        }
        let expected = format!(
            "/etc/systemd/{}/{}",
            if s.user.is_some() { "user" } else { "system" },
            s.unit
        );
        if values.get("FragmentPath") != Some(&expected.as_str()) {
            return Err("package/foreign effective unit".into());
        }
        let active = match values.get("ActiveState").copied() {
            Some("active") => true,
            Some("inactive") | Some("failed") => false,
            _ => return Err("service transitioning; migration refused".into()),
        };
        Ok(ServiceState { enabled, active })
    }
    fn set(&mut self, s: &Service, state: &ServiceState) -> Result<()> {
        let current = self.inspect(s)?;
        guard_active_legacy_admission(s, &current)?;
        if current.active && !state.active {
            Self::run(s, "stop")?;
        }
        if current.enabled != state.enabled {
            Self::run(s, if state.enabled { "enable" } else { "disable" })?;
        }
        if !current.active && state.active {
            Self::run(s, "start")?;
        }
        if self.inspect(s)? != *state {
            return Err("service state did not converge".into());
        }
        Ok(())
    }
    fn reload(&mut self) -> Result<()> {
        if !Self::command(&["daemon-reload".into()])?.1 {
            return Err("daemon-reload failed".into());
        }
        for user in self.user_sessions()? {
            if !Self::command(&[
                "--user".into(),
                "-M".into(),
                format!("{user}@"),
                "daemon-reload".into(),
            ])?
            .1
            {
                return Err("user daemon-reload failed".into());
            }
        }
        Ok(())
    }
    fn user_sessions(&mut self) -> Result<Vec<String>> {
        let mut command = Command::new("/usr/bin/loginctl");
        command
            .args(["list-users", "--no-legend"])
            .env("LC_ALL", "C");
        let out = crate::common::capture(&mut command, Some(65536)).map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err("cannot inventory logged-in user services".into());
        }
        let mut users = Vec::new();
        for line in String::from_utf8(out.stdout)
            .map_err(|_| "invalid loginctl output")?
            .lines()
        {
            let mut fields = line.split_whitespace();
            let uid: u32 = fields
                .next()
                .ok_or("invalid loginctl row")?
                .parse()
                .map_err(|_| "invalid loginctl uid")?;
            let name = fields.next().ok_or("missing loginctl user")?;
            if uid >= 1000 {
                users.push(name.to_owned());
            }
        }
        Ok(users)
    }
    fn assert_unmanaged(&mut self, paths: &[PathBuf]) -> Result<()> {
        let (binary, base, missing) = if Path::new("/usr/bin/pacman").is_file() {
            ("/usr/bin/pacman", vec!["-Qo", "--"], "No package owns")
        } else if Path::new("/usr/bin/dpkg-query").is_file() {
            (
                "/usr/bin/dpkg-query",
                vec!["-S", "--"],
                "no path found matching pattern",
            )
        } else if Path::new("/usr/bin/rpm").is_file() {
            (
                "/usr/bin/rpm",
                vec!["-qf", "--"],
                "is not owned by any package",
            )
        } else {
            return Err("cannot establish package ownership".into());
        };
        for path in paths.iter().filter(|p| !p.starts_with("/run")) {
            let mut command = Command::new(binary);
            command.args(&base).arg(path).env("LC_ALL", "C");
            let out =
                crate::common::capture(&mut command, Some(65536)).map_err(|e| e.to_string())?;
            if out.status.success() {
                return Err(format!("package-owned entry: {}", path.display()));
            }
            let message = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            if out.status.code() != Some(1) || !message.contains(missing) {
                return Err("package ownership query inconclusive".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod systemd_admission_tests {
    use super::{Service, ServiceState, guard_active_legacy_admission};

    fn service(unit: &str) -> Service {
        Service {
            unit: unit.into(),
            user: None,
            global: false,
        }
    }

    fn active() -> ServiceState {
        ServiceState {
            enabled: false,
            active: true,
        }
    }

    #[test]
    fn active_legacy_vpn_helper_and_operation_admissions_are_refused() {
        for unit in ["upd-vpn.service", "upd-helper.socket", "upd-auto.service"] {
            assert!(guard_active_legacy_admission(&service(unit), &active()).is_err());
        }
    }

    #[test]
    fn inactive_legacy_services_are_allowed() {
        for unit in ["upd-vpn.service", "upd-helper.socket", "upd-auto.service"] {
            assert!(
                guard_active_legacy_admission(&service(unit), &ServiceState::default()).is_ok()
            );
        }
    }

    #[test]
    fn active_timers_paths_and_new_cm_services_remain_allowed() {
        for unit in ["upd-auto.timer", "upd-mirrors.path", "cm-vpn.service"] {
            assert!(guard_active_legacy_admission(&service(unit), &active()).is_ok());
        }
    }
}

#[cfg(test)]
mod process_audit_tests {
    use super::{audit_processes, check_executable_identity};
    use std::{
        fs,
        os::unix::fs::{MetadataExt, PermissionsExt, symlink},
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("cm-process-audit-{}-{nonce}", std::process::id()));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.0.join(relative)
        }

        fn proc_root(&self) -> PathBuf {
            self.path("proc")
        }

        fn managed(&self) -> Vec<PathBuf> {
            [
                "root/usr/local/bin/upd",
                "root/usr/local/bin/upd-cosmic",
                "root/var/lib/upd/vpn/bin/mihomo",
            ]
            .iter()
            .map(|p| self.path(p))
            .collect()
        }

        fn add_process_link(&self, pid: &str, target: &Path) {
            let exe = self.path(&format!("proc/{pid}/exe"));
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            symlink(target, exe).unwrap();
        }

        fn add_process_stat(&self, pid: &str, stat: &str) {
            let directory = self.path(&format!("proc/{pid}"));
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("stat"), stat).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn live_legacy_core_is_detected_by_inode_but_same_name_is_ignored() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.path("root/var/lib/upd/vpn/bin")).unwrap();
        let core = fixture.managed()[2].clone();
        fs::write(&core, b"synthetic core image").unwrap();
        let alias = fixture.path("elsewhere/mihomo");
        fs::create_dir_all(alias.parent().unwrap()).unwrap();
        fs::hard_link(&core, &alias).unwrap();
        fs::create_dir_all(fixture.path("unrelated")).unwrap();
        let unrelated = fixture.path("unrelated/mihomo");
        fs::write(&unrelated, b"unrelated image with the same basename").unwrap();

        // The proc link uses a different pathname, so only device/inode catches
        // the hard-linked managed image.
        fixture.add_process_link("101", &alias);
        assert!(audit_processes(&fixture.managed(), &fixture.proc_root()).is_err());

        fs::remove_dir_all(fixture.proc_root()).unwrap();
        fixture.add_process_link("102", &unrelated);
        assert!(audit_processes(&fixture.managed(), &fixture.proc_root()).is_ok());
    }

    #[test]
    fn exact_deleted_legacy_executable_paths_are_detected() {
        let fixture = Fixture::new();
        let managed = fixture.managed();
        for (index, path) in managed.iter().enumerate() {
            let deleted = PathBuf::from(format!("{} (deleted)", path.display()));
            fixture.add_process_link(&format!("{}", 201 + index), &deleted);
            assert!(
                audit_processes(&managed, &fixture.proc_root()).is_err(),
                "deleted managed image was missed: {}",
                path.display()
            );
            fs::remove_dir_all(fixture.proc_root()).unwrap();
        }
    }

    #[test]
    fn deleted_hardlink_image_without_installed_identity_is_refused() {
        let fixture = Fixture::new();
        let core = fixture.managed()[2].clone();
        fs::create_dir_all(core.parent().unwrap()).unwrap();
        fs::write(&core, b"synthetic legacy core image").unwrap();
        let alias = fixture.path("elsewhere/alias");
        fs::create_dir_all(alias.parent().unwrap()).unwrap();
        fs::hard_link(&core, &alias).unwrap();
        let image = fs::File::open(&alias).unwrap();
        let linked = image.metadata().unwrap();
        assert!(check_executable_identity(&linked, &[]).is_ok());

        fs::remove_file(core).unwrap();
        fs::remove_file(alias).unwrap();
        let unlinked = image.metadata().unwrap();
        assert_eq!(unlinked.nlink(), 0);
        let error = check_executable_identity(&unlinked, &[]).unwrap_err();
        assert!(error.contains("unidentified unlinked executable"));
        let known = [(unlinked.dev(), unlinked.ino())];
        let error = check_executable_identity(&unlinked, &known).unwrap_err();
        assert!(error.contains("legacy executable still running"));
    }

    #[test]
    fn unreadable_or_missing_exe_for_a_present_pid_fails_closed() {
        let fixture = Fixture::new();
        fixture.add_process_stat("301", "301 (ordinary process) S 1 2 3 4 5 0 8\n");
        fixture.add_process_link("301", &fixture.path("missing/unrelated-image"));
        let error = audit_processes(&fixture.managed(), &fixture.proc_root()).unwrap_err();
        assert!(error.contains("cannot establish legacy process quiescence"));
    }

    #[test]
    fn missing_stat_for_present_pid_fails_closed() {
        let fixture = Fixture::new();
        fixture.add_process_link("302", &fixture.path("missing/unrelated-image"));
        let error = audit_processes(&fixture.managed(), &fixture.proc_root()).unwrap_err();
        assert!(error.contains("cannot establish legacy process state"));
    }

    #[test]
    fn missing_exe_is_ignored_only_for_kernel_threads_or_zombies() {
        let kernel = Fixture::new();
        kernel.add_process_stat("401", "401 (kernel ) thread) R 1 2 3 4 5 2097152 8\n");
        assert!(audit_processes(&kernel.managed(), &kernel.proc_root()).is_ok());

        let zombie = Fixture::new();
        zombie.add_process_stat("402", "402 (zombie process) Z 1 2 3 4 5 0 8\n");
        assert!(audit_processes(&zombie.managed(), &zombie.proc_root()).is_ok());

        let ordinary = Fixture::new();
        ordinary.add_process_stat("403", "403 (ordinary process) S 1 2 3 4 5 0 8\n");
        assert!(audit_processes(&ordinary.managed(), &ordinary.proc_root()).is_err());

        let malformed = Fixture::new();
        malformed.add_process_stat("404", "malformed stat row\n");
        assert!(audit_processes(&malformed.managed(), &malformed.proc_root()).is_err());
    }
}
