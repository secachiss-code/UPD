//! Contract every core (mihomo, xray, the in-memory fake) implements.
//!
//! Readiness is three independent facts. None of them implies the next one,
//! and none of them is a promise about the user's traffic.

use std::fmt;

/// Upper bound for a config blob held by an adapter. The bytes are never formatted.
pub const MAX_CONFIG_BYTES: usize = 1 << 20;

/// What this process can ask a core to do. Protocol tables live in the I03 matrix;
/// this struct only names the core and the lifecycle features the adapter claims.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreCapabilities {
    pub core: &'static str,
    pub version: &'static str,
    pub reload_without_restart: bool,
    pub delay_probe: bool,
}

/// Opaque config. `Debug` does not include the bytes.
#[derive(Clone, Eq, PartialEq)]
pub struct CoreConfig {
    bytes: Vec<u8>,
}

impl CoreConfig {
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self, CoreError> {
        let bytes = bytes.into();
        if bytes.is_empty() || bytes.len() > MAX_CONFIG_BYTES {
            return Err(CoreError::InvalidConfig);
        }
        Ok(Self { bytes })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for CoreConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoreConfig")
            .field("len", &self.bytes.len())
            .field("body", &"[REDACTED]")
            .finish()
    }
}

/// The core's local API accepts requests. Routes may still be absent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApiState {
    Down,
    ApiReady,
}

/// Instance routes exist. The remote node may still be unreachable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteState {
    Down,
    RouteReady,
}

/// A probe reached the selected remote. This is not a statement about host traffic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteState {
    Unknown,
    Unreachable,
    RemoteReachable,
}

/// Three axes, stored and reported separately.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreReadiness {
    pub api: ApiState,
    pub route: RouteState,
    pub remote: RemoteState,
}

impl CoreReadiness {
    pub const DOWN: Self = Self {
        api: ApiState::Down,
        route: RouteState::Down,
        remote: RemoteState::Unknown,
    };

    pub const fn api_ready() -> Self {
        Self {
            api: ApiState::ApiReady,
            route: RouteState::Down,
            remote: RemoteState::Unknown,
        }
    }
}

/// Counters from this instance's core API. They are not added to host or TUN interface counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CoreStatistics {
    pub upload_bytes: u64,
    pub download_bytes: u64,
    pub connections: u32,
}

/// Fixed failure classes. A variant never carries config text, a URL, or a secret.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreError {
    InvalidConfig,
    Unsupported,
    NotRunning,
    Busy,
    Timeout,
    ApiUnavailable,
    RouteUnavailable,
    RemoteUnreachable,
    Conflict,
    Failed,
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidConfig => "core config rejected",
            Self::Unsupported => "core operation is not supported",
            Self::NotRunning => "core is not running",
            Self::Busy => "core is busy",
            Self::Timeout => "core operation timed out",
            Self::ApiUnavailable => "core API is unavailable",
            Self::RouteUnavailable => "core routes are unavailable",
            Self::RemoteUnreachable => "core remote is unreachable",
            Self::Conflict => "core operation conflicts with its current state",
            Self::Failed => "core operation failed",
        })
    }
}

impl std::error::Error for CoreError {}

/// Lifecycle of one core instance. Implementors must not put config bytes into errors.
pub trait CoreAdapter {
    fn capabilities(&self) -> CoreCapabilities;
    fn validate(&mut self, config: &CoreConfig) -> Result<(), CoreError>;
    fn start(&mut self, config: &CoreConfig) -> Result<CoreReadiness, CoreError>;
    fn stop(&mut self) -> Result<(), CoreError>;
    fn reload(&mut self, config: &CoreConfig) -> Result<CoreReadiness, CoreError>;
    fn health(&mut self) -> Result<CoreReadiness, CoreError>;
    fn statistics(&mut self) -> Result<CoreStatistics, CoreError>;

    /// Pid of the running core, when this adapter spawned one. The default is none.
    fn core_pid(&self) -> Option<u32> {
        None
    }

    /// Application TUN for this worker. Cores without a TUN mode ignore it.
    fn set_tunnel_net(&mut self, _tunnel: crate::net::TunnelNet) {}
}
