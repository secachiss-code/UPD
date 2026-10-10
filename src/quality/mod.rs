//! Выбор узла: сначала ограничения, потом оценка, без переключения от шума.

pub mod score;
pub mod switch;

pub use score::{Candidate, Constraints, Outcome, Ranked, Rejection, evaluate, score};
pub use switch::{MIN_DWELL_MS, MIN_GAIN_PERCENT, StayReason, Switch, decide};
