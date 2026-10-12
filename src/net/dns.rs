//! DNS ядра берётся из политики туннеля. Отдельного DNS-процесса нет.

use super::host_plan::BOOTSTRAP_DNS;
use super::plan::{NetError, TunnelNet};
use super::policy::TunnelPolicy;
use crate::profiles::{FakeIpPolicy, Ipv6Policy};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DnsRuntime {
    pub resolver: String,
    pub enhanced_mode: &'static str,
    pub fake_ip_range: Option<String>,
    pub nameservers: Vec<String>,
    pub bootstrap: Vec<String>,
}

pub fn dns_runtime(policy: &TunnelPolicy, tunnel: &TunnelNet) -> Result<DnsRuntime, NetError> {
    if policy.ipv6 == Ipv6Policy::Pass {
        return Err(NetError::Unsupported);
    }
    let (enhanced_mode, fake_ip_range) = if policy.fake_ip == FakeIpPolicy::Allowed {
        ("fake-ip", Some(format!("198.19.{}.0/24", tunnel.index)))
    } else {
        ("redir-host", None)
    };
    Ok(DnsRuntime {
        resolver: tunnel.dns_addr.clone(),
        enhanced_mode,
        fake_ip_range,
        nameservers: vec![
            "https://1.1.1.1/dns-query".to_owned(),
            "https://dns.google/dns-query".to_owned(),
        ],
        bootstrap: BOOTSTRAP_DNS
            .iter()
            .map(|item| (*item).to_owned())
            .collect(),
    })
}

pub fn resolv_conf(tunnel: &TunnelNet) -> String {
    format!("nameserver {}\noptions edns0\n", tunnel.dns_addr)
}
