//! Фактическое состояние сети командами `ip -j`. Вывод длиннее предела — ошибка.

use std::collections::BTreeMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::commands::Program;
use super::host_plan::Uplink;
use super::observe::{Observed, parse_links, parse_routes, parse_rules};
use super::plan::NetError;
use super::uplink;

pub const MAX_QUERY_BYTES: usize = 1 << 20;

pub trait NetQuery {
    fn query(&mut self, program: Program, args: &[&str]) -> Result<String, NetError>;
}

pub struct SystemQuery;

impl NetQuery for SystemQuery {
    fn query(&mut self, program: Program, args: &[&str]) -> Result<String, NetError> {
        let path = match program {
            Program::Ip => "/usr/bin/ip",
            Program::Nft => "/usr/bin/nft",
            Program::Sysctl => "/usr/bin/sysctl",
        };
        let mut child = Command::new(path)
            .args(args)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| NetError::CommandFailed)?;
        let mut stdout = child.stdout.take().ok_or(NetError::CommandFailed)?;
        let reader = std::thread::spawn(move || {
            let mut limited = Read::take(&mut stdout, (MAX_QUERY_BYTES + 1) as u64);
            let mut buf = Vec::new();
            let _ = limited.read_to_end(&mut buf);
            buf
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return Err(NetError::Timeout);
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => {
                    let _ = reader.join();
                    return Err(NetError::CommandFailed);
                }
            }
        };
        let buf = reader.join().map_err(|_| NetError::CommandFailed)?;
        if !status.success() || buf.len() > MAX_QUERY_BYTES {
            return Err(NetError::CommandFailed);
        }
        String::from_utf8(buf).map_err(|_| NetError::CommandFailed)
    }
}

#[derive(Default)]
pub struct FixedQuery {
    pub answers: BTreeMap<String, String>,
    pub asked: Vec<String>,
}

impl NetQuery for FixedQuery {
    fn query(&mut self, _program: Program, args: &[&str]) -> Result<String, NetError> {
        let key = args.join(" ");
        self.asked.push(key.clone());
        self.answers
            .get(&key)
            .cloned()
            .ok_or(NetError::CommandFailed)
    }
}

pub fn observe(query: &mut dyn NetQuery, tables: &[u32]) -> Result<Observed, NetError> {
    let rules = parse_rules(&query.query(Program::Ip, &["-j", "rule", "show"])?)?;
    let mut routes = BTreeMap::new();
    for table in tables {
        let text = table.to_string();
        let parsed = match query.query(Program::Ip, &["-j", "route", "show", "table", &text]) {
            Ok(body) => parse_routes(&body)?,
            Err(_) => Vec::new(),
        };
        routes.insert(*table, parsed);
    }
    let links = parse_links(&query.query(Program::Ip, &["-j", "link", "show"])?)?;
    let listed = query.query(Program::Ip, &["netns", "list"])?;
    let netns = listed
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect();
    Ok(Observed {
        rules,
        routes,
        links,
        netns,
    })
}

pub fn observe_uplinks(query: &mut dyn NetQuery) -> Result<Vec<Uplink>, NetError> {
    let v4 = uplink::parse_default_routes(&query.query(
        Program::Ip,
        &["-j", "route", "show", "default", "table", "main"],
    )?)?;
    let v6 = match query.query(
        Program::Ip,
        &["-j", "-6", "route", "show", "default", "table", "main"],
    ) {
        Ok(body) => uplink::parse_default_routes(&body)?,
        Err(_) => Vec::new(),
    };
    let addrs = uplink::parse_addrs(&query.query(Program::Ip, &["-j", "-4", "addr", "show"])?)?;
    Ok(uplink::uplinks(&v4, &v6, &addrs))
}
