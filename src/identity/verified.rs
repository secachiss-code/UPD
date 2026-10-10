//! Реестр версий, проверенных лабораторией тем же механизмом, что BI.L1.

use super::engine::{Brand, EngineInfo};
use super::model::Strategy;
use serde::Deserialize;
use std::sync::OnceLock;

const REGISTRY_JSON: &str = include_str!("../../data/identity-verified.json");

#[derive(Debug, Deserialize)]
struct VerifiedRow {
    brand: Brand,
    major: u32,
    strategy: Strategy,
    evidence: String,
    note: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verification {
    Verified { evidence: String, note: String },
    Unverified,
}

pub fn status(engine: &EngineInfo, strategy: Strategy) -> Verification {
    match registry().iter().find(|row| {
        row.brand == engine.brand && row.major == engine.major && row.strategy == strategy
    }) {
        Some(row) => Verification::Verified {
            evidence: row.evidence.clone(),
            note: row.note.clone(),
        },
        None => Verification::Unverified,
    }
}

fn registry() -> &'static [VerifiedRow] {
    static ROWS: OnceLock<Vec<VerifiedRow>> = OnceLock::new();
    ROWS.get_or_init(|| serde_json::from_str(REGISTRY_JSON).expect("identity-verified.json"))
}
