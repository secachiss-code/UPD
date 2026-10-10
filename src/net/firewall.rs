//! Таблица nftables: чужой forwarding не трогаем, интерфейсы CM ходят только в свой TUN.

use super::commands::{Cmd, Program};
use super::plan::TunnelNet;

pub fn render_table(tunnels: &[TunnelNet]) -> String {
    let mut ordered: Vec<&TunnelNet> = tunnels.iter().collect();
    ordered.sort_by_key(|tunnel| tunnel.index);
    let mut text = String::from(
        "table inet cm {\n  chain forward {\n    type filter hook forward priority filter; policy accept;\n",
    );
    for tunnel in ordered {
        text.push_str(&format!(
            "    iifname \"{}\" oifname \"{}\" counter accept\n    iifname \"{}\" oifname \"{}\" counter accept\n",
            tunnel.veth_host, tunnel.tun, tunnel.tun, tunnel.veth_host
        ));
    }
    text.push_str("    iifname \"cmv*\" counter drop\n    oifname \"cmv*\" counter drop\n  }\n}\n");
    text
}

pub fn replace_commands(tunnels: &[TunnelNet]) -> Vec<Cmd> {
    vec![Cmd {
        program: Program::Nft,
        args: vec!["-f".to_owned(), "-".to_owned()],
        stdin: Some(format!(
            "table inet cm\ndelete table inet cm\n{}",
            render_table(tunnels)
        )),
    }]
}
