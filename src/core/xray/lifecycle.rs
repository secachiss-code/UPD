//! Процесс Xray: тот же контракт, что у mihomo, без перезагрузки на месте.

use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::controller::drop::{self, RunAs};
use crate::controller::harden::ChildLimits;
use crate::core::adapter::{
    ApiState, CoreAdapter, CoreCapabilities, CoreConfig, CoreError, CoreReadiness, CoreStatistics,
    RemoteState, RouteState,
};
use crate::core::instance::{InstanceId, InstanceRoot};
use crate::core::leases::{Lease, LeaseError, LeaseRegistry, ResourceKind};
use crate::core::process;

const PORT_WAIT: Duration = Duration::from_secs(5);
const TEST_WAIT: Duration = Duration::from_secs(20);

pub struct XrayWorker {
    id: InstanceId,
    root: InstanceRoot,
    lease_dir: PathBuf,
    binary: PathBuf,
    registry: Option<LeaseRegistry>,
    port: Option<Lease>,
    config_path: Option<PathBuf>,
    child: Option<Child>,
    readiness: CoreReadiness,
    run_as: Option<RunAs>,
}

impl XrayWorker {
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
            config_path: None,
            child: None,
            readiness: CoreReadiness::DOWN,
            run_as: None,
        }
    }

    pub fn set_run_as(&mut self, run_as: RunAs) {
        self.run_as = Some(run_as);
    }

    /// Проверить новый конфиг и, только если он принят, остановить текущий процесс и запустить его заново.
    pub fn restart(&mut self, config: &CoreConfig) -> Result<CoreReadiness, CoreError> {
        self.validate(config)?;
        if self.child.is_some() || self.group_alive() {
            CoreAdapter::stop(self)?;
        }
        self.start(config)
    }

    fn group_alive(&mut self) -> bool {
        self.reap();
        let Some(child) = self.child.as_ref() else {
            return false;
        };
        process::signal_group(child.id() as i32, 0)
    }

    fn reap(&mut self) {
        if let Some(child) = self.child.as_mut()
            && matches!(child.try_wait(), Ok(Some(_)) | Err(_))
        {
            self.readiness = CoreReadiness::DOWN;
        }
    }

    fn ensure_registry(&mut self) -> Result<(), CoreError> {
        if self.registry.is_some() {
            return Ok(());
        }
        fs::create_dir_all(&self.lease_dir).map_err(|_| CoreError::Failed)?;
        let registry = match LeaseRegistry::open(&self.lease_dir) {
            Ok(registry) => registry,
            Err(_) => LeaseRegistry::create(&self.lease_dir).map_err(lease_error)?,
        };
        self.registry = Some(registry);
        Ok(())
    }

    fn allocate_port(&mut self) -> Result<u16, CoreError> {
        self.ensure_registry()?;
        let holder = self.id.as_str().to_owned();
        let lease = self
            .registry
            .as_ref()
            .ok_or(CoreError::Failed)?
            .allocate(&holder, ResourceKind::Port)
            .map_err(lease_error)?;
        let port = lease.value.parse::<u16>().map_err(|_| CoreError::Failed)?;
        self.port = Some(lease);
        Ok(port)
    }

    fn release_port(&mut self) -> Result<(), CoreError> {
        let Some(lease) = self.port.clone() else {
            return Ok(());
        };
        if self.registry.is_none() {
            self.port = None;
            return Ok(());
        }
        let holder = self.id.as_str().to_owned();
        let registry = self.registry.as_ref().ok_or(CoreError::Failed)?;
        match registry.release(&holder, lease.kind, &lease.value) {
            Ok(()) | Err(LeaseError::Conflict) => {
                self.port = None;
                Ok(())
            }
            Err(error) => Err(lease_error(error)),
        }
    }

    fn test_file(&self, bytes: &[u8]) -> Result<(), CoreError> {
        let dirs = self.root.create(&self.id).map_err(|_| CoreError::Failed)?;
        let path = dirs.cache.join("xray-test.json");
        process::write_private(&path, bytes)?;
        let mut child = Command::new(&self.binary)
            .args(["run", "-test", "-c"])
            .arg(&path)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| CoreError::Failed)?;
        let deadline = Instant::now() + TEST_WAIT;
        let result = loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    break if status.success() {
                        Ok(())
                    } else {
                        Err(CoreError::InvalidConfig)
                    };
                }
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(CoreError::Timeout);
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => break Err(CoreError::Failed),
            }
        };
        let _ = process::remove_file(&path);
        result
    }

    fn spawn(&mut self, config: &Path) -> Result<(), CoreError> {
        let mut command = Command::new(&self.binary);
        command
            .args(["run", "-c"])
            .arg(config)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(run_as) = self.run_as.clone() {
            drop::worker_pre_exec(&mut command, run_as, &[], ChildLimits::default());
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
        self.child = Some(command.spawn().map_err(|_| CoreError::Failed)?);
        Ok(())
    }

    fn listen_port(&self) -> Result<u16, CoreError> {
        self.port
            .as_ref()
            .and_then(|lease| lease.value.parse().ok())
            .ok_or(CoreError::Failed)
    }
}

