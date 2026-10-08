// `cm vpn use source:ID` while `vpn_store_source` is on.
//
// The host config is still the legacy generator. Only the node source changes:
// inline `proxies` from the Store replace the subscription `proxy-providers`.
// TUN, DNS, geodata and saved selection stay on that generator.

use crate::core::mihomo::{policy, proxy_values};
use crate::profiles::{Id, Store};
use crate::sources::{self, read_source_artifact};

pub struct StoreSwitch {
    pub source_id: String,
    pub warnings: Vec<String>,
}

/// Active Store source when the flag is on and a source has been selected.
pub fn active_store_label(c: &Config) -> Option<String> {
    if !c.vpn_store_source {
        return None;
    }
    let subs = load_subs().ok()?;
    if subs.store_source.is_empty() {
        None
    } else {
        Some(t!("источник Store {0}", subs.store_source))
    }
}

/// `cm vpn store on|off`. Turning the flag off while a Store source is selected
/// is refused: the next build would otherwise silently return to the legacy provider.
/// The same path serves the TUI, which must re-exec `cm vpn store` so privileges
/// match the other mutating `cm vpn` commands.
pub fn set_store_source_flag(on: bool, c: &mut Config) -> Result<(), String> {
    if !crate::common::is_root() && !crate::common::test_mode() {
        return Err(t!("cm: нужны права root (sudo/doas не найдены)").into());
    }
    if !on {
        let subs = load_subs()?;
        if !subs.store_source.is_empty() {
            return Err(t!("источник Store ещё выбран: сначала cm vpn use N").into());
        }
    }
    c.set("vpn_store_source", if on { "1" } else { "0" })?;
    c.save().map_err(|error| t!("не сохранено: {0}", error))?;
    Ok(())
}

pub fn use_store_source(id: &str, c: &Config) -> Result<StoreSwitch, String> {
    if !c.vpn_store_source {
        return Err(t!("флаг vpn_store_source выключен").into());
    }
    let mut reload = || {
        reload()?;
        if !running() {
            return Err(t!("ядро не подтвердило конфиг после переключения").into());
        }
        Ok(())
    };
    switch_store_source(id, c, service_active(), &mut reload)
}

pub fn switch_store_source(
    id: &str,
    c: &Config,
    active: bool,
    reload_core: &mut impl FnMut() -> Result<(), String>,
) -> Result<StoreSwitch, String> {
    let source_id = Id::new(id).map_err(|_| t!("неверный идентификатор источника").to_string())?;
    let warnings = source_warnings(&source_id)?;
    let _vpn_files = vpn_config_lock(true)?;
    let latest = if Path::new(&conf_path()).exists() {
        Config::load(c.mirrors.clone())?
    } else {
        c.clone()
    };
    if !latest.vpn_store_source {
        return Err(t!("флаг vpn_store_source выключен").into());
    }
    let _lock = subscriptions_lock(true)?;
    let mut subs = load_subs()?;
    let previous_subs = subs.clone();
    let mut stage = "build";
    let result = (|| {
        subs.store_source = source_id.as_str().to_owned();
        let candidate = build_config_for(&latest, &subs)?;
        stage = "validate";
        validate_candidate(&candidate)?;
        let previous = match fs::read(config_path()) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !active => None,
            Err(error) => return Err(format!("{}: {error}", config_path())),
        };
        stage = "write";
        let applied = write_private(&config_path(), candidate.as_bytes()).and_then(|_| {
            stage = "apply";
            if active { reload_core() } else { Ok(()) }
        });
        let saving = applied.is_ok();
        if saving {
            stage = "save";
        }
        let committed = applied.and_then(|_| save_subs(&subs));
        if let Err(error) = committed {
            let mut rollback_errors = Vec::new();
            let restored = match previous {
                Some(bytes) => write_private(&config_path(), &bytes),
                None => match fs::remove_file(config_path()) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error.to_string()),
                },
            };
            match restored {
                Ok(()) if active => {
                    if let Err(error) = reload_core() {
                        rollback_errors.push(error);
                    }
                }
                Ok(()) => {}
                Err(error) => rollback_errors.push(error),
            }
            if saving && let Err(error) = save_subs(&previous_subs) {
                rollback_errors.push(error);
            }
            return Err(if rollback_errors.is_empty() {
                format!("{error}; previous VPN configuration restored")
            } else {
                format!(
                    "{error}; VPN rollback failed: {}",
                    rollback_errors.join("; ")
                )
            });
        }
        resolve_failure_locked();
        Ok(StoreSwitch {
            source_id: source_id.as_str().to_owned(),
            warnings,
        })
    })();
    if let Err(error) = &result {
        store_failure_locked(stage, None, error);
    }
    result
}

