//! M13, M14: constraints before scoring, deterministic ranking, no switching on noise.

use cm::quality::{
    Candidate, Constraints, MIN_DWELL_MS, MIN_GAIN_PERCENT, Ranked, Rejection, StayReason, Switch,
    decide, evaluate, score,
};
use serde_json::Value;

fn candidates() -> Vec<Candidate> {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/pack/quality.json")).unwrap();
    fixture["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| Candidate {
            node: item["node"].as_str().unwrap().to_owned(),
            country: item["country"].as_str().map(str::to_owned),
            protocol: item["protocol"].as_str().unwrap().to_owned(),
            delay_ms: item["delay_ms"].as_u64().map(|value| value as u32),
            loss_percent: item["loss_percent"].as_u64().unwrap() as u8,
            last_ok_unix_ms: None,
        })
        .collect()
}

fn free() -> Constraints {
    Constraints {
        countries: None,
        protocols: None,
        max_delay_ms: None,
    }
}

fn ranked(outcome: &[Ranked]) -> Vec<(&str, u32)> {
    outcome
        .iter()
        .map(|item| (item.node.as_str(), item.score))
        .collect()
}

fn rejection<'a>(rejected: &'a [(String, Rejection)], node: &str) -> Option<&'a Rejection> {
    rejected
        .iter()
        .find(|(name, _)| name == node)
        .map(|(_, reason)| reason)
}

#[test]
fn m13_constraints_first_then_score() {
    assert_eq!(
        (score(100, 0), score(100, 2), score(80, 1)),
        (100, 200, 130)
    );
    assert_eq!(score(u32::MAX, 100), u32::MAX);
    let all = candidates();
    let outcome = evaluate(&all[..3], &free());
    assert_eq!(ranked(&outcome.ranked), [("a", 120), ("b", 130)]);
    assert_eq!(
        outcome.rejected,
        [("c".to_owned(), Rejection::NoMeasurement)]
    );

    let german = Constraints {
        countries: Some(vec!["DE".to_owned()]),
        ..free()
    };
    let outcome = evaluate(&all, &german);
    assert_eq!(ranked(&outcome.ranked), [("a", 120)]);
    assert_eq!(
        rejection(&outcome.rejected, "b"),
        Some(&Rejection::CountryNotAllowed)
    );
    assert_eq!(
        rejection(&outcome.rejected, "d"),
        Some(&Rejection::CountryUnknown)
    );
    assert_eq!(
        rejection(&outcome.rejected, "e"),
        Some(&Rejection::TooLossy)
    );

    let vless = Constraints {
        protocols: Some(vec!["vless".to_owned()]),
        max_delay_ms: Some(100),
        ..free()
    };
    let outcome = evaluate(&all, &vless);
    assert_eq!(
        rejection(&outcome.rejected, "b"),
        Some(&Rejection::ProtocolNotAllowed)
    );
    assert_eq!(rejection(&outcome.rejected, "a"), Some(&Rejection::TooSlow));
    // 20 % loss passes, 21 % does not.
    assert_eq!(ranked(&outcome.ranked), [("d", 90 + 20 * 50)]);

    // When nothing fits, nothing is offered: the constraint is not relaxed.
    let nowhere = Constraints {
        countries: Some(vec!["JP".to_owned()]),
        ..free()
    };
    let outcome = evaluate(&all, &nowhere);
    assert!(outcome.ranked.is_empty());
    assert_eq!(outcome.rejected.len(), all.len());
}

#[test]
fn m13_ranking_is_deterministic() {
    let node = |name: &str| Candidate {
        node: name.to_owned(),
        country: None,
        protocol: "ss".to_owned(),
        delay_ms: Some(100),
        loss_percent: 0,
        last_ok_unix_ms: None,
    };
    let forward = evaluate(&[node("x"), node("m"), node("a")], &free());
    let backward = evaluate(&[node("a"), node("m"), node("x")], &free());
    assert_eq!(
        ranked(&forward.ranked),
        [("a", 100), ("m", 100), ("x", 100)]
    );
    assert_eq!(forward.ranked, backward.ranked);
}

fn rank(items: &[(&str, u32)]) -> Vec<Ranked> {
    items
        .iter()
        .map(|(node, score)| Ranked {
            node: (*node).to_owned(),
            score: *score,
        })
        .collect()
}

#[test]
fn m14_no_switch_on_noise() {
    assert_eq!((MIN_GAIN_PERCENT, MIN_DWELL_MS), (20, 60_000));
    let stay = |reason| Switch::Stay { reason };
    let to = |node: &str| Switch::To {
        node: node.to_owned(),
    };
    let long_ago = 0;
    let now = 1_000_000;
    assert_eq!(
        decide(Some("a"), &[], long_ago, now),
        stay(StayReason::NoCandidates)
    );
    assert_eq!(
        decide(None, &[], long_ago, now),
        stay(StayReason::NoCandidates)
    );
    let list = rank(&[("best", 80), ("cur", 100)]);
    assert_eq!(decide(None, &list, now - 1000, now), to("best"));
    // The current node dropped out of the list: leave it at once.
    assert_eq!(decide(Some("gone"), &list, now - 1000, now), to("best"));
    assert_eq!(
        decide(Some("best"), &list, long_ago, now),
        stay(StayReason::AlreadyBest)
    );
    let small = rank(&[("best", 85), ("cur", 100)]);
    assert_eq!(
        decide(Some("cur"), &small, long_ago, now),
        stay(StayReason::GainTooSmall)
    );
    assert_eq!(decide(Some("cur"), &list, long_ago, now), to("best"));
    let half = rank(&[("best", 50), ("cur", 100)]);
    assert_eq!(
        decide(Some("cur"), &half, now - 59_000, now),
        stay(StayReason::DwellNotElapsed)
    );
    assert_eq!(decide(Some("cur"), &half, now - 60_000, now), to("best"));
}
