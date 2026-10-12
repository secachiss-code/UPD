//! Переходы режима хоста и проверки, без которых туннель не включается.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;

use crate::profiles::HostMode;

use super::observe::{self, ObservedRule};
use super::query::NetQuery;
use super::uplink;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostStep {
    StopWorker,
    RevertNet,
    ApplyNet(HostMode),
    StartWorker,
}

pub fn transition(from: HostMode, to: HostMode) -> Vec<HostStep> {
    use HostMode::{Off, Proxy, Tunnel};
    use HostStep::{ApplyNet, RevertNet, StartWorker, StopWorker};
    match (from, to) {
        (Off, Off) | (Proxy, Proxy) | (Tunnel, Tunnel) => Vec::new(),
        (Off, Proxy | Tunnel) => vec![ApplyNet(to), StartWorker],
        (Proxy | Tunnel, Off) => vec![StopWorker, RevertNet],
        (Proxy, Tunnel) | (Tunnel, Proxy) => {
            vec![StopWorker, RevertNet, ApplyNet(to), StartWorker]
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostBlock {
    LegacyCm,
    ForeignAutoRoute,
    Resolved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Integration {
    pub network_manager: bool,
    pub firewalld: bool,
    pub resolved: bool,
}

pub fn detect(run_root: &Path) -> Integration {
    Integration {
        network_manager: run_root.join("NetworkManager").exists(),
        firewalld: run_root.join("firewalld").exists(),
        resolved: run_root.join("systemd/resolve/stub-resolv.conf").exists(),
    }
}

pub fn preflight(
    mode: HostMode,
    links: &[String],
    rules: &[ObservedRule],
    integration: &Integration,
) -> Result<(), HostBlock> {
    if mode != HostMode::Tunnel {
        return Ok(());
    }
    if links.iter().any(|link| link == "cm-vpn") {
        return Err(HostBlock::LegacyCm);
    }
    if rules.iter().any(|rule| rule.table == "2022") {
        return Err(HostBlock::ForeignAutoRoute);
    }
    if integration.resolved {
        return Err(HostBlock::Resolved);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Ok,
    Warn,
    Block,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckLine {
    pub name: &'static str,
    pub verdict: Verdict,
    pub token: Option<&'static str>,
}

pub struct HostFacts {
    pub nft: bool,
    pub links: Vec<String>,
    pub rules: Vec<ObservedRule>,
    pub integration: Integration,
    pub uplinks: usize,
    pub legacy_unit_active: bool,
    pub controller_socket: bool,
    pub host_mode: String,
    pub rp_filter_all: Option<u8>,
    pub rp_filter_default: Option<u8>,
}

/// Десять строк в фиксированном порядке. Строка без сработавшего условия — `Ok` без токена.
pub fn check_lines(facts: &HostFacts) -> Vec<CheckLine> {
    vec![
        line("nft", !facts.nft, Verdict::Block, "nft_missing"),
        line(
            "legacy_vpn",
            facts.legacy_unit_active || facts.links.iter().any(|link| link == "cm-vpn"),
            Verdict::Block,
            "legacy_vpn_active",
        ),
        line(
            "foreign_autoroute",
            facts.rules.iter().any(|rule| rule.table == "2022"),
            Verdict::Block,
            "foreign_autoroute",
        ),
        line(
            "resolved",
            facts.integration.resolved,
            Verdict::Block,
            "resolved_unsupported",
        ),
        line(
            "firewalld",
            facts.integration.firewalld,
            Verdict::Warn,
            "firewalld_unverified",
        ),
        line(
            "network_manager",
            facts.integration.network_manager,
            Verdict::Warn,
            "network_manager_unverified",
        ),
        uplink_line(facts.uplinks),
        line(
            "controller",
            !facts.controller_socket,
            Verdict::Block,
            "controller_unavailable",
        ),
        line(
            "stale_net",
            facts.host_mode == "off"
                && facts
                    .links
                    .iter()
                    .any(|link| link == "cmtunh" || link == "cmsink"),
            Verdict::Block,
            "stale_host_net",
        ),
        CheckLine {
            name: "rp_filter",
            verdict: Verdict::Ok,
            token: None,
        },
    ]
}

pub fn status_code(lines: &[CheckLine]) -> i32 {
    if lines.iter().any(|line| line.verdict == Verdict::Block) {
        1
    } else {
        0
    }
}

pub fn render_line(line: &CheckLine, facts: &HostFacts) -> String {
    let mut text = format!("check={} result={}", line.name, verdict_name(line.verdict));
    if let Some(token) = line.token {
        text.push(' ');
        text.push('[');
        text.push_str(token);
        text.push(']');
    }
    if line.name == "rp_filter" {
        text.push_str(" all=");
        text.push_str(&num_or_dash(facts.rp_filter_all));
        text.push_str(" default=");
        text.push_str(&num_or_dash(facts.rp_filter_default));
    }
    text
}

pub fn render_text(lines: &[CheckLine], facts: &HostFacts) -> String {
    let mut text = String::new();
    for line in lines {
        text.push_str(&render_line(line, facts));
        text.push('\n');
    }
    text
}

pub fn render_json(lines: &[CheckLine], facts: &HostFacts) -> String {
    let items: Vec<serde_json::Value> = lines
        .iter()
        .map(|line| {
            let mut item = serde_json::Map::new();
            item.insert("check".to_owned(), serde_json::json!(line.name));
            item.insert(
                "result".to_owned(),
                serde_json::json!(verdict_name(line.verdict)),
            );
            if let Some(token) = line.token {
                item.insert("token".to_owned(), serde_json::json!(token));
            }
            if line.name == "rp_filter" {
                item.insert("all".to_owned(), json_u8(facts.rp_filter_all));
                item.insert("default".to_owned(), json_u8(facts.rp_filter_default));
            }
            serde_json::Value::Object(item)
        })
        .collect();
    serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_owned())
}

fn line(name: &'static str, hit: bool, verdict: Verdict, token: &'static str) -> CheckLine {
    if hit {
        CheckLine {
            name,
            verdict,
            token: Some(token),
        }
    } else {
        CheckLine {
            name,
            verdict: Verdict::Ok,
            token: None,
        }
    }
}

fn uplink_line(count: usize) -> CheckLine {
    if count == 0 {
        line("uplinks", true, Verdict::Block, "no_uplink")
    } else if count > super::host_plan::MAX_UPLINKS {
        line("uplinks", true, Verdict::Block, "too_many_uplinks")
    } else {
        line("uplinks", false, Verdict::Block, "no_uplink")
    }
}

fn verdict_name(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Ok => "ok",
        Verdict::Warn => "warn",
        Verdict::Block => "block",
    }
}

fn num_or_dash(value: Option<u8>) -> String {
    match value {
        Some(number) => number.to_string(),
        None => "-".to_owned(),
    }
}

fn json_u8(value: Option<u8>) -> serde_json::Value {
    match value {
        Some(number) => serde_json::json!(number),
        None => serde_json::Value::Null,
    }
}

/// Собрать факты только чтением. Сокет контроллера не открывается, команд изменения сети нет.
pub fn collect_facts(
    query: &mut dyn NetQuery,
    run_root: &Path,
    proc_root: &Path,
    socket: &Path,
    state_file: &Path,
    nft: bool,
    legacy_unit_active: bool,
) -> HostFacts {
    let links = read_links(query);
    let rules = read_rules(query);
    HostFacts {
        nft,
        links,
        rules,
        integration: detect(run_root),
        uplinks: uplink_count(query),
        legacy_unit_active,
        controller_socket: socket_present(socket),
        host_mode: read_host_mode(state_file),
        rp_filter_all: read_u8(&proc_root.join("sys/net/ipv4/conf/all/rp_filter")),
        rp_filter_default: read_u8(&proc_root.join("sys/net/ipv4/conf/default/rp_filter")),
    }
}

fn read_links(query: &mut dyn NetQuery) -> Vec<String> {
    query
        .query(super::commands::Program::Ip, &["-j", "link", "show"])
        .ok()
        .and_then(|text| observe::parse_links(&text).ok())
        .unwrap_or_default()
}

fn read_rules(query: &mut dyn NetQuery) -> Vec<ObservedRule> {
    query
        .query(super::commands::Program::Ip, &["-j", "rule", "show"])
        .ok()
        .and_then(|text| observe::parse_rules(&text).ok())
        .unwrap_or_default()
}

fn uplink_count(query: &mut dyn NetQuery) -> usize {
    let Ok(text) = query.query(
        super::commands::Program::Ip,
        &["-j", "route", "show", "default", "table", "main"],
    ) else {
        return 0;
    };
    let Ok(v4) = uplink::parse_default_routes(&text) else {
        return 0;
    };
    let v6 = query
        .query(
            super::commands::Program::Ip,
            &["-j", "-6", "route", "show", "default", "table", "main"],
        )
        .ok()
        .and_then(|text| uplink::parse_default_routes(&text).ok())
        .unwrap_or_default();
    let mut seen = BTreeSet::new();
    for route in v4.iter().chain(&v6) {
        if route.dev == "lo" || route.dev.starts_with("cm") {
            continue;
        }
        seen.insert(route.dev.clone());
    }
    seen.len()
}

fn socket_present(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_socket())
}

fn read_host_mode(path: &Path) -> String {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return "off".to_owned();
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return "off".to_owned();
    }
    let Ok(bytes) = fs::read(path) else {
        return "off".to_owned();
    };
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|value| {
            value
                .get("mode")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .filter(|mode| mode == "off" || mode == "proxy" || mode == "tunnel")
        .unwrap_or_else(|| "off".to_owned())
}

fn read_u8(path: &Path) -> Option<u8> {
    let text = fs::read_to_string(path).ok()?;
    text.split_whitespace().next()?.parse().ok()
}
