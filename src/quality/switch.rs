//! Узел меняется только при заметном и устойчивом выигрыше.

use super::score::Ranked;

pub const MIN_GAIN_PERCENT: u32 = 20;
pub const MIN_DWELL_MS: i64 = 60_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Switch {
    Stay { reason: StayReason },
    To { node: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StayReason {
    AlreadyBest,
    GainTooSmall,
    DwellNotElapsed,
    NoCandidates,
}

pub fn decide(
    current: Option<&str>,
    ranked: &[Ranked],
    last_switch_unix_ms: i64,
    now_unix_ms: i64,
) -> Switch {
    let Some(best) = ranked.first() else {
        return Switch::Stay {
            reason: StayReason::NoCandidates,
        };
    };
    let Some(current_name) = current else {
        return Switch::To {
            node: best.node.clone(),
        };
    };
    let Some(current_rank) = ranked.iter().find(|item| item.node == current_name) else {
        return Switch::To {
            node: best.node.clone(),
        };
    };
    if current_rank.node == best.node {
        return Switch::Stay {
            reason: StayReason::AlreadyBest,
        };
    }
    if now_unix_ms.saturating_sub(last_switch_unix_ms) < MIN_DWELL_MS {
        return Switch::Stay {
            reason: StayReason::DwellNotElapsed,
        };
    }
    let gain = current_rank
        .score
        .saturating_sub(best.score)
        .saturating_mul(100)
        / current_rank.score.max(1);
    if gain < MIN_GAIN_PERCENT {
        Switch::Stay {
            reason: StayReason::GainTooSmall,
        }
    } else {
        Switch::To {
            node: best.node.clone(),
        }
    }
}
