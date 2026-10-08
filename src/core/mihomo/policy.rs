//! Publisher proxy groups and rules (D1, variant 3).
//!
//! `relay` is rejected. A rule target must name a proxy, a group, a sub-rule,
//! or a built-in action. `sub-rules` are checked for cycles. Rule providers
//! are only `inline`. GEOIP and GEOSITE are kept for the host path and refused
//! when the same policy is compiled for an app worker.

use std::collections::{BTreeSet, HashSet};

use serde_json::{Map, Value};

use super::config::ConfigError;

const GROUP_TYPES: &[&str] = &["select", "url-test", "fallback", "load-balance"];
const BUILTIN: &[&str] = &["DIRECT", "REJECT", "REJECT-DROP", "PASS", "COMPATIBLE"];
const MAX_GROUPS: usize = 256;
const MAX_RULES: usize = 4096;

/// Fixed policy failures. Display and Debug carry no rule text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyError {
    Relay,
    GhostTarget,
    Cycle,
    GeoRule,
    UnsupportedProvider,
    Invalid,
}

impl PolicyError {
    fn phrase(self) -> &'static str {
        match self {
            Self::Relay => "publisher group type relay is rejected",
            Self::GhostTarget => "publisher rule target does not exist",
            Self::Cycle => "publisher sub-rules contain a cycle",
            Self::GeoRule => "publisher geo rule is rejected for an app worker",
            Self::UnsupportedProvider => "publisher rule provider is not inline",
            Self::Invalid => "publisher policy is invalid",
        }
    }
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.phrase())
    }
}

impl std::error::Error for PolicyError {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PublisherPolicy {
    pub groups: Vec<Value>,
    pub rules: Vec<String>,
    pub sub_rules: Map<String, Value>,
    /// Inline `rule-providers` that `RULE-SET` rules name.
    pub rule_providers: Map<String, Value>,
}

impl PublisherPolicy {
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
            && self.rules.is_empty()
            && self.sub_rules.is_empty()
            && self.rule_providers.is_empty()
    }
}

/// Validate sections taken from an already parsed native document.
pub fn accept_publisher(
    proxy_names: &BTreeSet<String>,
    groups: Option<&Value>,
    rules: Option<&Value>,
    sub_rules: Option<&Value>,
    rule_providers: Option<&Value>,
) -> Result<PublisherPolicy, PolicyError> {
    let groups = match groups {
        None => Vec::new(),
        Some(Value::Array(items)) => parse_groups(items, proxy_names)?,
        Some(_) => return Err(PolicyError::Invalid),
    };
    let mut group_names = BTreeSet::new();
    for group in &groups {
        let name = group
            .get("name")
            .and_then(Value::as_str)
            .ok_or(PolicyError::Invalid)?;
        if !group_names.insert(name.to_owned()) {
            return Err(PolicyError::Invalid);
        }
    }
    let rule_providers = inline_providers(rule_providers)?;
    let providers: BTreeSet<String> = rule_providers.keys().cloned().collect();
    let (sub_rules, sub_names) = parse_sub_rules(sub_rules)?;
    let rules = match rules {
        None => Vec::new(),
        Some(Value::Array(items)) => parse_rules(items)?,
        Some(_) => return Err(PolicyError::Invalid),
    };
    if rules.len() > MAX_RULES {
        return Err(PolicyError::Invalid);
    }
    let known = Known {
        proxies: proxy_names,
        groups: &group_names,
        subs: &sub_names,
        providers: &providers,
    };
    for rule in &rules {
        check_target(rule, &known)?;
    }
    for items in sub_rules.values() {
        let Some(list) = items.as_array() else {
            return Err(PolicyError::Invalid);
        };
        for rule in list {
            let text = rule.as_str().ok_or(PolicyError::Invalid)?;
            check_target(text, &known)?;
        }
    }
    check_cycles(&sub_rules)?;
    Ok(PublisherPolicy {
        groups,
        rules,
        sub_rules,
        rule_providers,
    })
}

/// Re-read groups and rules from a stored raw body. URI lists have none.
pub fn from_raw(
    bytes: &[u8],
    proxy_names: &BTreeSet<String>,
) -> Result<PublisherPolicy, PolicyError> {
    let text = std::str::from_utf8(bytes).map_err(|_| PolicyError::Invalid)?;
    let text = text.trim().trim_start_matches('\u{feff}');
    if text.is_empty()
        || text.contains("://") && !text.starts_with('{') && !text.contains("proxies:")
    {
        return Ok(PublisherPolicy::default());
    }
    let root = if text.starts_with('{') {
        serde_json::from_str::<Value>(text).map_err(|_| PolicyError::Invalid)?
    } else {
        let yaml: serde_yaml::Value =
            serde_yaml::from_str(text).map_err(|_| PolicyError::Invalid)?;
        serde_json::to_value(yaml).map_err(|_| PolicyError::Invalid)?
    };
    let Some(object) = root.as_object() else {
        return Ok(PublisherPolicy::default());
    };
    accept_publisher(
        proxy_names,
        object.get("proxy-groups"),
        object.get("rules"),
        object.get("sub-rules"),
        object.get("rule-providers"),
    )
}

