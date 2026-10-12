//! T08, R04: prepared session data dirs (controller accepts only user-prepared trees).

#[path = "ab_support/mod.rs"]
mod ab_support;

use cm::env::private::{DataState, check_dir, data_dirs};

use ab_support::with_cm_conf;

#[test]
fn t08_session_data_dirs_must_exist() {
    with_cm_conf(|| {
        let dirs = data_dirs(
            std::path::Path::new("/home/u"),
            "work",
            DataState::Isolated,
            "s1",
        )
        .unwrap();
        let env = cm::env::private::env_overrides(&dirs);
        assert_eq!(env.len(), 4);
        assert!(!check_dir(&dirs.root, 1000));
    });
}

#[test]
fn r04_nul_timezone_rejected_before_spawn() {
    with_cm_conf(|| {
        assert!(!cm::identity::tzdata::valid_zone_name("Europe\u{0}Berlin"));
    });
}
