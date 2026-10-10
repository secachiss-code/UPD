//! Сеть приложения: свой netns, veth и таблица, в которой единственный путь — TUN worker-а.

pub mod commands;
pub mod dns;
pub mod exec;
pub mod firewall;
pub mod observe;
pub mod plan;
pub mod state;

pub use commands::{Cmd, Program, create_commands, destroy_commands};
pub use exec::{NetExec, RecordingExec, SystemExec, create, destroy};
pub use firewall::{render_table, replace_commands};
pub use observe::{
    Drift, Observed, ObservedRoute, ObservedRule, audit, parse_links, parse_routes, parse_rules,
    repair_commands,
};
pub use plan::{MAX_TUNNELS, NetError, TunnelNet, tunnel_net};
pub use state::{TunnelEvent, TunnelState, net_axis, next, passes_traffic};