/// Add publisher groups and rules to a worker document. GEO rules are refused.
///
/// Rule order: generator (CM user) rules, then publisher rules, then one final
/// `MATCH` — the publisher's when it has one, otherwise the generator's.
pub fn merge_worker(document: &mut Value, policy: &PublisherPolicy) -> Result<(), ConfigError> {
    if policy_has_geo(policy) {
        return Err(ConfigError::InvalidRule);
    }
    let object = document.as_object_mut().ok_or(ConfigError::InvalidRule)?;
    if !policy.groups.is_empty() {
        let groups = object
            .entry("proxy-groups")
            .or_insert_with(|| Value::Array(Vec::new()));
        let Some(list) = groups.as_array_mut() else {
            return Err(ConfigError::InvalidRule);
        };
        let mut taken: BTreeSet<String> = list
            .iter()
            .filter_map(|group| group.get("name").and_then(Value::as_str))
            .map(str::to_owned)
            .collect();
        for group in &policy.groups {
            let name = group.get("name").and_then(Value::as_str).unwrap_or("");
            if !taken.insert(name.to_owned()) {
                return Err(ConfigError::InvalidRule);
            }
            list.push(group.clone());
        }
    }
    for (key, section) in [
        ("sub-rules", &policy.sub_rules),
        ("rule-providers", &policy.rule_providers),
    ] {
        if section.is_empty() {
            continue;
        }
        if object.contains_key(key) {
            return Err(ConfigError::InvalidRule);
        }
        object.insert(key.to_owned(), Value::Object(section.clone()));
    }
    if !policy.rules.is_empty() {
        let rules = object
            .entry("rules")
            .or_insert_with(|| Value::Array(Vec::new()));
        let Some(list) = rules.as_array_mut() else {
            return Err(ConfigError::InvalidRule);
        };
        let is_match = |rule: &Value| rule.as_str().is_some_and(|text| text.starts_with("MATCH,"));
        let generator_match = list.iter().find(|rule| is_match(rule)).cloned();
        list.retain(|rule| !is_match(rule));
        let mut publisher_match = None;
        for rule in &policy.rules {
            let rule = Value::String(rule.clone());
            if is_match(&rule) {
                publisher_match.get_or_insert(rule);
            } else {
                list.push(rule);
            }
        }
        if let Some(rule) = publisher_match.or(generator_match) {
            list.push(rule);
        }
    }
    Ok(())
}

pub fn policy_has_geo(policy: &PublisherPolicy) -> bool {
    policy.rules.iter().any(|rule| is_geo(rule))
        || policy.sub_rules.values().any(|value| {
            value
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item.as_str().is_some_and(is_geo)))
        })
}

struct Known<'a> {
    proxies: &'a BTreeSet<String>,
    groups: &'a BTreeSet<String>,
    subs: &'a BTreeSet<String>,
    providers: &'a BTreeSet<String>,
}

fn parse_groups(items: &[Value], proxies: &BTreeSet<String>) -> Result<Vec<Value>, PolicyError> {
    if items.len() > MAX_GROUPS {
        return Err(PolicyError::Invalid);
    }
    let mut names = BTreeSet::new();
    for item in items {
        let object = item.as_object().ok_or(PolicyError::Invalid)?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .ok_or(PolicyError::Invalid)?;
        if name.is_empty()
            || name.chars().any(|c| c.is_ascii_control())
            || !names.insert(name.to_owned())
        {
            return Err(PolicyError::Invalid);
        }
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or(PolicyError::Invalid)?;
        if kind.eq_ignore_ascii_case("relay") {
            return Err(PolicyError::Relay);
        }
        if !GROUP_TYPES
            .iter()
            .any(|allowed| kind.eq_ignore_ascii_case(allowed))
        {
            return Err(PolicyError::Invalid);
        }
    }
    let known_groups = &names;
    for item in items {
        let object = item.as_object().ok_or(PolicyError::Invalid)?;
        // Publisher proxy-providers are not imported (D1), so `use` names nothing.
        if object.contains_key("use") {
            return Err(PolicyError::UnsupportedProvider);
        }
        if let Some(Value::Array(members)) = object.get("proxies") {
            for member in members {
                let name = member.as_str().ok_or(PolicyError::Invalid)?;
                if !proxies.contains(name)
                    && !known_groups.contains(name)
                    && !BUILTIN.contains(&name)
                {
                    return Err(PolicyError::GhostTarget);
                }
            }
        } else if object.get("include-all") != Some(&Value::Bool(true))
            && object.get("include-all-proxies") != Some(&Value::Bool(true))
        {
            return Err(PolicyError::Invalid);
        }
    }
    Ok(items.to_vec())
}

