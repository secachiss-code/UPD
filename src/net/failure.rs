//! Решение при отказе. Открытые соединения на другой узел не переносятся.

use super::policy::FailureMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    CoreDown,
    RemoteLost,
    Drift,
    ReloadRejected,
    UplinkLost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Block,
    RestartCore { delay_ms: u64 },
    Failover,
    Repair,
    GiveUp,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FailureState {
    pub window_start_ms: i64,
    pub restarts: u8,
    pub switches: u8,
}

pub const WINDOW_MS: i64 = 600_000;
pub const MAX_RESTARTS: u8 = 5;
pub const MAX_SWITCHES: u8 = 3;
pub const RESTART_DELAYS_MS: [u64; 5] = [0, 1_000, 5_000, 30_000, 120_000];

pub fn decide(
    failure: Failure,
    mode: FailureMode,
    state: &FailureState,
    now_ms: i64,
) -> (Action, FailureState) {
    let mut next = state.clone();
    if now_ms.saturating_sub(next.window_start_ms) >= WINDOW_MS {
        next.window_start_ms = now_ms;
        next.restarts = 0;
        next.switches = 0;
    }
    let action = match failure {
        Failure::CoreDown => {
            if next.restarts < MAX_RESTARTS {
                let delay_ms = RESTART_DELAYS_MS[usize::from(next.restarts)];
                next.restarts += 1;
                Action::RestartCore { delay_ms }
            } else {
                Action::GiveUp
            }
        }
        Failure::RemoteLost => {
            if mode == FailureMode::Failover && next.switches < MAX_SWITCHES {
                next.switches += 1;
                Action::Failover
            } else {
                Action::Block
            }
        }
        Failure::Drift => Action::Repair,
        Failure::ReloadRejected | Failure::UplinkLost => Action::None,
    };
    (action, next)
}
