//! App-worker process: start, reload and stop through the unix API socket.
//!
//! The socket path is chosen here from the instance run directory after
//! `worker::generate`. A caller cannot put `external-controller-unix` in the
//! document: that key stays forbidden on input.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::api;
use super::config::{self, attach_instance_controller};
use super::stats;
use super::validate::validate_file;
use crate::core::adapter::{
    ApiState, CoreAdapter, CoreCapabilities, CoreConfig, CoreError, CoreReadiness, CoreStatistics,
    RemoteState, RouteState,
};
use crate::core::instance::{InstanceId, InstanceRoot};
use crate::core::leases::{Lease, LeaseError, LeaseRegistry, ResourceKind};
use crate::core::mihomo;

const API_WAIT: Duration = Duration::from_secs(5);

/// One mihomo process for an application instance.
pub struct MihomoWorker {
    id: InstanceId,
    root: InstanceRoot,
    lease_dir: PathBuf,
    binary: PathBuf,
    registry: Option<LeaseRegistry>,
    port: Option<Lease>,
    socket: Option<Lease>,
    socket_path: Option<PathBuf>,
    config_path: Option<PathBuf>,
    child: Option<Child>,
    selected: Option<String>,
    readiness: CoreReadiness,
    run_as: Option<crate::controller::drop::RunAs>,
    tunnel: Option<crate::net::TunnelNet>,
}

impl MihomoWorker {
    pub fn new(
        id: InstanceId,
        root: InstanceRoot,
        lease_dir: impl Into<PathBuf>,
        binary: impl Into<PathBuf>,
    ) -> Self {
        Self {
            id,
            root,
            lease_dir: lease_dir.into(),
            binary: binary.into(),
            registry: None,
            port: None,
            socket: None,
            socket_path: None,
            config_path: None,
            child: None,
            selected: None,
            readiness: CoreReadiness::DOWN,
            run_as: None,
            tunnel: None,
        }
    }

    /// Запускать ядро от имени вызывающего. Без вызова `spawn` оставляет прежний `setsid`.
    pub fn set_run_as(&mut self, run_as: crate::controller::drop::RunAs) {
        self.run_as = Some(run_as);
    }

    pub fn set_tunnel_net(&mut self, tunnel: crate::net::TunnelNet) {
        self.tunnel = Some(tunnel);
    }

    pub fn socket_path(&self) -> Option<&Path> {
        self.socket_path.as_deref()
    }

    pub fn config_bytes(&self) -> Option<Vec<u8>> {
        self.config_path
            .as_ref()
            .and_then(|path| fs::read(path).ok())
    }

    pub fn group_alive(&mut self) -> bool {
        self.reap();
        let Some(child) = self.child.as_ref() else {
            return false;
        };
        signal_group(child.id() as i32, 0)
    }

    /// Drop this holder's leases when its process is gone.
    pub fn reclaim_if_absent(&mut self, live: &[&str]) -> Result<usize, CoreError> {
        self.ensure_registry()?;
        let registry = self.registry.as_ref().ok_or(CoreError::Failed)?;
        registry.reclaim_absent(live).map_err(lease_error)
    }

    pub fn leases(&self) -> Result<Vec<Lease>, CoreError> {
        let registry = self.registry.as_ref().ok_or(CoreError::NotRunning)?;
        registry.list().map_err(lease_error)
    }

    /// Process-group id of the running core. `setsid` makes this the group leader.
    pub fn process_id(&self) -> Option<u32> {
        self.child.as_ref().map(|child| child.id())
    }