fn inline_providers(providers: Option<&Value>) -> Result<Map<String, Value>, PolicyError> {
    let Some(value) = providers else {
        return Ok(Map::new());
    };
    let Some(object) = value.as_object() else {
        return Err(PolicyError::Invalid);
    };
    for (name, spec) in object {
        if name.is_empty() {
            return Err(PolicyError::Invalid);
        }
        let kind = spec
            .as_object()
            .and_then(|item| item.get("type"))
            .and_then(Value::as_str)
            .ok_or(PolicyError::Invalid)?;
        if !kind.eq_ignore_ascii_case("inline") {
            return Err(PolicyError::UnsupportedProvider);
        }
    }
    Ok(object.clone())
}

fn parse_sub_rules(
    value: Option<&Value>,
) -> Result<(Map<String, Value>, BTreeSet<String>), PolicyError> {
    let Some(value) = value else {
        return Ok((Map::new(), BTreeSet::new()));
    };
    let Some(object) = value.as_object() else {
        return Err(PolicyError::Invalid);
    };
    let mut names = BTreeSet::new();
    for (name, rules) in object {
        if name.is_empty() || !rules.is_array() || !names.insert(name.clone()) {
            return Err(PolicyError::Invalid);
        }
    }
    Ok((object.clone(), names))
}

fn parse_rules(items: &[Value]) -> Result<Vec<String>, PolicyError> {
    let mut rules = Vec::with_capacity(items.len());
    for item in items {
        let text = item.as_str().ok_or(PolicyError::Invalid)?;
        if text.is_empty() || text.chars().any(|c| c == '\n' || c.is_ascii_control()) {
            return Err(PolicyError::Invalid);
        }
        rules.push(text.to_owned());
    }
    Ok(rules)
}

fn check_target(rule: &str, known: &Known<'_>) -> Result<(), PolicyError> {
    let target = rule_target(rule).ok_or(PolicyError::Invalid)?;
    if rule_kind(rule).eq_ignore_ascii_case("RULE-SET") {
        let provider = rule.split(',').nth(1).unwrap_or("").trim();
        if !known.providers.contains(provider) {
            return Err(PolicyError::GhostTarget);
        }
    }
    if rule_kind(rule).eq_ignore_ascii_case("SUB-RULE") {
        let name = sub_rule_name(rule).ok_or(PolicyError::Invalid)?;
        if !known.subs.contains(name) {
            return Err(PolicyError::GhostTarget);
        }
    }
    if BUILTIN.contains(&target)
        || known.proxies.contains(target)
        || known.groups.contains(target)
        || known.subs.contains(target)
    {
        return Ok(());
    }
    Err(PolicyError::GhostTarget)
}

fn check_cycles(sub_rules: &Map<String, Value>) -> Result<(), PolicyError> {
    let mut seen = HashSet::new();
    let mut stack = HashSet::new();
    for name in sub_rules.keys() {
        if cycle(name, sub_rules, &mut seen, &mut stack) {
            return Err(PolicyError::Cycle);
        }
    }
    Ok(())
}

fn cycle(
    name: &str,
    sub_rules: &Map<String, Value>,
    seen: &mut HashSet<String>,
    stack: &mut HashSet<String>,
) -> bool {
    if !stack.insert(name.to_owned()) {
        return true;
    }
    if !seen.insert(name.to_owned()) {
        stack.remove(name);
        return false;
    }
    if let Some(items) = sub_rules.get(name).and_then(Value::as_array) {
        for item in items {
            let Some(text) = item.as_str() else {
                continue;
            };
            if let Some(next) = sub_rule_name(text)
                && cycle(next, sub_rules, seen, stack)
            {
                return true;
            }
        }
    }
    stack.remove(name);
    false
}

/// Mihomo writes `SUB-RULE,(NETWORK,TCP),name`: one condition in parentheses, then the sub-rule name.
fn sub_rule_name(rule: &str) -> Option<&str> {
    if !rule_kind(rule).eq_ignore_ascii_case("SUB-RULE") {
        return None;
    }
    let rest = rule.split_once(',')?.1.trim();
    let open = rest.find('(')?;
    let close = rest.rfind(')')?;
    if close <= open || rest[open + 1..close].trim().is_empty() {
        return None;
    }
    let name = rest[close + 1..].trim().trim_start_matches(',').trim();
    if name.is_empty() || name.contains(',') || name.contains('(') {
        return None;
    }
    Some(name)
}

fn rule_kind(rule: &str) -> &str {
    rule.split(',').next().unwrap_or("").trim()
}

fn rule_target(rule: &str) -> Option<&str> {
    let mut parts: Vec<&str> = rule
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if parts
        .last()
        .is_some_and(|part| part.eq_ignore_ascii_case("no-resolve"))
    {
        parts.pop();
    }
    parts.last().copied()
}

fn is_geo(rule: &str) -> bool {
    let kind = rule_kind(rule);
    kind.eq_ignore_ascii_case("GEOIP") || kind.eq_ignore_ascii_case("GEOSITE")
}
