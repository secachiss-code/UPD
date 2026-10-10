//! resolv.conf внутри netns приложения. Адрес — из TunnelNet, не из константы.

use super::plan::TunnelNet;

pub fn resolv_conf(tunnel: &TunnelNet) -> String {
    format!("nameserver {}\noptions edns0\n", tunnel.dns_addr)
}
