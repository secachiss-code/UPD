//! Legacy host VPN as [`CoreAdapter`](super::adapter::CoreAdapter) instance `host-legacy`.
//!
//! The legacy unit still starts mihomo from `{home}/bin/mihomo` via [`crate::vpn::core_bin`],
//! which is a writable directory. [`super::unit::CORE_BIN`] (`/var/lib/cm/cores/mihomo/mihomo`)
//! is not applied to that unit. This wrapper does not change `core_bin()` or the unit file.
//!
//! Host service lifecycle stays on `vpn::start` / `vpn::stop` / `vpn::reload`, which call
//! `dispatch_*` into the unchanged bodies. Trait `start` / `stop` / `reload` do not invoke
//! systemctl: [`CoreConfig`](super::adapter::CoreConfig) is not the cm [`Config`](crate::common::Config)
//! and must not mutate the host unit. `vpn::reload` talks to the live API socket and can change
//! the running core, so the trait returns [`CoreError::Unsupported`] instead of calling it.

use super::adapter::{
    CoreAdapter, CoreCapabilities, CoreConfig, CoreError, CoreReadiness, CoreStatistics,
};
use super::mihomo;

/// Stable id of the legacy host instance. It is not a path and not a unit name.
pub const INSTANCE_ID: &str = "host-legacy";

/// Adapter over the already-running host VPN. It does not install or stop the unit.
#[derive(Clone, Copy, Debug, Default)]
pub struct LegacyHost;

impl CoreAdapter for LegacyHost {
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
        let text = std::str::from_utf8(config.as_bytes()).map_err(|_| CoreError::InvalidConfig)?;
        crate::vpn::validate_candidate_body(text).map_err(|_| CoreError::InvalidConfig)
    }

    fn start(&mut self, _config: &CoreConfig) -> Result<CoreReadiness, CoreError> {
        Err(CoreError::Unsupported)
    }

    fn stop(&mut self) -> Result<(), CoreError> {
        Err(CoreError::Unsupported)
    }

    fn reload(&mut self, _config: &CoreConfig) -> Result<CoreReadiness, CoreError> {
        Err(CoreError::Unsupported)
    }

    fn health(&mut self) -> Result<CoreReadiness, CoreError> {
        if crate::vpn::snapshot().running {
            Ok(CoreReadiness::api_ready())
        } else {
            Ok(CoreReadiness::DOWN)
        }
    }

    fn statistics(&mut self) -> Result<CoreStatistics, CoreError> {
        let snapshot = crate::vpn::snapshot();
        Ok(CoreStatistics {
            upload_bytes: snapshot.up,
            download_bytes: snapshot.down,
            connections: u32::try_from(snapshot.conns).unwrap_or(u32::MAX),
        })
    }
}

/// User path for `vpn::start`. Calls the unchanged body, not `vpn::start`.
pub fn dispatch_start(config: &crate::common::Config) -> Result<(), String> {
    crate::vpn::start_service_body(config)
}

/// User path for `vpn::stop`. Calls the unchanged body, not `vpn::stop`.
pub fn dispatch_stop() -> Result<(), String> {
    crate::vpn::stop_service_body()
}

/// User path for `vpn::reload`. Calls the unchanged body, not `vpn::reload`.
pub fn dispatch_reload() -> Result<(), String> {
    crate::vpn::reload_service_body()
}

/// User path for profile validation. Calls the unchanged body, not the wrapper.
pub fn dispatch_validate(config: &str) -> Result<(), String> {
    crate::vpn::validate_candidate_body(config)
}
