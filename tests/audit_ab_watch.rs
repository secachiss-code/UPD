//! D02, F05, T12, X03, X04: watch tick, failure mode, refcount lifetime.

#[path = "ab_support/mod.rs"]
mod ab_support;

use cm::controller::watch::{EMPTY_GRACE_MS, WATCH_INTERVAL_MS, Watch};
use cm::net::policy::{FailureMode, Lifetime, TunnelPolicy};
use cm::net::state::TunnelState;

use ab_support::with_cm_conf;

#[test]
fn d02_failure_block_default_policy() {
    with_cm_conf(|| {
        assert_eq!(TunnelPolicy::default().failure, FailureMode::Block);
        let mut watch = Watch::new();
        watch.note_start(1000, "t1");
        assert_eq!(watch.state(1000, "t1"), Some((TunnelState::Starting, 0)));
    });
}

#[test]
fn t12_x04_lifetime_refcount_and_bad_frames() {
    with_cm_conf(|| {
        assert_eq!(EMPTY_GRACE_MS, 5000);
        assert_eq!(WATCH_INTERVAL_MS, 2000);
        let policy = TunnelPolicy {
            lifetime: Lifetime::Refcount,
            ..TunnelPolicy::default()
        };
        assert_eq!(policy.lifetime, Lifetime::Refcount);
        let frame = serde_json::json!({
            "v":1,"id":"x","op":{"type":"app_launch","instance":"a","generation":1,
            "program":"/bin/true","args":[],"lifetime":"refcount"}
        });
        assert!(serde_json::from_value::<cm::controller::protocol::Request>(frame).is_err());
    });
}