    fn ensure_registry(&mut self) -> Result<(), CoreError> {
        if self.registry.is_some() {
            return Ok(());
        }
        fs::create_dir_all(&self.lease_dir).map_err(|_| CoreError::Failed)?;
        let meta = fs::symlink_metadata(&self.lease_dir).map_err(|_| CoreError::Failed)?;
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(CoreError::Failed);
        }
        let mut permissions = meta.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&self.lease_dir, permissions).map_err(|_| CoreError::Failed)?;
        let registry = match LeaseRegistry::open(&self.lease_dir) {
            Ok(registry) => registry,
            Err(_) => LeaseRegistry::create(&self.lease_dir).map_err(lease_error)?,
        };
        self.registry = Some(registry);
        Ok(())
    }

    fn allocate(&mut self, kind: ResourceKind) -> Result<Lease, CoreError> {
        self.ensure_registry()?;
        let holder = self.id.as_str().to_owned();
        self.registry
            .as_ref()
            .ok_or(CoreError::Failed)?
            .allocate(&holder, kind)
            .map_err(lease_error)
    }

    fn release_leases(&mut self) -> Result<(), CoreError> {
        let holder = self.id.as_str().to_owned();
        if self.registry.is_none() {
            self.port = None;
            self.socket = None;
            return Ok(());
        }
        self.release_one(&holder, true)?;
        self.release_one(&holder, false)?;
        Ok(())
    }

    fn release_one(&mut self, holder: &str, port: bool) -> Result<(), CoreError> {
        let lease = if port {
            self.port.clone()
        } else {
            self.socket.clone()
        };
        let Some(lease) = lease else {
            return Ok(());
        };
        let registry = self.registry.as_ref().ok_or(CoreError::Failed)?;
        match registry.release(holder, lease.kind, &lease.value) {
            Ok(()) | Err(LeaseError::Conflict) => {
                if port {
                    self.port = None;
                } else {
                    self.socket = None;
                }
                Ok(())
            }
            Err(error) => Err(lease_error(error)),
        }
    }

    fn abort_start(&mut self, error: CoreError) -> Result<CoreReadiness, CoreError> {
        self.release_leases()?;
        Err(error)
    }

    fn spawn(&mut self, state: &Path, config: &Path) -> Result<(), CoreError> {
        let mut command = Command::new(&self.binary);
        command
            .arg("-d")
            .arg(state)
            .arg("-f")
            .arg(config)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("LC_ALL", "C");
        if let Some(run_as) = self.run_as.clone() {
            crate::controller::drop::worker_pre_exec(
                &mut command,
                run_as,
                &[],
                crate::controller::harden::ChildLimits::default(),
            );
        } else {
            // SAFETY: setsid runs in the forked child before exec and only changes that child's session.
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        let child = command.spawn().map_err(|_| CoreError::Failed)?;
        self.child = Some(child);
        Ok(())
    }

    fn wait_api(&self) -> Result<(), CoreError> {
        let socket = self.socket_path.as_ref().ok_or(CoreError::Failed)?;
        let deadline = Instant::now() + API_WAIT;
        let uid = crate::common::sys::euid();
        loop {
            if api::request(
                socket,
                uid,
                "GET",
                "/version",
                None,
                Duration::from_millis(400),
            )
            .is_ok()
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(CoreError::Timeout);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The core answers on its API socket before it binds listeners. Route
    /// readiness means the leased `127.0.0.1` port accepts connections.
    fn wait_listener(&self) -> Result<(), CoreError> {
        let port = self
            .port
            .as_ref()
            .and_then(|lease| lease.value.parse::<u16>().ok())
            .ok_or(CoreError::Failed)?;
        crate::core::process::wait_port(port, API_WAIT)
    }

    fn wait_tun(&self) -> Result<(), CoreError> {
        let name = self
            .tunnel
            .as_ref()
            .map(|tunnel| tunnel.tun.as_str())
            .ok_or(CoreError::Failed)?;
        let deadline = Instant::now() + API_WAIT;
        loop {
            if tun_is_up(name) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(CoreError::Timeout);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn reap(&mut self) {
        if let Some(child) = self.child.as_mut()
            && matches!(child.try_wait(), Ok(Some(_)) | Err(_))
        {
            self.readiness = CoreReadiness::DOWN;
        }
    }

    fn probe_delay(&self) -> RemoteState {
        let Some(socket) = self.socket_path.as_ref() else {
            return RemoteState::Unknown;
        };
        let Some(name) = self.selected.as_deref() else {
            return RemoteState::Unknown;
        };
        let path = format!(
            "/proxies/{}/delay?timeout=500&url={}",
            pct(name),
            pct("http://www.gstatic.com/generate_204")
        );
        match api::request(
            socket,
            crate::common::sys::euid(),
            "GET",
            &path,
            None,
            Duration::from_millis(800),
        ) {
            Ok(value) if value.get("delay").and_then(|item| item.as_u64()).is_some() => {
                RemoteState::RemoteReachable
            }
            Ok(_) => RemoteState::Unreachable,
            Err(_) => RemoteState::Unreachable,
        }
    }
}

impl CoreAdapter for MihomoWorker {
    fn capabilities(&self) -> CoreCapabilities {
        let descriptor = mihomo::descriptor();
        CoreCapabilities {
            core: "mihomo",
            version: descriptor.version,
            reload_without_restart: descriptor.reload_without_restart,
            delay_probe: descriptor.delay_probe,
        }
    }

    fn validate(&mut self, config: &CoreConfig) -> Result<(), CoreError> {
        let document: Value =
            serde_json::from_slice(config.as_bytes()).map_err(|_| CoreError::InvalidConfig)?;
        config::reject_geo_document(&document).map_err(|_| CoreError::InvalidConfig)?;
        attach_instance_controller(&document, 20_000, Path::new("/run/cm-worker.sock"))
            .map(|_| ())
            .map_err(|_| CoreError::InvalidConfig)?;
        Ok(())
    }

    fn start(&mut self, config: &CoreConfig) -> Result<CoreReadiness, CoreError> {
        if self.group_alive() {
            return Err(CoreError::Conflict);
        }
        if self.child.is_some() {
            CoreAdapter::stop(self)?;
        }
        let document: Value =
            serde_json::from_slice(config.as_bytes()).map_err(|_| CoreError::InvalidConfig)?;
        config::reject_geo_document(&document).map_err(|_| CoreError::InvalidConfig)?;
        let dirs = self.root.create(&self.id).map_err(|_| CoreError::Failed)?;
        let tun = self.tunnel.clone();
        if tun.is_none() {
            self.port = Some(self.allocate(ResourceKind::Port)?);
        }
        self.socket = Some(match self.allocate(ResourceKind::Socket) {
            Ok(socket) => socket,
            Err(error) => return self.abort_start(error),
        });
        let socket_name = match self.socket.as_ref().map(|lease| lease.value.clone()) {
            Some(name) => name,
            None => return self.abort_start(CoreError::Failed),
        };
        let socket_path = dirs.run.join(format!("{socket_name}.sock"));
        let bytes = match render_start(&document, tun.as_ref(), self.port.as_ref(), &socket_path) {
            Ok(bytes) => bytes,
            Err(_) => return self.abort_start(CoreError::InvalidConfig),
        };
        let config_path = dirs.config.join("config.json");
        if write_private(&config_path, &bytes).is_err() {
            return self.abort_start(CoreError::Failed);
        }
        if validate_file(&self.binary, &dirs.cache, &bytes).is_err() {
            let removed = remove_path(&config_path);
            let released = self.release_leases();
            if removed.is_err() || released.is_err() {
                return Err(CoreError::Failed);
            }
            return Err(CoreError::InvalidConfig);
        }
        if self.spawn(&dirs.root, &config_path).is_err() {
            let removed = remove_path(&config_path);
            let released = self.release_leases();
            if removed.is_err() || released.is_err() {
                return Err(CoreError::Failed);
            }
            return Err(CoreError::Failed);
        }
        self.socket_path = Some(socket_path);
        self.config_path = Some(config_path);
        self.selected = first_proxy(&document);
        if self.wait_api().is_err()
            || (if tun.is_some() {
                self.wait_tun()
            } else {
                self.wait_listener()
            })
            .is_err()
        {
            CoreAdapter::stop(self)?;
            return Err(CoreError::Timeout);
        }
        self.readiness = CoreReadiness {
            api: ApiState::ApiReady,
            route: RouteState::RouteReady,
            remote: RemoteState::Unknown,
        };
        Ok(self.readiness)
    }

    fn stop(&mut self) -> Result<(), CoreError> {
        let pgid = self.child.as_ref().map(|child| child.id() as i32);
        if let Some(child) = self.child.as_mut() {
            crate::core::process::terminate_child(child);
        }
        let mut error = None;
        if let Some(mut child) = self.child.take() {
            match child.wait() {
                Ok(_) => {}
                Err(err) if err.raw_os_error() == Some(libc::ECHILD) => {}
                Err(_) => error = Some(CoreError::Failed),
            }
        }
        if let Some(pgid) = pgid
            && signal_group(pgid, 0)
        {
            error.get_or_insert(CoreError::Failed);
        }
        if let Some(path) = self.socket_path.take()
            && let Err(remove) = remove_path(&path)
        {
            error.get_or_insert(remove);
        }
        if let Err(release) = self.release_leases() {
            error.get_or_insert(release);
        }
        self.config_path = None;
        self.selected = None;
        self.readiness = CoreReadiness::DOWN;
        match error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn reload(&mut self, config: &CoreConfig) -> Result<CoreReadiness, CoreError> {
        if !self.group_alive() {
            self.readiness = CoreReadiness::DOWN;
            return Err(CoreError::NotRunning);
        }
        let document: Value =
            serde_json::from_slice(config.as_bytes()).map_err(|_| CoreError::InvalidConfig)?;
        config::reject_geo_document(&document).map_err(|_| CoreError::InvalidConfig)?;
        let socket_path = self.socket_path.clone().ok_or(CoreError::Failed)?;
        let config_path = self.config_path.clone().ok_or(CoreError::Failed)?;
        let bytes = render_start(
            &document,
            self.tunnel.as_ref(),
            self.port.as_ref(),
            &socket_path,
        )
        .map_err(|_| CoreError::InvalidConfig)?;
        let parent = config_path.parent().ok_or(CoreError::Failed)?;
        let sandbox = parent.join("reload-check");
        fs::create_dir_all(&sandbox).map_err(|_| CoreError::Failed)?;
        if validate_file(&self.binary, &sandbox, &bytes).is_err() {
            return Err(CoreError::InvalidConfig);
        }
        let previous = fs::read(&config_path).map_err(|_| CoreError::Failed)?;
        if write_private(&config_path, &bytes).is_err() {
            return Err(CoreError::Failed);
        }
        let put = api::request(
            &socket_path,
            crate::common::sys::euid(),
            "PUT",
            "/configs?force=true",
            Some(serde_json::json!({ "path": config_path })),
            Duration::from_secs(8),
        );
        if put.is_err() {
            write_private(&config_path, &previous).map_err(|_| CoreError::Failed)?;
            return Err(CoreError::InvalidConfig);
        }
        // PUT /configs rebinds listeners; the route is ready once the port or TUN answers again.
        if self.tunnel.is_some() {
            self.wait_tun()?
        } else {
            self.wait_listener()?
        };
        self.selected = first_proxy(&document);
        self.readiness.api = ApiState::ApiReady;
        Ok(self.readiness)
    }

    fn health(&mut self) -> Result<CoreReadiness, CoreError> {
        self.reap();
        if !self.group_alive() {
            self.readiness = CoreReadiness::DOWN;
            return Ok(CoreReadiness::DOWN);
        }
        let socket = self.socket_path.as_ref().ok_or(CoreError::ApiUnavailable)?;
        if api::request(
            socket,
            crate::common::sys::euid(),
            "GET",
            "/version",
            None,
            Duration::from_secs(2),
        )
        .is_err()
        {
            self.readiness = CoreReadiness::DOWN;
            return Ok(CoreReadiness::DOWN);
        }
        let remote = self.probe_delay();
        self.readiness = CoreReadiness {
            api: ApiState::ApiReady,
            route: RouteState::RouteReady,
            remote,
        };
        Ok(self.readiness)
    }

    fn statistics(&mut self) -> Result<CoreStatistics, CoreError> {
        if !self.group_alive() {
            return Err(CoreError::NotRunning);
        }
        let socket = self.socket_path.as_ref().ok_or(CoreError::ApiUnavailable)?;
        stats::read_statistics(socket, crate::common::sys::euid())
    }

    fn core_pid(&self) -> Option<u32> {
        self.process_id()
    }

    fn set_tunnel_net(&mut self, tunnel: crate::net::TunnelNet) {
        self.tunnel = Some(tunnel);
    }
}

fn first_proxy(document: &Value) -> Option<String> {
    document
        .get("proxies")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("name"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

impl Drop for MihomoWorker {
    fn drop(&mut self) {
        let held = self.child.is_some()
            || self.socket_path.is_some()
            || self.port.is_some()
            || self.socket.is_some();
        if held && CoreAdapter::stop(self).is_err() {
            self.readiness = CoreReadiness::DOWN;
        }
    }
}

fn render_start(
    document: &Value,
    tunnel: Option<&crate::net::TunnelNet>,
    port: Option<&Lease>,
    socket_path: &Path,
) -> Result<Vec<u8>, CoreError> {
    if let Some(tunnel) = tunnel {
        let tun_bytes =
            config::attach_tun(document, tunnel).map_err(|_| CoreError::InvalidConfig)?;
        let value: Value =
            serde_json::from_slice(&tun_bytes).map_err(|_| CoreError::InvalidConfig)?;
        return config::attach_unix_controller(&value, socket_path)
            .map_err(|_| CoreError::InvalidConfig);
    }
    let port = port
        .and_then(|lease| lease.value.parse().ok())
        .ok_or(CoreError::Failed)?;
    attach_instance_controller(document, port, socket_path).map_err(|_| CoreError::InvalidConfig)
}

/// The TUN is created up by CM before the core starts, so `IFF_UP` alone proves nothing.
/// `IFF_RUNNING` appears only while a process holds the device open. The flags come from an
/// ioctl, not from sysfs: sysfs shows the namespace it was mounted in, not the current one.
fn tun_is_up(name: &str) -> bool {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    if name.is_empty() || name.len() >= libc::IFNAMSIZ {
        return false;
    }
    // SAFETY: socket returns a new descriptor or -1; no pointer is passed.
    let raw = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if raw < 0 {
        return false;
    }
    // SAFETY: the descriptor was just returned by socket and is owned here.
    let socket = unsafe { OwnedFd::from_raw_fd(raw) };
    // SAFETY: ifreq is a plain C struct; the all-zero pattern is valid.
    let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
    for (slot, byte) in request.ifr_name.iter_mut().zip(name.bytes()) {
        *slot = byte as libc::c_char;
    }
    // SAFETY: SIOCGIFFLAGS reads the NUL-terminated name and writes the flags into `request`,
    // which outlives the call.
    if unsafe { libc::ioctl(socket.as_raw_fd(), libc::SIOCGIFFLAGS as _, &mut request) } != 0 {
        return false;
    }
    // SAFETY: a successful SIOCGIFFLAGS fills the flags member of the union.
    let flags = i32::from(unsafe { request.ifr_ifru.ifru_flags });
    let wanted = libc::IFF_UP | libc::IFF_RUNNING;
    flags & wanted == wanted
}

fn remove_path(path: &Path) -> Result<(), CoreError> {
    crate::core::process::remove_file(path)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), ()> {
    crate::core::process::write_private(path, bytes).map_err(|_| ())
}

fn signal_group(pgid: i32, signal: i32) -> bool {
    crate::core::process::signal_group(pgid, signal)
}

fn lease_error(error: LeaseError) -> CoreError {
    match error {
        LeaseError::Busy => CoreError::Busy,
        LeaseError::Conflict => CoreError::Conflict,
        _ => CoreError::Failed,
    }
}

fn pct(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
