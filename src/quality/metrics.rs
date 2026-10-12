//! Показатели узла по последним пробам. Числа бюджета — исходные, не измеренные.

use super::score::Candidate;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    pub node: String,
    pub at_unix_ms: i64,
    pub ok: bool,
    pub delay_ms: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeStats {
    pub node: String,
    pub samples: usize,
    pub loss_percent: u8,
    pub median_delay_ms: Option<u32>,
    pub jitter_ms: Option<u32>,
    pub last_ok_unix_ms: Option<i64>,
}

pub const WINDOW: usize = 20;

pub fn stats(samples: &[Sample], node: &str) -> NodeStats {
    let mut indexed: Vec<(usize, &Sample)> = samples
        .iter()
        .enumerate()
        .filter(|(_, sample)| sample.node == node)
        .collect();
    indexed.sort_by_key(|(index, sample)| (sample.at_unix_ms, *index));
    if indexed.len() > WINDOW {
        let skip = indexed.len() - WINDOW;
        indexed.drain(..skip);
    }
    let window: Vec<&Sample> = indexed.into_iter().map(|(_, sample)| sample).collect();
    if window.is_empty() {
        return NodeStats {
            node: node.to_owned(),
            samples: 0,
            loss_percent: 100,
            median_delay_ms: None,
            jitter_ms: None,
            last_ok_unix_ms: None,
        };
    }
    let failed = window.iter().filter(|sample| !sample.ok).count();
    let loss_percent = ((failed * 100) / window.len()) as u8;
    let mut delays: Vec<u32> = window
        .iter()
        .filter(|sample| sample.ok)
        .filter_map(|sample| sample.delay_ms)
        .collect();
    delays.sort_unstable();
    let median_delay_ms = lower_median(&delays);
    let jitter_ms = median_delay_ms.map(|median| {
        let mut deviations: Vec<u32> = delays.iter().map(|delay| delay.abs_diff(median)).collect();
        deviations.sort_unstable();
        lower_median(&deviations).unwrap_or(0)
    });
    let last_ok_unix_ms = window
        .iter()
        .filter(|sample| sample.ok)
        .map(|sample| sample.at_unix_ms)
        .max();
    NodeStats {
        node: node.to_owned(),
        samples: window.len(),
        loss_percent,
        median_delay_ms,
        jitter_ms,
        last_ok_unix_ms,
    }
}

pub fn to_candidate(stats: &NodeStats, country: Option<String>, protocol: &str) -> Candidate {
    Candidate {
        node: stats.node.clone(),
        country,
        protocol: protocol.to_owned(),
        delay_ms: stats.median_delay_ms,
        loss_percent: stats.loss_percent,
        last_ok_unix_ms: stats.last_ok_unix_ms,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionMode {
    Pinned,
    Failover,
    Adaptive,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Conditions {
    pub metered: bool,
    pub on_battery: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub min_interval_ms: i64,
    pub max_per_hour: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeDecision {
    Allow,
    TooSoon,
    HourlyBudget,
}

pub fn budget(mode: SelectionMode, conditions: Conditions) -> Budget {
    let (interval, per_hour): (i64, u32) = match mode {
        SelectionMode::Pinned => (300_000, 12),
        SelectionMode::Failover => (60_000, 60),
        SelectionMode::Adaptive => (30_000, 120),
    };
    if conditions.metered || conditions.on_battery {
        Budget {
            min_interval_ms: interval.saturating_mul(4),
            max_per_hour: per_hour / 4,
        }
    } else {
        Budget {
            min_interval_ms: interval,
            max_per_hour: per_hour,
        }
    }
}

pub fn allow_probe(history_unix_ms: &[i64], now_unix_ms: i64, budget: Budget) -> ProbeDecision {
    if let Some(last) = history_unix_ms.iter().copied().max()
        && now_unix_ms.saturating_sub(last) < budget.min_interval_ms
    {
        return ProbeDecision::TooSoon;
    }
    let recent = history_unix_ms
        .iter()
        .filter(|stamp| now_unix_ms.saturating_sub(**stamp) < 3_600_000)
        .count();
    if recent >= usize::try_from(budget.max_per_hour).unwrap_or(usize::MAX) {
        ProbeDecision::HourlyBudget
    } else {
        ProbeDecision::Allow
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceSample {
    pub rss_kib: u64,
    pub cpu_ticks: u64,
}

pub fn parse_resources(stat: &str, statm: &str, page_kib: u64) -> Option<ResourceSample> {
    let tail = stat.rsplit_once(')')?.1;
    let fields: Vec<&str> = tail.split_whitespace().collect();
    // Поля 14 и 15 всей строки — это 12-е и 13-е после последней скобки имени.
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    let resident: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(ResourceSample {
        rss_kib: resident.saturating_mul(page_kib),
        cpu_ticks: utime.saturating_add(stime),
    })
}

fn lower_median(sorted: &[u32]) -> Option<u32> {
    if sorted.is_empty() {
        return None;
    }
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        Some(sorted[mid - 1])
    } else {
        Some(sorted[mid])
    }
}
