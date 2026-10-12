//! Внешние интерфейсы по выводу `ip -j`. Свои интерфейсы CM в список не входят.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::host_plan::{MAX_UPLINK_ADDRS, MAX_UPLINKS, Uplink};
use super::plan::NetError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultRoute {
    pub dev: String,
    pub metric: u32,
}

/// Вывод `ip -j route show default table main` и того же с `-6`.
pub fn parse_default_routes(json: &str) -> Result<Vec<DefaultRoute>, NetError> {
    let value: Value = serde_json::from_str(json).map_err(|_| NetError::CommandFailed)?;
    let items = value.as_array().ok_or(NetError::CommandFailed)?;
    let mut routes = Vec::new();
    for item in items {
        let Some(dev) = item.get("dev").and_then(Value::as_str) else {
            continue;
        };
        let metric = item
            .get("metric")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(u64::from(u32::MAX)) as u32;
        routes.push(DefaultRoute {
            dev: dev.to_owned(),
            metric,
        });
    }
    Ok(routes)
}

/// Вывод `ip -j -4 addr show`: имя интерфейса и глобальные `addr_info[].local`.
pub fn parse_addrs(json: &str) -> Result<BTreeMap<String, Vec<String>>, NetError> {
    let value: Value = serde_json::from_str(json).map_err(|_| NetError::CommandFailed)?;
    let items = value.as_array().ok_or(NetError::CommandFailed)?;
    let mut addrs = BTreeMap::new();
    for item in items {
        let Some(name) = item.get("ifname").and_then(Value::as_str) else {
            continue;
        };
        let Some(info) = item.get("addr_info").and_then(Value::as_array) else {
            continue;
        };
        let entry = addrs.entry(name.to_owned()).or_insert_with(Vec::new);
        for address in info {
            if address.get("scope").and_then(Value::as_str) != Some("global") {
                continue;
            }
            if let Some(local) = address.get("local").and_then(Value::as_str) {
                entry.push(local.to_owned());
            }
        }
    }
    Ok(addrs)
}

pub fn uplinks(
    v4: &[DefaultRoute],
    v6: &[DefaultRoute],
    addrs: &BTreeMap<String, Vec<String>>,
) -> Vec<Uplink> {
    let v4_names = ordered_devs(v4);
    let mut seen = BTreeSet::new();
    let mut devs = Vec::new();
    for dev in &v4_names {
        if seen.insert(dev.clone()) {
            devs.push(dev.clone());
        }
    }
    for dev in ordered_devs(v6) {
        if seen.insert(dev.clone()) {
            devs.push(dev);
        }
    }
    devs.retain(|dev| dev != "lo" && !dev.starts_with("cm"));
    devs.truncate(MAX_UPLINKS);
    let v4_set: BTreeSet<&str> = v4_names.iter().map(String::as_str).collect();
    let mut total = 0usize;
    let mut result = Vec::with_capacity(devs.len());
    for dev in devs {
        let mut list = Vec::new();
        if v4_set.contains(dev.as_str())
            && let Some(found) = addrs.get(&dev)
        {
            list = found.clone();
            list.sort();
            let room = MAX_UPLINK_ADDRS.saturating_sub(total);
            list.truncate(room);
            total += list.len();
        }
        result.push(Uplink { dev, addrs: list });
    }
    result
}

fn ordered_devs(routes: &[DefaultRoute]) -> Vec<String> {
    let mut ordered: Vec<&DefaultRoute> = routes.iter().collect();
    ordered.sort_by(|left, right| {
        left.metric
            .cmp(&right.metric)
            .then_with(|| left.dev.cmp(&right.dev))
    });
    let mut seen = BTreeSet::new();
    let mut devs = Vec::new();
    for route in ordered {
        if seen.insert(route.dev.as_str()) {
            devs.push(route.dev.clone());
        }
    }
    devs
}
