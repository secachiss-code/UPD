//! Q01, Q02, Z08: metrics, select, degradation scenarios.

#[path = "ab_support/mod.rs"]
mod ab_support;

use cm::quality::metrics::{
    self, Conditions, ProbeDecision, Sample, SelectionMode, allow_probe, budget, parse_resources,
    stats, to_candidate,
};
use cm::quality::score::{Candidate, Constraints, Rejection, evaluate};
use cm::quality::select::{
    self, BlockReason, Choice, KeepReason, SelectInput, SwitchReason, choose, reason_token,
};
use cm::quality::switch::MIN_DWELL_MS;

use ab_support::{Xorshift, with_cm_conf};

#[test]
fn q01_metrics_and_budget() {
    with_cm_conf(|| {
        let samples: Vec<Sample> = vec![
            Sample {
                node: "n".into(),
                at_unix_ms: 1,
                ok: true,
                delay_ms: Some(100),
            },
            Sample {
                node: "n".into(),
                at_unix_ms: 2,
                ok: true,
                delay_ms: Some(120),
            },
            Sample {
                node: "n".into(),
                at_unix_ms: 3,
                ok: true,
                delay_ms: Some(110),
            },
            Sample {
                node: "n".into(),
                at_unix_ms: 4,
                ok: true,
                delay_ms: Some(300),
            },
            Sample {
                node: "n".into(),
                at_unix_ms: 5,
                ok: false,
                delay_ms: None,
            },
        ];
        let s = stats(&samples, "n");
        assert_eq!(s.samples, 5);
        assert_eq!(s.loss_percent, 20);
        assert_eq!(s.median_delay_ms, Some(110));
        assert_eq!(s.jitter_ms, Some(10));
        assert_eq!(stats(&[], "n").loss_percent, 100);
        let mut many = samples.clone();
        for i in 0..25 {
            many.push(Sample {
                node: "n".into(),
                at_unix_ms: 100 + i,
                ok: true,
                delay_ms: Some(100),
            });
        }
        assert_eq!(stats(&many, "n").samples, 20);
        assert_eq!(
            budget(SelectionMode::Pinned, Conditions::default()).min_interval_ms,
            300_000
        );
        assert_eq!(
            budget(SelectionMode::Adaptive, Conditions::default()).max_per_hour,
            120
        );
        assert_eq!(
            allow_probe(
                &[0],
                29_999,
                budget(SelectionMode::Adaptive, Conditions::default())
            ),
            ProbeDecision::TooSoon
        );
        assert_eq!(
            allow_probe(
                &[0],
                30_000,
                budget(SelectionMode::Adaptive, Conditions::default())
            ),
            ProbeDecision::Allow
        );
        let parsed = parse_resources(
            "12 (a b) c) S 1 12 12 0 -1 4194560 100 0 0 0 7 5 0 0 20 0 1 0 100 0 0",
            "100 250 30 1 0 20 0",
            4,
        )
        .unwrap();
        assert_eq!(parsed.cpu_ticks, 12);
        assert_eq!(parsed.rss_kib, 1000);
        assert!(parse_resources("no paren", "100 250", 4).is_none());
        let st = stats(&samples[..4], "n");
        let c = to_candidate(&st, None, "ss");
        assert_eq!(c.delay_ms, st.median_delay_ms);
        assert_eq!(c.loss_percent, st.loss_percent);
    });
}

#[test]
fn q02_z08_select_and_property() {
    with_cm_conf(|| {
        let ranked = evaluate(
            &[Candidate {
                node: "a".into(),
                country: Some("DE".into()),
                protocol: "ss".into(),
                delay_ms: Some(50),
                loss_percent: 0,
                last_ok_unix_ms: None,
            }],
            &Constraints {
                countries: None,
                protocols: None,
                max_delay_ms: None,
            },
        );
        let input = SelectInput {
            mode: SelectionMode::Pinned,
            current: Some("a"),
            outcome: &ranked,
            live_sessions: 0,
            last_switch_unix_ms: 0,
            last_failed_switch_unix_ms: None,
            now_unix_ms: 0,
        };
        assert!(matches!(choose(&input), Choice::Keep(KeepReason::Pinned)));
        let input = SelectInput {
            mode: SelectionMode::Adaptive,
            current: Some("a"),
            outcome: &ranked,
            live_sessions: 1,
            last_switch_unix_ms: 0,
            last_failed_switch_unix_ms: None,
            now_unix_ms: 0,
        };
        assert!(matches!(
            choose(&input),
            Choice::Keep(KeepReason::SessionsPinned)
        ));
        let empty = evaluate(
            &[],
            &Constraints {
                countries: None,
                protocols: None,
                max_delay_ms: None,
            },
        );
        let block = SelectInput {
            mode: SelectionMode::Pinned,
            current: Some("x"),
            outcome: &empty,
            live_sessions: 0,
            last_switch_unix_ms: 0,
            last_failed_switch_unix_ms: None,
            now_unix_ms: 0,
        };
        assert!(matches!(
            choose(&block),
            Choice::Block(BlockReason::PinnedNodeRejected)
        ));
        assert!(!reason_token(&Choice::Keep(KeepReason::GainTooSmall)).is_empty());
        let mut rng = Xorshift(42);
        for _ in 0..10_000 {
            let a = 100 + (rng.next() % 21) as u32;
            let b = 100 + (rng.next() % 21) as u32;
            let outcome = evaluate(
                &[
                    Candidate {
                        node: "a".into(),
                        country: None,
                        protocol: "ss".into(),
                        delay_ms: Some(a),
                        loss_percent: 0,
                        last_ok_unix_ms: None,
                    },
                    Candidate {
                        node: "b".into(),
                        country: None,
                        protocol: "ss".into(),
                        delay_ms: Some(b),
                        loss_percent: 0,
                        last_ok_unix_ms: None,
                    },
                ],
                &Constraints {
                    countries: None,
                    protocols: None,
                    max_delay_ms: None,
                },
            );
            let mut last_switch = 0i64;
            let mut switches = 0u32;
            for step in 0..120 {
                let now = step * 30_000;
                let choice = choose(&SelectInput {
                    mode: SelectionMode::Adaptive,
                    current: Some("a"),
                    outcome: &outcome,
                    live_sessions: 0,
                    last_switch_unix_ms: last_switch,
                    last_failed_switch_unix_ms: None,
                    now_unix_ms: now,
                });
                if matches!(
                    choice,
                    Choice::Switch {
                        reason: SwitchReason::BetterNode,
                        ..
                    }
                ) {
                    switches += 1;
                    last_switch = now;
                }
            }
            assert_eq!(switches, 0);
        }
        let _ = MIN_DWELL_MS;
        let lossy = evaluate(
            &[Candidate {
                node: "a".into(),
                country: None,
                protocol: "ss".into(),
                delay_ms: None,
                loss_percent: 50,
                last_ok_unix_ms: None,
            }],
            &Constraints {
                countries: None,
                protocols: None,
                max_delay_ms: None,
            },
        );
        assert!(lossy.ranked.is_empty());
    });
}
