//! In-memory [`CoreAdapter`](super::adapter::CoreAdapter) with injected failures.
//!
//! I06, I07 and I09 can drive this until a real core is available. It does not
//! open sockets, create devices, or keep the caller's config in error text.

use super::adapter::{
    ApiState, CoreAdapter, CoreCapabilities, CoreConfig, CoreError, CoreReadiness, CoreStatistics,
    RemoteState, RouteState,
};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FakeOp {
    Validate,
    Start,
    Stop,
    Reload,
    Health,
    Statistics,
}

/// Scripted adapter. `fail_next` applies to one later call of that operation and
/// leaves the previous readiness in place.
#[derive(Debug)]
pub struct FakeAdapter {
    running: bool,
    readiness: CoreReadiness,
    statistics: CoreStatistics,
    generation: u64,
    fail_next: VecDeque<(FakeOp, CoreError)>,
}

impl Default for FakeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeAdapter {
    pub fn new() -> Self {
        Self {
            running: false,
            readiness: CoreReadiness::DOWN,
            statistics: CoreStatistics::default(),
            generation: 0,
            fail_next: VecDeque::new(),
        }
    }

    pub fn fail_next(&mut self, op: FakeOp, error: CoreError) {
        self.fail_next.push_back((op, error));
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Test hook: route readiness does not change the API or remote axes.
    pub fn set_route(&mut self, route: RouteState) -> Result<(), CoreError> {
        if !self.running {
            return Err(CoreError::NotRunning);
        }
        self.readiness.route = route;
        Ok(())
    }

    /// Test hook: remote readiness does not change the API or route axes.
    pub fn set_remote(&mut self, remote: RemoteState) -> Result<(), CoreError> {
        if !self.running {
            return Err(CoreError::NotRunning);
        }
        self.readiness.remote = remote;
        Ok(())
    }

    pub fn set_statistics(&mut self, statistics: CoreStatistics) {
        self.statistics = statistics;
    }

    fn take_failure(&mut self, op: FakeOp) -> Result<(), CoreError> {
        if let Some(index) = self
            .fail_next
            .iter()
            .position(|(candidate, _)| *candidate == op)
        {
            let (_, error) = self.fail_next.remove(index).expect("index exists");
            return Err(error);
        }
        Ok(())
    }
}

impl CoreAdapter for FakeAdapter {
    fn capabilities(&self) -> CoreCapabilities {
        CoreCapabilities {
            core: "fake",
            version: "0",
            reload_without_restart: true,
            delay_probe: true,
        }
    }

    fn validate(&mut self, config: &CoreConfig) -> Result<(), CoreError> {
        self.take_failure(FakeOp::Validate)?;
        if config.as_bytes().is_empty() {
            return Err(CoreError::InvalidConfig);
        }
        Ok(())
    }

    fn start(&mut self, config: &CoreConfig) -> Result<CoreReadiness, CoreError> {
        self.take_failure(FakeOp::Start)?;
        self.validate(config)?;
        if self.running {
            return Err(CoreError::Conflict);
        }
        self.running = true;
        self.generation = self.generation.saturating_add(1);
        self.readiness = CoreReadiness::api_ready();
        Ok(self.readiness)
    }

    fn stop(&mut self) -> Result<(), CoreError> {
        self.take_failure(FakeOp::Stop)?;
        self.running = false;
        self.readiness = CoreReadiness::DOWN;
        self.statistics = CoreStatistics::default();
        Ok(())
    }

    fn reload(&mut self, config: &CoreConfig) -> Result<CoreReadiness, CoreError> {
        self.take_failure(FakeOp::Reload)?;
        if !self.running {
            return Err(CoreError::NotRunning);
        }
        self.validate(config)?;
        self.generation = self.generation.saturating_add(1);
        self.readiness.api = ApiState::ApiReady;
        Ok(self.readiness)
    }

    fn health(&mut self) -> Result<CoreReadiness, CoreError> {
        self.take_failure(FakeOp::Health)?;
        if !self.running {
            return Ok(CoreReadiness::DOWN);
        }
        Ok(self.readiness)
    }

    fn statistics(&mut self) -> Result<CoreStatistics, CoreError> {
        self.take_failure(FakeOp::Statistics)?;
        if !self.running {
            return Ok(CoreStatistics::default());
        }
        Ok(self.statistics)
    }
}
