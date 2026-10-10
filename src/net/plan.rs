//! Имена и адреса туннеля выводятся только из его номера.
//!
//! Диапазоны `10.213.0.0/16` и `198.18.0.0/16` проверены лабораторией
//! `tools/packet_flow_lab.sh`: трафик veth попадает в таблицу, где путь один — TUN.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TunnelNet {
    pub index: u8,
    pub netns: String,
    pub veth_host: String,
    pub veth_ns: String,
    pub host_addr: String,
    pub ns_addr: String,
    pub gateway: String,
    pub tun: String,
    pub tun_addr: String,
    pub dns_addr: String,
    pub table: u32,
    pub rule_priority: u32,
    pub mtu: u32,
    pub owner_uid: u32,
}

pub const MAX_TUNNELS: u8 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetError {
    BadIndex,
    CommandFailed,
    Timeout,
    Drift,
    Busy,
    Unsupported,
}

pub fn tunnel_net(index: u8, owner_uid: u32) -> Result<TunnelNet, NetError> {
    if index >= MAX_TUNNELS {
        return Err(NetError::BadIndex);
    }
    let index_text = index.to_string();
    Ok(TunnelNet {
        index,
        netns: format!("cm-{index_text}"),
        veth_host: format!("cmv{index_text}h"),
        veth_ns: format!("cmv{index_text}n"),
        host_addr: format!("10.213.{index_text}.1/30"),
        ns_addr: format!("10.213.{index_text}.2/30"),
        gateway: format!("10.213.{index_text}.1"),
        tun: format!("cmtun{index_text}"),
        tun_addr: format!("198.18.{index_text}.1/30"),
        dns_addr: format!("198.18.{index_text}.2"),
        table: 100 + u32::from(index),
        rule_priority: 1000 + u32::from(index),
        mtu: 1400,
        owner_uid,
    })
}
