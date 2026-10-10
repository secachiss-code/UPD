//! Смена страны меняет только ось REGION. Приложение не перезапускается.

use crate::profiles::VerificationValue;

use super::geo::RegionDecision;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Invalidation {
    Keep,
    RegionUnknown,
    RegionMismatch { expected: String, observed: String },
}

pub fn on_region_change(
    preset_country: &str,
    _previous: &RegionDecision,
    current: &RegionDecision,
) -> Invalidation {
    match current {
        RegionDecision::Agreed { country, .. } if country == preset_country => Invalidation::Keep,
        RegionDecision::Agreed { country, .. } => Invalidation::RegionMismatch {
            expected: preset_country.to_owned(),
            observed: country.clone(),
        },
        RegionDecision::Disagree { .. }
        | RegionDecision::Insufficient { .. }
        | RegionDecision::Stale => Invalidation::RegionUnknown,
    }
}

pub fn axis_after(invalidation: &Invalidation) -> VerificationValue {
    match invalidation {
        Invalidation::Keep => VerificationValue::Verified,
        Invalidation::RegionUnknown => VerificationValue::Unknown,
        Invalidation::RegionMismatch { .. } => VerificationValue::Blocked,
    }
}
