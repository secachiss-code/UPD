//! T05, T06: private dirs, session parse, environment profile.

#[path = "ab_support/mod.rs"]
mod ab_support;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use cm::app::env::{environment_profile, parse_session, session_file};
use cm::common::contract_fixtures::TempDirGuard;
use cm::common::sys::euid;
use cm::env::private::{
    self, DataState, IpcPolicy, check_dir, data_dirs, env_overrides, prepare, runtime_dir,
    valid_name,
};
use cm::profiles::{EnvironmentPreset, Id, SCHEMA_VERSION};
use serde_json::json;

use ab_support::with_cm_conf;

#[test]
fn t05_t06_private_and_session() {
    with_cm_conf(|| {
        for ok in ["work", "s0123456789ab", "a-1"] {
            assert!(valid_name(ok));
        }
        for bad in ["", "-a", "A", "a_b", "a/b", &"a".repeat(33)] {
            assert!(!valid_name(bad));
        }
        assert!(
            data_dirs(
                std::path::Path::new("/home/u"),
                "work",
                DataState::Existing,
                "s1"
            )
            .is_none()
        );
        let dir = TempDirGuard::new("cm-ab-t05").unwrap();
        let home = dir.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let isolated = data_dirs(&home, "work", DataState::Isolated, "s1").unwrap();
        assert!(isolated.root.ends_with("data/work"));
        let fresh = data_dirs(&home, "work", DataState::Fresh, "s1").unwrap();
        assert!(fresh.root.ends_with("fresh-s1"));
        let env = env_overrides(&isolated);
        assert_eq!(env.len(), 4);
        assert!(!env.contains_key("HOME"));
        assert_eq!(
            runtime_dir(std::path::Path::new("/run/user/1000"), "s1"),
            std::path::PathBuf::from("/run/user/1000/cm/s1")
        );
        prepare(&isolated).unwrap();
        prepare(&isolated).unwrap();
        for sub in [
            &isolated.config,
            &isolated.data,
            &isolated.cache,
            &isolated.state,
        ] {
            assert!(check_dir(sub, euid()));
        }
        fs::set_permissions(&isolated.root, fs::Permissions::from_mode(0o750)).unwrap();
        assert!(!check_dir(&isolated.root, euid()));
        let parsed =
            parse_session(br#"{"timezone":"Europe/Berlin","locale":"de_DE.UTF-8"}"#).unwrap();
        assert!(parsed.data.is_none());
        assert_eq!(parsed.ipc, IpcPolicy::Shared);
        assert!(
            parse_session(br#"{"timezone":"Europe/Berlin","locale":"de DE","extra":1}"#).is_err()
        );
        assert!(parse_session(br#"{"timezone":"../etc/passwd","locale":"de_DE.UTF-8"}"#).is_err());
        assert!(parse_session(
            br#"{"timezone":"Europe/Berlin","locale":"de_DE.UTF-8","data":{"profile":"A","state":"isolated"}}"#
        )
        .is_err());
        let preset = EnvironmentPreset {
            schema_version: SCHEMA_VERSION,
            id: Id::new("preset").unwrap(),
            timezone: "UTC".into(),
            locale: "en_US.UTF-8".into(),
            languages: vec!["en".into()],
        };
        let sf = session_file(&preset, None, IpcPolicy::Shared);
        assert_eq!(sf.timezone, "UTC");
        let profile = environment_profile(Id::new("p1").unwrap(), Id::new("s1").unwrap(), &preset);
        assert_eq!(profile.timezone, "UTC");
        let frame: serde_json::Value = serde_json::from_slice(&serde_json::to_vec(&json!({
            "v":1,"id":"x","op":{"type":"app_launch","instance":"a1","generation":1,"program":"/bin/true","args":[]}
        })).unwrap()).unwrap();
        assert!(frame["op"].get("session").is_none());
    });
}
