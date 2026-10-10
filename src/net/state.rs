//! Конечный автомат туннеля. Трафик идёт только в Up и Degraded.
//!
//! Остановка туннеля с живыми сессиями приложений (Q12): вызывающий оставляет сеть
//! (`NetRevert` не вызывается). Приложение остаётся в `Blocked`, а не выходит напрямую.

use crate::profiles::VerificationValue;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TunnelState {
    Stopped,
    Starting,
    Up,
    Degraded,
    Blocked,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TunnelEvent {
    StartRequested,
    NetReady,
    CoreReady,
    RemoteLost,
    RemoteBack,
    CoreDown,
    DriftFound,
    StopRequested,
    Repaired,
}

pub fn next(state: TunnelState, event: TunnelEvent) -> TunnelState {
    use TunnelEvent as E;
    use TunnelState as S;
    match (state, event) {
        (_, E::StopRequested) => S::Stopped,
        (S::Stopped, E::StartRequested) | (S::Failed, E::StartRequested) => S::Starting,
        (S::Starting, E::CoreReady) | (S::Blocked, E::CoreReady) => S::Up,
        (S::Starting, E::CoreDown) => S::Failed,
        (S::Starting, E::DriftFound)
        | (S::Up, E::CoreDown)
        | (S::Up, E::DriftFound)
        | (S::Degraded, E::CoreDown)
        | (S::Degraded, E::DriftFound) => S::Blocked,
        (S::Up, E::RemoteLost) => S::Degraded,
        (S::Degraded, E::RemoteBack) => S::Up,
        (S::Blocked, E::Repaired) => S::Starting,
        _ => state,
    }
}

pub fn net_axis(state: TunnelState) -> VerificationValue {
    match state {
        TunnelState::Up => VerificationValue::Verified,
        TunnelState::Degraded => VerificationValue::Partial,
        TunnelState::Blocked => VerificationValue::Blocked,
        TunnelState::Failed => VerificationValue::Error,
        TunnelState::Stopped | TunnelState::Starting => VerificationValue::Unknown,
    }
}

pub fn passes_traffic(state: TunnelState) -> bool {
    matches!(state, TunnelState::Up | TunnelState::Degraded)
}
