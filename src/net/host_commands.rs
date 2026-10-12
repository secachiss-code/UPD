//! Команды туннеля хоста. Приёмник — dummy: blackhole в этой таблице ломает ответы ядрам приложений.

use super::commands::{Cmd, Program};
use super::host_plan::{HostNet, Uplink};

pub fn host_create_commands(host: &HostNet, uplinks: &[Uplink]) -> Vec<Cmd> {
    let mut commands = setup(host);
    commands.extend(uplink_rule_commands(host, uplinks, "add"));
    commands.extend(fixed_rule_commands(host, "add"));
    commands
}

pub fn host_destroy_commands(host: &HostNet, uplinks: &[Uplink]) -> Vec<Cmd> {
    let mut rules = uplink_rule_commands(host, uplinks, "add");
    rules.extend(fixed_rule_commands(host, "add"));
    rules.reverse();
    for command in &mut rules {
        if let Some(verb) = command.args.iter_mut().find(|arg| *arg == "add") {
            *verb = "del".to_owned();
        }
    }
    let table = host.table.to_string();
    let sink = host.sink_table.to_string();
    rules.push(ip(&["route", "flush", "table", &table]));
    rules.push(ip(&["route", "flush", "table", &sink]));
    rules.push(ip(&["-6", "route", "flush", "table", &sink]));
    rules.push(ip(&["link", "del", &host.tun]));
    rules.push(ip(&["link", "del", &host.sink]));
    rules
}

pub fn uplink_commands(host: &HostNet, old: &[Uplink], new: &[Uplink]) -> Vec<Cmd> {
    if old == new {
        return Vec::new();
    }
    let mut commands = uplink_rule_commands(host, old, "add");
    commands.reverse();
    for command in &mut commands {
        if let Some(verb) = command.args.iter_mut().find(|arg| *arg == "add") {
            *verb = "del".to_owned();
        }
    }
    commands.extend(uplink_rule_commands(host, new, "add"));
    commands
}

fn setup(host: &HostNet) -> Vec<Cmd> {
    let uid = host.owner_uid.to_string();
    let mtu = host.mtu.to_string();
    let table = host.table.to_string();
    let sink = host.sink_table.to_string();
    vec![
        ip(&["link", "add", &host.sink, "type", "dummy"]),
        ip(&["link", "set", &host.sink, "up"]),
        ip(&["route", "add", "default", "dev", &host.sink, "table", &sink]),
        ip(&[
            "-6", "route", "add", "default", "dev", &host.sink, "table", &sink,
        ]),
        ip(&[
            "tuntap", "add", "dev", &host.tun, "mode", "tun", "user", &uid,
        ]),
        ip(&["addr", "add", &host.tun_addr, "dev", &host.tun]),
        ip(&["link", "set", &host.tun, "mtu", &mtu, "up"]),
        ip(&["route", "add", "default", "dev", &host.tun, "table", &table]),
    ]
}

fn uplink_rule_commands(host: &HostNet, uplinks: &[Uplink], verb: &str) -> Vec<Cmd> {
    let mut commands = Vec::new();
    let mut address = 0u32;
    for (index, uplink) in uplinks.iter().enumerate() {
        let priority = (host.rule_uplink + index as u32).to_string();
        commands.push(rule(
            false,
            verb,
            &["priority", &priority, "oif", &uplink.dev, "lookup", "main"],
        ));
        commands.push(rule(
            true,
            verb,
            &["priority", &priority, "oif", &uplink.dev, "lookup", "main"],
        ));
        for addr in &uplink.addrs {
            let from = (host.rule_uplink_addr + address).to_string();
            address += 1;
            commands.push(rule(
                false,
                verb,
                &["priority", &from, "from", addr, "lookup", "main"],
            ));
        }
    }
    commands
}

fn fixed_rule_commands(host: &HostNet, verb: &str) -> Vec<Cmd> {
    let main = host.rule_main.to_string();
    let sink_rule = host.rule_sink.to_string();
    let tun = host.rule_tun.to_string();
    let sink = host.sink_table.to_string();
    let table = host.table.to_string();
    vec![
        rule(
            true,
            verb,
            &[
                "priority",
                &main,
                "lookup",
                "main",
                "suppress_prefixlength",
                "0",
            ],
        ),
        rule(true, verb, &["priority", &sink_rule, "lookup", &sink]),
        rule(
            false,
            verb,
            &[
                "priority",
                &main,
                "lookup",
                "main",
                "suppress_prefixlength",
                "0",
            ],
        ),
        rule(false, verb, &["priority", &sink_rule, "lookup", &sink]),
        rule(false, verb, &["priority", &tun, "lookup", &table]),
    ]
}

fn rule(v6: bool, verb: &str, tail: &[&str]) -> Cmd {
    let mut args = Vec::new();
    if v6 {
        args.push("-6".to_owned());
    }
    args.push("rule".to_owned());
    args.push(verb.to_owned());
    args.extend(tail.iter().map(|part| (*part).to_owned()));
    Cmd {
        program: Program::Ip,
        args,
        stdin: None,
    }
}

fn ip(args: &[&str]) -> Cmd {
    Cmd {
        program: Program::Ip,
        args: args.iter().map(|part| (*part).to_owned()).collect(),
        stdin: None,
    }
}
