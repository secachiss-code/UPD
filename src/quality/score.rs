//! Ограничения не ослабляются, даже если после них не остаётся узлов.

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub node: String,
    pub country: Option<String>,
    pub protocol: String,
    pub delay_ms: Option<u32>,
    pub loss_percent: u8,
    pub last_ok_unix_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Constraints {
    pub countries: Option<Vec<String>>,
    pub protocols: Option<Vec<String>>,
    pub max_delay_ms: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejection {
    NoMeasurement,
    CountryNotAllowed,
    CountryUnknown,
    ProtocolNotAllowed,
    TooSlow,
    TooLossy,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ranked {
    pub node: String,
    pub score: u32,
}

pub struct Outcome {
    pub ranked: Vec<Ranked>,
    pub rejected: Vec<(String, Rejection)>,
}

pub fn evaluate(candidates: &[Candidate], constraints: &Constraints) -> Outcome {
    let mut ranked = Vec::new();
    let mut rejected = Vec::new();
    for candidate in candidates {
        if let Some(reason) = reject(candidate, constraints) {
            rejected.push((candidate.node.clone(), reason));
        } else {
            let delay = candidate.delay_ms.unwrap_or(0);
            ranked.push(Ranked {
                node: candidate.node.clone(),
                score: score(delay, candidate.loss_percent),
            });
        }
    }
    ranked.sort_by(|left, right| {
        left.score
            .cmp(&right.score)
            .then_with(|| left.node.cmp(&right.node))
    });
    Outcome { ranked, rejected }
}

pub fn score(delay_ms: u32, loss_percent: u8) -> u32 {
    delay_ms.saturating_add(u32::from(loss_percent).saturating_mul(50))
}

fn reject(candidate: &Candidate, constraints: &Constraints) -> Option<Rejection> {
    let Some(delay) = candidate.delay_ms else {
        return Some(Rejection::NoMeasurement);
    };
    if let Some(countries) = &constraints.countries {
        match &candidate.country {
            None => return Some(Rejection::CountryUnknown),
            Some(country) if !countries.iter().any(|allowed| allowed == country) => {
                return Some(Rejection::CountryNotAllowed);
            }
            Some(_) => {}
        }
    }
    if let Some(protocols) = &constraints.protocols
        && !protocols
            .iter()
            .any(|protocol| protocol == &candidate.protocol)
    {
        return Some(Rejection::ProtocolNotAllowed);
    }
    if let Some(max) = constraints.max_delay_ms
        && delay > max
    {
        return Some(Rejection::TooSlow);
    }
    if candidate.loss_percent > 20 {
        return Some(Rejection::TooLossy);
    }
    None
}
