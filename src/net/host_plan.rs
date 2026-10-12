//! Имена и номера туннеля хоста. Один на машину, сразу за диапазоном приложений.

use serde::{Deserialize, Serialize};

use super::plan::TunnelNet;

pub const HOST_INSTANCE: &str = "host";
pub const HOST_INDEX: u8 = 64;
pub const MAX_UPLINKS: usize = 8;
pub const MAX_UPLINK_ADDRS: usize = 32;
pub const BOOTSTRAP_DNS: [&str; 2] = ["1.1.1.1", "8.8.8.8"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostNet {
    pub tun: String,
    pub tun_addr: String,
    pub dns_addr: String,
    pub sink: String,
    pub table: u32,
    pub sink_table: u32,
    pub rule_uplink: u32,
    pub rule_uplink_addr: u32,
    pub rule_main: u32,
    pub rule_tun: u32,
    pub rule_sink: u32,
    pub mtu: u32,
    pub owner_uid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Uplink {
    pub dev: String,
    pub addrs: Vec<String>,
}

pub fn host_net(owner_uid: u32) -> HostNet {
    HostNet {
        tun: "cmtunh".to_owned(),
        tun_addr: "198.18.64.1/30".to_owned(),
        dns_addr: "198.18.64.2".to_owned(),
        sink: "cmsink".to_owned(),
        table: 90,
        sink_table: 91,
        rule_uplink: 1900,
        rule_uplink_addr: 1950,
        rule_main: 1990,
        rule_tun: 2000,
        rule_sink: 2001,
        mtu: 1400,
        owner_uid,
    }
}

impl HostNet {
    /// Тот же `TunnelNet`, что у ядра приложения: пустые поля netns ядру хоста не нужны.
    pub fn as_tunnel(&self) -> TunnelNet {
        TunnelNet {
            index: HOST_INDEX,
            netns: String::new(),
            veth_host: String::new(),
            veth_ns: String::new(),
            host_addr: String::new(),
            ns_addr: String::new(),
            gateway: String::new(),
            tun: self.tun.clone(),
            tun_addr: self.tun_addr.clone(),
            dns_addr: self.dns_addr.clone(),
            table: self.table,
            rule_priority: self.rule_tun,
            mtu: self.mtu,
            owner_uid: self.owner_uid,
        }
    }
}
