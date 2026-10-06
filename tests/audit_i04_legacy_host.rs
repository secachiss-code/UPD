//! I04.T02.d: legacy host VPN behind CoreAdapter, without changing the unit.

use cm::common::contract_fixtures::{self, EnvGuard, TempDirGuard};
use cm::core::legacy_host::{self, LegacyHost};
use cm::core::{CoreAdapter, CoreConfig, CoreError, CoreReadiness};
use std::os::unix::fs::PermissionsExt;

#[test]
fn legacy_validate_hides_marker_and_dispatch_uses_the_same_body() {
    let _lock = contract_fixtures::isolation_lock();
    let home = TempDirGuard::new("cm-a2-vpn").unwrap();
    std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut env = EnvGuard::new();
    env.remove("UPD_CONF");
    env.remove("UPD_STATE_DIR");
    env.remove("UPD_VPN_ETC");
    env.remove("UPD_VPN_HOME");
    env.set("CM_VPN_HOME", home.path());

    let mut host = LegacyHost;
    let non_utf8 = CoreConfig::from_bytes(b"\xffsecret-marker-a2".to_vec()).unwrap();
    let error = host.validate(&non_utf8).unwrap_err();
    assert_eq!(error, CoreError::InvalidConfig);
    assert!(!error.to_string().contains("secret-marker-a2"));
    assert!(!format!("{error:?}").contains("secret-marker-a2"));

    let rejected = CoreConfig::from_bytes(b"secret-marker-a2\n".to_vec()).unwrap();
    let error = host.validate(&rejected).unwrap_err();
    assert_eq!(error, CoreError::InvalidConfig);
    assert_eq!(error.to_string(), "core config rejected");
    assert!(!error.to_string().contains("secret-marker-a2"));

    let missing = legacy_host::dispatch_validate("secret-marker-a2\n").unwrap_err();
    assert!(
        !missing.contains("secret-marker-a2"),
        "missing-binary error leaked the marker"
    );

    if let Some(binary) = std::env::var_os("CM_TEST_MIHOMO") {
        let bin_dir = home.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::os::unix::fs::symlink(&binary, bin_dir.join("mihomo")).unwrap();
        let rejected_by_core = legacy_host::dispatch_validate("{\nsecret-marker-a2\n").unwrap_err();
        assert!(
            rejected_by_core.contains("mihomo не принял конфиг:")
                || rejected_by_core.contains("mihomo rejected the config:"),
            "historical prefix missing"
        );
        let bytes = CoreConfig::from_bytes(b"{\nsecret-marker-a2\n".to_vec()).unwrap();
        let error = host.validate(&bytes).unwrap_err();
        assert_eq!(error.to_string(), "core config rejected");
        assert!(!error.to_string().contains("secret-marker-a2"));
    }

    assert_eq!(host.health().unwrap(), CoreReadiness::DOWN);
    let stats = host.statistics().unwrap();
    assert_eq!(stats.upload_bytes, 0);
    assert_eq!(stats.download_bytes, 0);
    assert_eq!(stats.connections, 0);
    assert_eq!(host.start(&rejected).unwrap_err(), CoreError::Unsupported);
    assert_eq!(host.stop().unwrap_err(), CoreError::Unsupported);
    assert_eq!(
        host.reload(&rejected).unwrap_err().to_string(),
        "core operation is not supported"
    );
    assert_eq!(host.capabilities().core, "mihomo");
    assert_eq!(legacy_host::INSTANCE_ID, "host-legacy");
}
