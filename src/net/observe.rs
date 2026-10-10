//! Сверка желаемой сети с выводом `ip -j`. Нераспознанный JSON — ошибка, не пустой аудит.

use std::collections::BTreeMap;

use serde_json::Value;

use super::commands::{Cmd, Program};
use super::plan::{NetError, TunnelNet};

pub struct Observed {
    pub rules: Vec<ObservedRule>,
    pub routes: BTreeMap<u32, Vec<ObservedRoute>>,
    pub links: Vec<String>,
    pub netns: Vec<String>,
}

pub struct ObservedRule {
    pub priority: u32,
    pub iif: Option<String>,
    pub table: String,
}

pub struct ObservedRoute {
    pub dst: String,
    pub dev: Option<String>,
    pub kind: Option<String>,
    pub metric: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drift {
    MissingBlackhole(u8),
    MissingRule(u8),
    MissingTunRoute(u8),
    MissingVeth(u8),
    MissingTun(u8),
    MissingNetns(u8),
    OrphanVeth(String),
    OrphanRule(u32),
}

pub fn parse_rules(json: &str) -> Result<Vec<ObservedRule>, NetError> {
    let value: Value = serde_json::from_str(json).map_err(|_| NetError::Drift)?;
    let items = value.as_array().ok_or(NetError::Drift)?;
    items.iter().map(one_rule).collect()
}

pub fn parse_routes(json: &str) -> Result<Vec<ObservedRoute>, NetError> {
    let value: Value = serde_json::from_str(json).map_err(|_| NetError::Drift)?;
    let items = value.as_array().ok_or(NetError::Drift)?;
    items.iter().map(one_route).collect()
}

pub fn parse_links(json: &str) -> Result<Vec<String>, NetError> {
    let value: Value = serde_json::from_str(json).map_err(|_| NetError::Drift)?;
    let items = value.as_array().ok_or(NetError::Drift)?;
    items
        .iter()
        .map(|item| {
            item.get("ifname")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or(NetError::Drift)
        })
        .collect()
}

pub fn audit(desired: &[TunnelNet], observed: &Observed) -> Vec<Drift> {
    let mut ordered: Vec<&TunnelNet> = desired.iter().collect();
    ordered.sort_by_key(|tunnel| tunnel.index);
    let mut drift = Vec::new();
    for tunnel in &ordered {
        let routes = observed
            .routes
            .get(&tunnel.table)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if !routes
            .iter()
            .any(|route| route.dst == "default" && route.kind.as_deref() == Some("blackhole"))
        {
            drift.push(Drift::MissingBlackhole(tunnel.index));
        }
        let rule_ok = observed.rules.iter().any(|rule| {
            rule.priority == tunnel.rule_priority
                && rule.iif.as_deref() == Some(tunnel.veth_host.as_str())
                && rule.table == tunnel.table.to_string()
        });
        if !rule_ok {
            drift.push(Drift::MissingRule(tunnel.index));
        }
        let tun_present = observed.links.iter().any(|name| name == &tunnel.tun);
        let route_ok = routes.iter().any(|route| {
            route.dst == "default" && route.dev.as_deref() == Some(tunnel.tun.as_str())
        });
        if !route_ok || !tun_present {
            drift.push(Drift::MissingTunRoute(tunnel.index));
        }
        if !observed.links.iter().any(|name| name == &tunnel.veth_host) {
            drift.push(Drift::MissingVeth(tunnel.index));
        }
        if !tun_present {
            drift.push(Drift::MissingTun(tunnel.index));
        }
        if !observed.netns.iter().any(|name| name == &tunnel.netns) {
            drift.push(Drift::MissingNetns(tunnel.index));
        }
    }
    let mut orphans_veth: Vec<String> = observed
        .links
        .iter()
        .filter(|name| {
            veth_index(name)
                .is_some_and(|index| !ordered.iter().any(|tunnel| tunnel.index == index))
        })
        .cloned()
        .collect();
    orphans_veth.sort();
    for name in orphans_veth {
        drift.push(Drift::OrphanVeth(name));
    }
    let mut orphans_rule: Vec<u32> = observed
        .rules
        .iter()
        .filter(|rule| (1000..1064).contains(&rule.priority))
        // Чужое правило с тем же приоритетом не наше: сирота — только правило с нашим veth.
        .filter(|rule| {
            rule.iif.as_deref().and_then(veth_index).map(u32::from) == Some(rule.priority - 1000)
        })
        .filter(|rule| {
            !ordered
                .iter()
                .any(|tunnel| tunnel.rule_priority == rule.priority)
        })
        .map(|rule| rule.priority)
        .collect();
    orphans_rule.sort_unstable();
    orphans_rule.dedup();
    for priority in orphans_rule {
        drift.push(Drift::OrphanRule(priority));
    }
    drift
}

pub fn repair_commands(drift: &Drift, desired: &[TunnelNet]) -> Vec<Cmd> {
    match drift {
        Drift::MissingBlackhole(index) => {
            let Some(tunnel) = tunnel(*index, desired) else {
                return Vec::new();
            };
            vec![ip_owned([
                "route",
                "add",
                "blackhole",
                "default",
                "metric",
                "200",
                "table",
                &tunnel.table.to_string(),
            ])]
        }
        Drift::MissingRule(index) => {
            let Some(tunnel) = tunnel(*index, desired) else {
                return Vec::new();
            };
            let table = tunnel.table.to_string();
            let priority = tunnel.rule_priority.to_string();
            vec![ip_owned([
                "rule",
                "add",
                "iif",
                &tunnel.veth_host,
                "lookup",
                &table,
                "priority",
                &priority,
            ])]
        }
        Drift::OrphanVeth(name) => vec![ip_owned(["link", "del", name])],
        Drift::OrphanRule(priority) => {
            let veth = format!("cmv{}h", priority.saturating_sub(1000));
            vec![ip_owned([
                "rule",
                "del",
                "iif",
                &veth,
                "priority",
                &priority.to_string(),
            ])]
        }
        Drift::MissingTunRoute(_)
        | Drift::MissingVeth(_)
        | Drift::MissingTun(_)
        | Drift::MissingNetns(_) => Vec::new(),
    }
}

fn tunnel(index: u8, desired: &[TunnelNet]) -> Option<&TunnelNet> {
    desired.iter().find(|tunnel| tunnel.index == index)
}

fn veth_index(name: &str) -> Option<u8> {
    let rest = name.strip_prefix("cmv")?.strip_suffix('h')?;
    let index: u8 = rest.parse().ok()?;
    (index < 64).then_some(index)
}

fn one_rule(value: &Value) -> Result<ObservedRule, NetError> {
    let object = value.as_object().ok_or(NetError::Drift)?;
    let priority = object
        .get("priority")
        .and_then(Value::as_u64)
        .ok_or(NetError::Drift)?;
    let priority = u32::try_from(priority).map_err(|_| NetError::Drift)?;
    let iif = object.get("iif").and_then(Value::as_str).map(str::to_owned);
    let table = match object.get("table") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        _ => return Err(NetError::Drift),
    };
    Ok(ObservedRule {
        priority,
        iif,
        table,
    })
}

fn one_route(value: &Value) -> Result<ObservedRoute, NetError> {
    let object = value.as_object().ok_or(NetError::Drift)?;
    let dst = object
        .get("dst")
        .and_then(Value::as_str)
        .unwrap_or("default")
        .to_owned();
    let dev = object.get("dev").and_then(Value::as_str).map(str::to_owned);
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let metric = object
        .get("metric")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    Ok(ObservedRoute {
        dst,
        dev,
        kind,
        metric,
    })
}

fn ip_owned<const N: usize>(args: [&str; N]) -> Cmd {
    Cmd {
        program: Program::Ip,
        args: args.into_iter().map(str::to_owned).collect(),
        stdin: None,
    }
}
