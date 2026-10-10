//! Команды создания и удаления сети. Порядок проверен лабораторией: blackhole раньше TUN и правила.

use crate::profiles::Ipv6Policy;

use super::plan::TunnelNet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Program {
    Ip,
    Nft,
    Sysctl,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cmd {
    pub program: Program,
    pub args: Vec<String>,
    pub stdin: Option<String>,
}

pub fn create_commands(tunnel: &TunnelNet, ipv6: Ipv6Policy) -> Vec<Cmd> {
    let mut commands = vec![
        ip(["netns", "add", &tunnel.netns]),
        ip([
            "link",
            "add",
            &tunnel.veth_host,
            "type",
            "veth",
            "peer",
            "name",
            &tunnel.veth_ns,
        ]),
        ip(["link", "set", &tunnel.veth_ns, "netns", &tunnel.netns]),
        ip(["addr", "add", &tunnel.host_addr, "dev", &tunnel.veth_host]),
        ip(["link", "set", &tunnel.veth_host, "up"]),
        ip(["-n", &tunnel.netns, "link", "set", "lo", "up"]),
        ip([
            "-n",
            &tunnel.netns,
            "addr",
            "add",
            &tunnel.ns_addr,
            "dev",
            &tunnel.veth_ns,
        ]),
        ip(["-n", &tunnel.netns, "link", "set", &tunnel.veth_ns, "up"]),
        ip([
            "-n",
            &tunnel.netns,
            "route",
            "add",
            "default",
            "via",
            &tunnel.gateway,
        ]),
    ];
    if ipv6 == Ipv6Policy::Block {
        commands.push(ip([
            "netns",
            "exec",
            &tunnel.netns,
            "sysctl",
            "-qw",
            "net.ipv6.conf.all.disable_ipv6=1",
        ]));
    }
    let user = tunnel.owner_uid.to_string();
    let table = tunnel.table.to_string();
    let priority = tunnel.rule_priority.to_string();
    let mtu = tunnel.mtu.to_string();
    commands.extend([
        ip([
            "tuntap",
            "add",
            "dev",
            &tunnel.tun,
            "mode",
            "tun",
            "user",
            &user,
        ]),
        ip(["addr", "add", &tunnel.tun_addr, "dev", &tunnel.tun]),
        ip(["link", "set", &tunnel.tun, "mtu", &mtu, "up"]),
        ip([
            "route",
            "add",
            "blackhole",
            "default",
            "metric",
            "200",
            "table",
            &table,
        ]),
        ip([
            "route",
            "add",
            "default",
            "dev",
            &tunnel.tun,
            "table",
            &table,
        ]),
        ip([
            "rule",
            "add",
            "iif",
            &tunnel.veth_host,
            "lookup",
            &table,
            "priority",
            &priority,
        ]),
        Cmd {
            program: Program::Sysctl,
            args: vec!["-qw".to_owned(), "net.ipv4.ip_forward=1".to_owned()],
            stdin: None,
        },
    ]);
    commands
}

pub fn destroy_commands(tunnel: &TunnelNet) -> Vec<Cmd> {
    let table = tunnel.table.to_string();
    let priority = tunnel.rule_priority.to_string();
    vec![
        ip([
            "rule",
            "del",
            "iif",
            &tunnel.veth_host,
            "lookup",
            &table,
            "priority",
            &priority,
        ]),
        ip(["route", "flush", "table", &table]),
        ip(["link", "del", &tunnel.tun]),
        ip(["link", "del", &tunnel.veth_host]),
        ip(["netns", "del", &tunnel.netns]),
    ]
}

fn ip<const N: usize>(args: [&str; N]) -> Cmd {
    Cmd {
        program: Program::Ip,
        args: args.into_iter().map(str::to_owned).collect(),
        stdin: None,
    }
}
