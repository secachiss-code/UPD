//! C04: the polkit authorizer. One test, because it changes the process environment.

mod pack_support;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use cm::common::contract_fixtures::TempDirGuard;
use cm::controller::actions::{ACTION_WORKER, Authorizer, PolkitAuthorizer};
use cm::controller::protocol::ControlError;

#[test]
fn c04_pkcheck_subject_comes_from_the_peer() {
    let dir = TempDirGuard::new("cm-i06-c04").unwrap();
    let log = dir.path().join("args");
    let fake = dir.path().join("pkcheck");
    fs::write(
        &fake,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" > {}\nexit 1\n",
            log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let peer = pack_support::self_peer();
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    // SAFETY: this file holds one test, so no other thread reads the environment.
    unsafe {
        std::env::remove_var("CM_STATE_DIR");
        std::env::remove_var("UPD_STATE_DIR");
        std::env::set_var("CM_HELPER_ALLOW", "1");
        std::env::set_var(
            "PATH",
            format!("{}:{}", dir.path().display(), old_path.to_string_lossy()),
        );
    }
    // Outside test mode the override is ignored and pkcheck decides.
    assert_eq!(
        PolkitAuthorizer.authorize(&peer.identity, ACTION_WORKER),
        Err(ControlError::Denied)
    );
    let args = fs::read_to_string(&log).unwrap();
    let subject = format!(
        "{},{},{}",
        peer.identity.pid, peer.identity.start_time, peer.identity.uid
    );
    assert_eq!(
        args.trim(),
        format!("--action-id io.github.cm.worker --process {subject} --allow-user-interaction")
    );
    fs::remove_file(&log).unwrap();
    // SAFETY: as above.
    unsafe {
        std::env::set_var("CM_STATE_DIR", dir.path());
    }
    assert_eq!(
        PolkitAuthorizer.authorize(&peer.identity, ACTION_WORKER),
        Ok(())
    );
    assert!(
        !log.exists(),
        "pkcheck must not run under the test override"
    );
    // SAFETY: as above.
    unsafe {
        std::env::remove_var("CM_STATE_DIR");
        std::env::remove_var("CM_HELPER_ALLOW");
        std::env::set_var("PATH", old_path);
    }
}
