//! Таблица `ip cm_host` направляет DNS хоста в туннель. Таблицу `inet cm` она не трогает.

use super::commands::{Cmd, Program};
use super::host_plan::{BOOTSTRAP_DNS, HostNet};

pub fn render_host_table(host: &HostNet) -> String {
    let dns = BOOTSTRAP_DNS.join(", ");
    format!(
        "table ip cm_host {{\n  chain output {{\n    type nat hook output priority -100; policy accept;\n    oifname != \"{tun}\" oifname != \"lo\" ip daddr != {{ {dns} }} meta l4proto {{ tcp, udp }} th dport 53 counter dnat to {dns_addr}\n  }}\n}}\n",
        tun = host.tun,
        dns_addr = host.dns_addr,
    )
}

pub fn host_firewall_commands(host: Option<&HostNet>) -> Vec<Cmd> {
    let prefix = "table ip cm_host\ndelete table ip cm_host\n";
    let stdin = match host {
        Some(host) => format!("{prefix}{}", render_host_table(host)),
        None => prefix.to_owned(),
    };
    vec![Cmd {
        program: Program::Nft,
        args: vec!["-f".to_owned(), "-".to_owned()],
        stdin: Some(stdin),
    }]
}