fn source_warnings(id: &Id) -> Result<Vec<String>, String> {
    let store = open_profile_store()?;
    let snapshot = store
        .read_snapshot()
        .map_err(|_| t!("источник не найден").to_string())?;
    let source = snapshot
        .sources
        .get(id)
        .ok_or_else(|| t!("источник не найден").to_string())?;
    let proxies = proxy_values(&store, id).map_err(|_| t!("источник не найден").to_string())?;
    let names = proxy_names(&proxies);
    let artifact =
        read_source_artifact(&store, id).map_err(|_| t!("источник не найден").to_string())?;
    let policy = publisher_policy(artifact.raw_body(), &names)?;
    let mut warnings = Vec::new();
    if policy.is_empty()
        && let Some(provenance) = &source.provenance
    {
        let pending: Vec<&str> = provenance
            .omissions
            .section_names
            .iter()
            .map(String::as_str)
            .filter(|name| {
                matches!(
                    *name,
                    "proxy-groups" | "rules" | "sub-rules" | "rule-providers"
                )
            })
            .collect();
        if !pending.is_empty() {
            warnings.push(t!(
                "группы и правила издателя не перенесены: {0}",
                pending.join(", ")
            ));
        }
    }
    Ok(warnings)
}

fn open_profile_store() -> Result<Store, String> {
    let root = sources::cli::store_root();
    if root.exists() {
        Store::open(&root).map_err(|_| t!("источник не найден").to_string())
    } else {
        Err(t!("источник не найден").into())
    }
}

/// Profile part for a selected Store source. It replaces the legacy subscription
/// body: nodes, groups and rules all come from the Store, so nothing of another
/// subscription is mixed in. `build_config_for` then adds the CM settings, CM user
/// and geodata rules in front, and the auto group, exactly as for a legacy profile.
///
/// `None` keeps the legacy path: the flag is off, or no source is selected.
fn store_profile(c: &Config, subs: &Subs) -> Result<Option<Mapping>, String> {
    if !c.vpn_store_source {
        if !subs.store_source.is_empty() {
            return Err(t!("источник Store выбран при выключенном флаге").into());
        }
        return Ok(None);
    }
    if subs.store_source.is_empty() {
        return Ok(None);
    }
    let id = Id::new(&subs.store_source)
        .map_err(|_| t!("неверный идентификатор источника").to_string())?;
    let store = open_profile_store()?;
    let proxies = proxy_values(&store, &id).map_err(|_| t!("источник не найден").to_string())?;
    let names: Vec<String> = proxies
        .iter()
        .filter_map(|proxy| {
            proxy
                .get("name")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .collect();
    let artifact =
        read_source_artifact(&store, &id).map_err(|_| t!("источник не найден").to_string())?;
    let policy = publisher_policy(artifact.raw_body(), &proxy_names(&proxies))?;
    let mut document = serde_json::Map::new();
    document.insert("proxies".into(), serde_json::Value::Array(proxies));
    if policy.groups.is_empty() {
        // The same single selector the legacy path builds for a URI list.
        document.insert(
            "proxy-groups".into(),
            serde_json::json!([{"name": "Proxy", "type": "select", "proxies": names}]),
        );
        let mut rules: Vec<serde_json::Value> = policy
            .rules
            .iter()
            .filter(|rule| !rule.starts_with("MATCH,"))
            .cloned()
            .map(serde_json::Value::String)
            .collect();
        rules.push(serde_json::Value::String("MATCH,Proxy".into()));
        document.insert("rules".into(), serde_json::Value::Array(rules));
    } else {
        for group in &policy.groups {
            let name = group
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if is_auto_group(name) {
                return Err(t!("имя группы издателя совпало с уже существующей").into());
            }
        }
        document.insert(
            "proxy-groups".into(),
            serde_json::Value::Array(policy.groups.clone()),
        );
        if !policy.rules.is_empty() {
            let rules = policy
                .rules
                .iter()
                .cloned()
                .map(serde_json::Value::String)
                .collect();
            document.insert("rules".into(), serde_json::Value::Array(rules));
        }
    }
    if !policy.sub_rules.is_empty() {
        document.insert(
            "sub-rules".into(),
            serde_json::Value::Object(policy.sub_rules.clone()),
        );
    }
    if !policy.rule_providers.is_empty() {
        document.insert(
            "rule-providers".into(),
            serde_json::Value::Object(policy.rule_providers.clone()),
        );
    }
    match serde_yaml::to_value(serde_json::Value::Object(document)) {
        Ok(Value::Mapping(m)) => Ok(Some(m)),
        _ => Err(t!("не удалось собрать узлы источника").into()),
    }
}

fn publisher_policy(
    bytes: &[u8],
    names: &std::collections::BTreeSet<String>,
) -> Result<policy::PublisherPolicy, String> {
    policy::from_raw(bytes, names).map_err(|_| t!("правила издателя не приняты").to_string())
}

fn proxy_names(proxies: &[serde_json::Value]) -> std::collections::BTreeSet<String> {
    proxies
        .iter()
        .filter_map(|proxy| {
            proxy
                .get("name")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    #[test]
    fn a_rejected_publisher_policy_is_an_error() {
        let names = BTreeSet::from(["n1".to_owned()]);
        let body = br#"{"proxies":[{"name":"n1","type":"ss","server":"203.0.113.8","port":443,"cipher":"aes-128-gcm","password":"secret-marker-v04"}],"proxy-groups":[{"name":"Relay","type":"relay","proxies":["n1"]}],"rules":["MATCH,n1"]}"#;
        let error = super::publisher_policy(body, &names).unwrap_err();
        assert!(error.contains("не приняты"), "{error}");
        assert!(!error.contains("secret-marker-v04"));
    }
}
