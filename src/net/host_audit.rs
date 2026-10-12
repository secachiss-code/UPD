//! Сверка туннеля хоста с тем, что ставит `host_create_commands`.

use super::commands::Cmd;
use super::host_commands::host_create_commands;
use super::host_firewall::host_firewall_commands;
use super::host_plan::{HostNet, Uplink};
use super::observe::{Observed, ObservedRoute, ObservedRule};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostDrift {
    MissingSink,
    MissingSinkRoute { v6: bool },
    MissingTun,
    MissingTunRoute,
    MissingRule { v6: bool, priority: u32 },
    MissingFirewall,
}

struct ExpectedRule {
    v6: bool,
    priority: u32,
    table: String,
}

pub fn host_audit(
    host: &HostNet,
    uplinks: &[Uplink],
    observed: &Observed,
    rules6: &[ObservedRule],
    routes6_sink: &[ObservedRoute],
    firewall_present: bool,
) -> Vec<HostDrift> {
    let mut drift = Vec::new();
    if !has_link(&observed.links, &host.sink) {
        drift.push(HostDrift::MissingSink);
    }
    let sink_routes = observed
        .routes
        .get(&host.sink_table)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if !has_default(sink_routes, &host.sink) {
        drift.push(HostDrift::MissingSinkRoute { v6: false });
    }
    if !has_default(routes6_sink, &host.sink) {
        drift.push(HostDrift::MissingSinkRoute { v6: true });
    }
    if !has_link(&observed.links, &host.tun) {
        drift.push(HostDrift::MissingTun);
    }
    let tun_routes = observed
        .routes
        .get(&host.table)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if !has_default(tun_routes, &host.tun) {
        drift.push(HostDrift::MissingTunRoute);
    }
    for rule in expected_rules(host, uplinks) {
        let rules = if rule.v6 {
            rules6
        } else {
            observed.rules.as_slice()
        };
        let found = rules
            .iter()
            .any(|item| item.priority == rule.priority && item.table == rule.table);
        if !found {
            drift.push(HostDrift::MissingRule {
                v6: rule.v6,
                priority: rule.priority,
            });
        }
    }
    if !firewall_present {
        drift.push(HostDrift::MissingFirewall);
    }
    drift
}

pub fn host_repair_commands(host: &HostNet, uplinks: &[Uplink], drift: &[HostDrift]) -> Vec<Cmd> {
    let mut commands = Vec::new();
    for command in host_create_commands(host, uplinks) {
        if command_needed(&command, host, drift) {
            commands.push(route_replace(command));
        }
    }
    if drift
        .iter()
        .any(|item| matches!(item, HostDrift::MissingFirewall))
    {
        commands.extend(host_firewall_commands(Some(host)));
    }
    commands
}

fn expected_rules(host: &HostNet, uplinks: &[Uplink]) -> Vec<ExpectedRule> {
    let mut rules = Vec::new();
    let mut address = 0u32;
    for (index, _) in uplinks.iter().enumerate() {
        rules.push(ExpectedRule {
            v6: false,
            priority: host.rule_uplink + index as u32,
            table: "main".to_owned(),
        });
    }
    for uplink in uplinks {
        for _addr in &uplink.addrs {
            rules.push(ExpectedRule {
                v6: false,
                priority: host.rule_uplink_addr + address,
                table: "main".to_owned(),
            });
            address += 1;
        }
    }
    rules.push(ExpectedRule {
        v6: false,
        priority: host.rule_main,
        table: "main".to_owned(),
    });
    rules.push(ExpectedRule {
        v6: false,
        priority: host.rule_tun,
        table: host.table.to_string(),
    });
    rules.push(ExpectedRule {
        v6: false,
        priority: host.rule_sink,
        table: host.sink_table.to_string(),
    });
    for (index, _) in uplinks.iter().enumerate() {
        rules.push(ExpectedRule {
            v6: true,
            priority: host.rule_uplink + index as u32,
            table: "main".to_owned(),
        });
    }
    rules.push(ExpectedRule {
        v6: true,
        priority: host.rule_main,
        table: "main".to_owned(),
    });
    rules.push(ExpectedRule {
        v6: true,
        priority: host.rule_sink,
        table: host.sink_table.to_string(),
    });
    rules
}

fn command_needed(command: &Cmd, host: &HostNet, drift: &[HostDrift]) -> bool {
    let args = &command.args;
    let v6 = args.first().map(String::as_str) == Some("-6");
    match verb(args) {
        Some("link") if names_dev(args, &host.sink) => {
            has(drift, |item| matches!(item, HostDrift::MissingSink))
        }
        Some("route") if names_dev(args, &host.sink) => has(
            drift,
            |item| matches!(item, HostDrift::MissingSinkRoute { v6: flag } if *flag == v6),
        ),
        Some("tuntap") | Some("addr") if names_dev(args, &host.tun) => {
            has(drift, |item| matches!(item, HostDrift::MissingTun))
        }
        Some("link") if names_dev(args, &host.tun) => {
            has(drift, |item| matches!(item, HostDrift::MissingTun))
        }
        Some("route") if names_dev(args, &host.tun) => {
            has(drift, |item| matches!(item, HostDrift::MissingTunRoute))
        }
        Some("rule") => {
            let Some(priority) =
                arg_after(args, "priority").and_then(|text| text.parse::<u32>().ok())
            else {
                return false;
            };
            has(drift, |item| {
                matches!(
                    item,
                    HostDrift::MissingRule { v6: flag, priority: found }
                        if *flag == v6 && *found == priority
                )
            })
        }
        _ => false,
    }
}

fn route_replace(mut command: Cmd) -> Cmd {
    if let Some(index) = command.args.iter().position(|arg| arg == "route")
        && command.args.get(index + 1).map(String::as_str) == Some("add")
    {
        command.args[index + 1] = "replace".to_owned();
    }
    command
}

fn has(drift: &[HostDrift], pred: impl Fn(&HostDrift) -> bool) -> bool {
    drift.iter().any(pred)
}

fn has_link(links: &[String], name: &str) -> bool {
    links.iter().any(|link| link == name)
}

fn has_default(routes: &[ObservedRoute], dev: &str) -> bool {
    routes
        .iter()
        .any(|route| route.dst == "default" && route.dev.as_deref() == Some(dev))
}

fn verb(args: &[String]) -> Option<&str> {
    args.iter()
        .find(|arg| arg.as_str() != "-6" && arg.as_str() != "-4")
        .map(String::as_str)
}

fn names_dev(args: &[String], dev: &str) -> bool {
    args.windows(2)
        .any(|pair| pair[0] == "dev" && pair[1] == dev)
        || args.iter().any(|arg| arg == dev)
}

fn arg_after<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let index = args.iter().position(|arg| arg == name)?;
    args.get(index + 1).map(String::as_str)
}
