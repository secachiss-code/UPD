//! Страна выхода только по свежим наблюдениям. Имя узла сюда не передаётся.

use std::collections::BTreeMap;

use crate::profiles::VerificationValue;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub source: String,
    pub country: String,
    pub at_unix_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegionDecision {
    Agreed { country: String, sources: usize },
    Disagree { countries: Vec<String> },
    Insufficient { fresh: usize },
    Stale,
}

pub const MIN_SOURCES: usize = 2;
pub const MAX_AGE_MS: i64 = 15 * 60 * 1000;

pub fn decide(observations: &[Observation], now_unix_ms: i64) -> RegionDecision {
    let mut latest: BTreeMap<&str, &Observation> = BTreeMap::new();
    let mut saw_stale = false;
    for observation in observations {
        if !valid_country(&observation.country) || observation.at_unix_ms > now_unix_ms {
            continue;
        }
        let age = now_unix_ms.saturating_sub(observation.at_unix_ms);
        if age > MAX_AGE_MS {
            saw_stale = true;
            continue;
        }
        match latest.get(observation.source.as_str()) {
            Some(previous) if previous.at_unix_ms >= observation.at_unix_ms => {}
            _ => {
                latest.insert(observation.source.as_str(), observation);
            }
        }
    }
    if latest.is_empty() {
        return if saw_stale && !observations.is_empty() {
            RegionDecision::Stale
        } else {
            RegionDecision::Insufficient { fresh: 0 }
        };
    }
    if latest.len() < MIN_SOURCES {
        return RegionDecision::Insufficient {
            fresh: latest.len(),
        };
    }
    let mut countries: Vec<String> = latest.values().map(|item| item.country.clone()).collect();
    countries.sort();
    countries.dedup();
    if countries.len() == 1 {
        RegionDecision::Agreed {
            country: countries.remove(0),
            sources: latest.len(),
        }
    } else {
        RegionDecision::Disagree { countries }
    }
}

pub fn region_axis(decision: &RegionDecision, preset_country: &str) -> VerificationValue {
    match decision {
        RegionDecision::Agreed { country, .. } if country == preset_country => {
            VerificationValue::Verified
        }
        RegionDecision::Agreed { .. } => VerificationValue::Blocked,
        RegionDecision::Disagree { .. } => VerificationValue::Partial,
        RegionDecision::Insufficient { .. } | RegionDecision::Stale => VerificationValue::Unknown,
    }
}

fn valid_country(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 2 && bytes.iter().all(|byte| byte.is_ascii_uppercase())
}
