//! Выбор узла. Закреплённый узел не заменяется, живые сессии узел не меняют.

use super::metrics::SelectionMode;
use super::score::Outcome;
use super::switch::{StayReason, Switch, decide};

pub struct SelectInput<'a> {
    pub mode: SelectionMode,
    pub current: Option<&'a str>,
    pub outcome: &'a Outcome,
    pub live_sessions: usize,
    pub last_switch_unix_ms: i64,
    pub last_failed_switch_unix_ms: Option<i64>,
    pub now_unix_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeepReason {
    Pinned,
    CurrentUsable,
    SessionsPinned,
    Cooldown,
    AlreadyBest,
    GainTooSmall,
    DwellNotElapsed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchReason {
    NoCurrent,
    CurrentRejected,
    BetterNode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockReason {
    NoCandidates,
    PinnedNodeRejected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Keep(KeepReason),
    Switch { node: String, reason: SwitchReason },
    Block(BlockReason),
}

pub const FAILED_SWITCH_COOLDOWN_MS: i64 = 300_000;

pub fn choose(input: &SelectInput<'_>) -> Choice {
    let usable = input
        .current
        .is_some_and(|name| input.outcome.ranked.iter().any(|item| item.node == name));
    if input.mode == SelectionMode::Pinned {
        return if usable {
            Choice::Keep(KeepReason::Pinned)
        } else {
            Choice::Block(BlockReason::PinnedNodeRejected)
        };
    }
    let Some(best) = input.outcome.ranked.first() else {
        return Choice::Block(BlockReason::NoCandidates);
    };
    let Some(current) = input.current else {
        return Choice::Switch {
            node: best.node.clone(),
            reason: SwitchReason::NoCurrent,
        };
    };
    if !usable {
        return Choice::Switch {
            node: best.node.clone(),
            reason: SwitchReason::CurrentRejected,
        };
    }
    if input.mode == SelectionMode::Failover {
        return Choice::Keep(KeepReason::CurrentUsable);
    }
    if input.live_sessions > 0 {
        return Choice::Keep(KeepReason::SessionsPinned);
    }
    if input
        .last_failed_switch_unix_ms
        .is_some_and(|failed| input.now_unix_ms.saturating_sub(failed) < FAILED_SWITCH_COOLDOWN_MS)
    {
        return Choice::Keep(KeepReason::Cooldown);
    }
    match decide(
        Some(current),
        &input.outcome.ranked,
        input.last_switch_unix_ms,
        input.now_unix_ms,
    ) {
        Switch::To { node } => Choice::Switch {
            node,
            reason: SwitchReason::BetterNode,
        },
        Switch::Stay { reason } => match reason {
            StayReason::AlreadyBest => Choice::Keep(KeepReason::AlreadyBest),
            StayReason::GainTooSmall => Choice::Keep(KeepReason::GainTooSmall),
            StayReason::DwellNotElapsed => Choice::Keep(KeepReason::DwellNotElapsed),
            StayReason::NoCandidates => Choice::Block(BlockReason::NoCandidates),
        },
    }
}

pub fn reason_token(choice: &Choice) -> &'static str {
    match choice {
        Choice::Keep(KeepReason::Pinned) => "pinned",
        Choice::Keep(KeepReason::CurrentUsable) => "current_usable",
        Choice::Keep(KeepReason::SessionsPinned) => "sessions_pinned",
        Choice::Keep(KeepReason::Cooldown) => "cooldown",
        Choice::Keep(KeepReason::AlreadyBest) => "already_best",
        Choice::Keep(KeepReason::GainTooSmall) => "gain_too_small",
        Choice::Keep(KeepReason::DwellNotElapsed) => "dwell_not_elapsed",
        Choice::Switch {
            reason: SwitchReason::NoCurrent,
            ..
        } => "no_current",
        Choice::Switch {
            reason: SwitchReason::CurrentRejected,
            ..
        } => "current_rejected",
        Choice::Switch {
            reason: SwitchReason::BetterNode,
            ..
        } => "better_node",
        Choice::Block(BlockReason::NoCandidates) => "no_candidates",
        Choice::Block(BlockReason::PinnedNodeRejected) => "pinned_node_rejected",
    }
}
