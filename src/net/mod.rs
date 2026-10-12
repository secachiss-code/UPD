//! Сеть приложения: свой netns, veth и таблица, в которой единственный путь — TUN worker-а.

pub mod cli;
pub mod commands;
pub mod dns;
pub mod exec;
pub mod failure;
pub mod firewall;
pub mod host;
pub mod host_audit;
pub mod host_commands;
pub mod host_firewall;
pub mod host_plan;
pub mod observe;
pub mod plan;
pub mod policy;
pub mod query;
pub mod state;
pub mod uplink;

pub use commands::{Cmd, Program, create_commands, destroy_commands};
pub use exec::{NetExec, RecordingExec, SystemExec, create, destroy};
pub use firewall::{render_table, replace_commands};
pub use observe::{
    Drift, Observed, ObservedRoute, ObservedRule, audit, parse_links, parse_routes, parse_rules,
    repair_commands,
};
pub use plan::{MAX_TUNNELS, NetError, TunnelNet, tunnel_net};
pub use state::{TunnelEvent, TunnelState, net_axis, next, passes_traffic};
