//! Регион выхода: согласие источников, применимость окружения, реакция на смену страны.

pub mod apply;
pub mod geo;
pub mod invalidate;

pub use apply::{EnvIssue, check_preset, normalize_locale, parse_locale_list};
pub use geo::{MAX_AGE_MS, MIN_SOURCES, Observation, RegionDecision, decide, region_axis};
pub use invalidate::{Invalidation, axis_after, on_region_change};