impl CoreAdapter for XrayWorker {
    fn capabilities(&self) -> CoreCapabilities {
        CoreCapabilities {
            core: "xray",
            version: "26.3.27",
            reload_without_restart: false,
            delay_probe: false,
        }
    }

    fn validate(&mut self, config: &CoreConfig) -> Result<(), CoreError> {
        let document: Value =
            serde_json::from_slice(config.as_bytes()).map_err(|_| CoreError::InvalidConfig)?;
        let bytes = prepared(&document, 20_000).map_err(|_| CoreError::InvalidConfig)?;
        self.test_file(&bytes)
    }

    fn start(&mut self, config: &CoreConfig) -> Result<CoreReadiness, CoreError> {
        if self.group_alive() {
            return Err(CoreError::Conflict);
        }
        let document: Value =
            serde_json::from_slice(config.as_bytes()).map_err(|_| CoreError::InvalidConfig)?;
        // Отказ формы — до аренды порта.
        prepared(&document, 1)?;
        let port = self.allocate_port()?;
        let bytes = match prepared(&document, port) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.release_port()?;
                return Err(error);
            }
        };
        let dirs = match self.root.create(&self.id) {
            Ok(dirs) => dirs,
            Err(_) => {
                self.release_port()?;
                return Err(CoreError::Failed);
            }
        };
        let config_path = dirs.config.join("config.json");
        if process::write_private(&config_path, &bytes).is_err() {
            self.release_port()?;
            return Err(CoreError::Failed);
        }
        if self.test_file(&bytes).is_err() {
            let _ = process::remove_file(&config_path);
            self.release_port()?;
            return Err(CoreError::InvalidConfig);
        }
        if self.spawn(&config_path).is_err() {
            let _ = process::remove_file(&config_path);
            self.release_port()?;
            return Err(CoreError::Failed);
        }
        if process::wait_port(port, PORT_WAIT).is_err() {
            CoreAdapter::stop(self)?;
            return Err(CoreError::Timeout);
        }
        self.config_path = Some(config_path);
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
            process::terminate_child(child);
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
            && process::signal_group(pgid, 0)
        {
            error.get_or_insert(CoreError::Failed);
        }
        if let Err(release) = self.release_port() {
            error.get_or_insert(release);
        }
        self.config_path = None;
        self.readiness = CoreReadiness::DOWN;
        match error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn reload(&mut self, _config: &CoreConfig) -> Result<CoreReadiness, CoreError> {
        Err(CoreError::Unsupported)
    }

    fn health(&mut self) -> Result<CoreReadiness, CoreError> {
        self.reap();
        if !self.group_alive() {
            self.readiness = CoreReadiness::DOWN;
            return Ok(CoreReadiness::DOWN);
        }
        let port = self.listen_port()?;
        let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        if std::net::TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_err() {
            self.readiness = CoreReadiness::DOWN;
            return Ok(CoreReadiness::DOWN);
        }
        self.readiness.remote = RemoteState::Unknown;
        Ok(self.readiness)
    }

    fn statistics(&mut self) -> Result<CoreStatistics, CoreError> {
        Err(CoreError::Unsupported)
    }

    fn core_pid(&self) -> Option<u32> {
        self.child.as_ref().map(|child| child.id())
    }
}

impl Drop for XrayWorker {
    fn drop(&mut self) {
        if self.child.is_some() || self.port.is_some() {
            let _ = CoreAdapter::stop(self);
        }
    }
}

fn prepared(document: &Value, port: u16) -> Result<Vec<u8>, CoreError> {
    let inbounds = document
        .get("inbounds")
        .and_then(Value::as_array)
        .ok_or(CoreError::InvalidConfig)?;
    if inbounds.len() != 1 {
        return Err(CoreError::InvalidConfig);
    }
    let inbound = inbounds[0].as_object().ok_or(CoreError::InvalidConfig)?;
    if inbound.get("protocol").and_then(Value::as_str) != Some("mixed") {
        return Err(CoreError::InvalidConfig);
    }
    if inbound.get("listen").and_then(Value::as_str) != Some("127.0.0.1") {
        return Err(CoreError::InvalidConfig);
    }
    let mut document = document.clone();
    let inbound = document
        .get_mut("inbounds")
        .and_then(Value::as_array_mut)
        .and_then(|items| items.first_mut())
        .and_then(Value::as_object_mut)
        .ok_or(CoreError::InvalidConfig)?;
    inbound.insert("port".to_owned(), Value::from(port));
    serde_json::to_vec(&document).map_err(|_| CoreError::InvalidConfig)
}

fn lease_error(error: LeaseError) -> CoreError {
    match error {
        LeaseError::Busy => CoreError::Busy,
        LeaseError::Conflict => CoreError::Conflict,
        _ => CoreError::Failed,
    }
}
