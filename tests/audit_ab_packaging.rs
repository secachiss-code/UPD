//! T09, U01, U02: packaging detect/support tokens.

#[path = "ab_support/mod.rs"]
mod ab_support;

use std::path::Path;

use cm::app::packaging::detect::{Packaging, detect, read_head};
use cm::app::packaging::policy::{self, Limit, Reason, Support};
use cm::env::private::{DataState, IpcPolicy};

use ab_support::with_cm_conf;

#[test]
fn t09_u01_u02_packaging() {
    with_cm_conf(|| {
        assert!(matches!(
            policy::support(&Packaging::Native, IpcPolicy::Private, DataState::Existing),
            Support::Supported
        ));
        assert!(matches!(
            policy::support(&Packaging::Native, IpcPolicy::Shared, DataState::Existing),
            Support::Partial(_)
        ));
        assert!(matches!(
            policy::support(
                &Packaging::AppImage,
                IpcPolicy::Private,
                DataState::Existing
            ),
            Support::Partial(_)
        ));
        assert!(matches!(
            policy::support(
                &Packaging::Launcher,
                IpcPolicy::Private,
                DataState::Existing
            ),
            Support::Unsupported(Reason::ExecutorOutsideNetns)
        ));
        assert_eq!(
            detect(
                Path::new("/usr/bin/flatpak"),
                &["run".into(), "org.example.App".into()],
                None
            ),
            Packaging::Flatpak {
                app_id: Some("org.example.App".into())
            }
        );
        assert_eq!(
            detect(Path::new("/usr/bin/flatpak"), &["list".into()], None),
            Packaging::Native
        );
        assert_eq!(
            detect(Path::new("/snap/bin/tool"), &[], None),
            Packaging::Snap
        );
        assert_eq!(
            detect(
                Path::new("/usr/bin/systemd-run"),
                &["--user".into(), "app".into()],
                None
            ),
            Packaging::Launcher
        );
        let mut head = *b"\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        head[8..11].copy_from_slice(b"AI\x02");
        assert_eq!(
            detect(Path::new("/tmp/x"), &[], Some(&head)),
            Packaging::AppImage
        );
        head[8..11].copy_from_slice(b"\0\0\0");
        assert_eq!(
            detect(Path::new("/tmp/x"), &[], Some(&head)),
            Packaging::Native
        );
        assert_eq!(policy::limit_token(Limit::SessionBus), "limit_session_bus");
        assert_eq!(
            policy::reason_token(Reason::NotVerified),
            "unsupported_not_verified"
        );
        let dir = cm::common::contract_fixtures::TempDirGuard::new("cm-ab-u01").unwrap();
        let short = dir.path().join("short");
        std::fs::write(&short, b"x").unwrap();
        assert!(read_head(&short).is_none());
    });
}
