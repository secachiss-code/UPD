//! D01, D03: tunnel policy parse/read and DNS runtime / attach_tun_dns.

#[path = "ab_support/mod.rs"]
mod ab_support;

use std::fs;
use std::os::unix::fs::symlink;

use cm::common::contract_fixtures::TempDirGuard;
use cm::common::sys::euid;
use cm::controller::dispatch::handle;
use cm::controller::owner::{owned, policy_path};
use cm::core::mihomo::config::{ConfigError, attach_tun, attach_tun_dns};
use cm::net::dns::{DnsRuntime, dns_runtime};
use cm::net::policy::{self, FailureMode, Lifetime, TunnelPolicy};
use cm::net::tunnel_net;
use cm::profiles::{FakeIpPolicy, Ipv6Policy};
use serde_json::json;

use ab_support::{frame, reply, rig, self_peer, with_cm_conf};

#[test]
#[ignore = "дефект D01: parse(b\"[]\") возвращает Ok(TunnelPolicy по умолчанию) вместо Err"]
fn d01_parse_empty_json_array_rejected() {
    with_cm_conf(|| {
        assert!(
            policy::parse(b"[]").is_err(),
            "получено {:?}",
            policy::parse(b"[]")
        );
    });
}

#[test]
fn d01_policy_parse_and_read() {
    with_cm_conf(|| {
        let default = policy::parse(b"{}").unwrap();
        assert_eq!(
            default,
            TunnelPolicy {
                ipv6: Ipv6Policy::Block,
                fake_ip: FakeIpPolicy::Forbidden,
                lifetime: Lifetime::Pinned,
                failure: FailureMode::Block,
            }
        );
        let custom =
            policy::parse(br#"{"lifetime":"refcount","failure":"failover","fake_ip":"allowed"}"#)
                .unwrap();
        assert_eq!(custom.lifetime, Lifetime::Refcount);
        assert_eq!(custom.failure, FailureMode::Failover);
        assert_eq!(custom.fake_ip, FakeIpPolicy::Allowed);
        assert_eq!(custom.ipv6, Ipv6Policy::Block);
        assert!(policy::parse(br#"{"dns":"8.8.8.8"}"#).is_err());
        assert!(policy::parse(br#"{"ipv6":"maybe"}"#).is_err());
        let dir = TempDirGuard::new("cm-ab-d01").unwrap();
        let o = owned(dir.path(), euid(), "t1").unwrap();
        assert_eq!(cm::controller::owner::read_policy(&o).unwrap(), default);
        fs::create_dir_all(policy_path(&o).parent().unwrap()).unwrap();
        fs::write(policy_path(&o), br#"{"extra":true}"#).unwrap();
        assert_eq!(
            cm::controller::owner::read_policy(&o).unwrap_err(),
            cm::controller::protocol::ControlError::InvalidConfig
        );
        fs::remove_file(policy_path(&o)).unwrap();
        symlink("/etc/passwd", policy_path(&o)).unwrap();
        assert_eq!(
            cm::controller::owner::read_policy(&o).unwrap_err(),
            cm::controller::protocol::ControlError::InvalidConfig
        );
        fs::remove_file(policy_path(&o)).unwrap();
        fs::write(policy_path(&o), vec![b' '; 1025]).unwrap();
        assert_eq!(
            cm::controller::owner::read_policy(&o).unwrap_err(),
            cm::controller::protocol::ControlError::TooLarge
        );
        let rig =
            ab_support::rig_with_query(dir.path(), false, cm::net::query::FixedQuery::default());
        fs::write(policy_path(&o), br#"{"ipv6":"pass"}"#).unwrap();
        let peer = self_peer();
        reply(&handle(
            &frame(
                "d01prep1",
                json!({"type":"instance_prepare","instance":"t1"}),
            ),
            &peer.identity,
            &rig.deps,
        ));
        let ans = reply(&handle(
            &frame(
                "d01net01",
                json!({"type":"net_apply","instance":"t1","generation":1}),
            ),
            &peer.identity,
            &rig.deps,
        ));
        assert_eq!(ans["code"], "unsupported");
        assert!(rig.exec.lock().unwrap().ran.is_empty());
    });
}

#[test]
fn d03_dns_runtime_and_attach() {
    with_cm_conf(|| {
        let tunnel = tunnel_net(3, 1000).unwrap();
        let default = dns_runtime(&TunnelPolicy::default(), &tunnel).unwrap();
        assert_eq!(
            default,
            DnsRuntime {
                resolver: "198.18.3.2".to_owned(),
                enhanced_mode: "redir-host",
                fake_ip_range: None,
                nameservers: vec![
                    "https://1.1.1.1/dns-query".to_owned(),
                    "https://dns.google/dns-query".to_owned(),
                ],
                bootstrap: vec!["1.1.1.1".to_owned(), "8.8.8.8".to_owned()],
            }
        );
        let mut allowed = TunnelPolicy::default();
        allowed.fake_ip = FakeIpPolicy::Allowed;
        let fake = dns_runtime(&allowed, &tunnel).unwrap();
        assert_eq!(fake.enhanced_mode, "fake-ip");
        assert_eq!(fake.fake_ip_range, Some("198.19.3.0/24".to_owned()));
        let mut v6 = TunnelPolicy::default();
        v6.ipv6 = Ipv6Policy::Pass;
        assert!(dns_runtime(&v6, &tunnel).is_err());
        let doc = json!({"mode":"direct","ipv6":false});
        let plain = attach_tun(&doc, &tunnel).unwrap();
        let dns = attach_tun_dns(&doc, &tunnel, &default).unwrap();
        assert_eq!(plain, dns);
        let parsed: serde_json::Value = serde_json::from_slice(&dns).unwrap();
        assert_eq!(parsed["dns"]["enhanced-mode"], "redir-host");
        assert_eq!(parsed["profile"]["store-fake-ip"], false);
        let bad = json!({"mode":"direct","profile":{}});
        assert_eq!(
            attach_tun_dns(&bad, &tunnel, &default).err(),
            Some(ConfigError::Forbidden)
        );
    });
}
