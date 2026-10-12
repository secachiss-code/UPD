//! F04: failure window, delays, property on random sequences.

#[path = "ab_support/mod.rs"]
mod ab_support;

use cm::net::failure::{
    self, Action, Failure, FailureState, MAX_RESTARTS, RESTART_DELAYS_MS, decide,
};
use cm::net::policy::FailureMode;

use ab_support::{Xorshift, with_cm_conf};

#[test]
fn f04_failure_decide_matrix() {
    with_cm_conf(|| {
        let mut state = FailureState::default();
        let mut now = 0i64;
        let mut delays = Vec::new();
        for _ in 0..5 {
            let (action, next) = decide(Failure::CoreDown, FailureMode::Block, &state, now);
            state = next;
            match action {
                Action::RestartCore { delay_ms } => delays.push(delay_ms),
                _ => panic!("expected restart"),
            }
            now += 1;
        }
        assert_eq!(delays, RESTART_DELAYS_MS.to_vec());
        let (sixth, _) = decide(Failure::CoreDown, FailureMode::Block, &state, now);
        assert_eq!(sixth, Action::GiveUp);
        state = FailureState::default();
        now = 600_000;
        let (action, next) = decide(Failure::CoreDown, FailureMode::Block, &state, now);
        assert_eq!(action, Action::RestartCore { delay_ms: 0 });
        assert_eq!(next.restarts, 1);
        assert_eq!(
            decide(
                Failure::RemoteLost,
                FailureMode::Block,
                &FailureState::default(),
                0
            )
            .0,
            Action::Block
        );
        let mut sw = FailureState::default();
        for _ in 0..3 {
            let (a, n) = decide(Failure::RemoteLost, FailureMode::Failover, &sw, 0);
            assert_eq!(a, Action::Failover);
            sw = n;
        }
        assert_eq!(
            decide(Failure::RemoteLost, FailureMode::Failover, &sw, 0).0,
            Action::Block
        );
        assert_eq!(
            decide(
                Failure::Drift,
                FailureMode::Block,
                &FailureState::default(),
                0
            )
            .0,
            Action::Repair
        );
        assert_eq!(
            decide(
                Failure::ReloadRejected,
                FailureMode::Block,
                &FailureState::default(),
                0
            )
            .0,
            Action::None
        );
    });
}

#[test]
fn f04_failure_property_random() {
    with_cm_conf(|| {
        let mut rng = Xorshift(0xABCD_EF01);
        for _ in 0..10_000 {
            let mut state = FailureState::default();
            let start = (rng.next() % 1_000_000) as i64;
            let mut now = start;
            let mut switches = 0u8;
            for _ in 0..32 {
                let failure = match rng.next() % 6 {
                    0 => Failure::CoreDown,
                    1 => Failure::RemoteLost,
                    2 => Failure::Drift,
                    3 => Failure::ReloadRejected,
                    4 => Failure::UplinkLost,
                    _ => Failure::CoreDown,
                };
                let mode = if rng.next() % 2 == 0 {
                    FailureMode::Block
                } else {
                    FailureMode::Failover
                };
                let (action, next) = decide(failure, mode, &state, now);
                if matches!(action, Action::Failover) {
                    switches = next.switches;
                }
                state = next;
                now += (rng.next() % 120_000) as i64;
                assert!(state.restarts <= MAX_RESTARTS);
                assert!(switches <= 3);
            }
        }
    });
}
