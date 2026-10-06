#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SubPub {
    /// неизменный id записи: подтверждение в TUI адресует его, а не номер строки
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub host: String,
    pub updated: i64,
    pub nodes: usize,
    pub info: Option<SubInfo>,
    pub error: String,
    pub active: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct VpnState {
    #[serde(default)]
    pub core_version: String,
    #[serde(default)]
    pub core_latest: String,
    #[serde(default)]
    pub flclash_tag: String,
    #[serde(default)]
    pub flclash_applied: String,
    #[serde(default)]
    pub checked: i64,
    #[serde(default)]
    pub geo_updated: i64,
    #[serde(default)]
    pub subs: Vec<SubPub>,
    #[serde(default)]
    pub event: String,
    #[serde(default)]
    pub event_time: i64,
    #[serde(default)]
    pub last_failure: Option<LastFailure>,
}

/// Последняя ошибка остаётся доступна после выхода команды и перезапуска helper.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LastFailure {
    pub stage: String,
    pub time: i64,
    pub subscription: String,
    pub reason: String,
    #[serde(default)]
    pub resolved: Option<i64>,
}
fn failure_path() -> PathBuf { Path::new(&home()).join("last-error.json") }
pub fn last_failure() -> Option<LastFailure> {
    fs::read(failure_path()).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok())
}
fn safe_diagnostic(error: &str, sub: Option<&Sub>) -> String {
    let mut safe = error.to_owned();
    // Hide the entire URL-bearing error before replacing a known URL: appended
    // query parameters and redirected addresses may contain different secrets.
    if scrub_stored_error(&mut safe) { return safe; }
    if let Some(sub) = sub {
        if !sub.url.is_empty() { safe = safe.replace(&sub.url, "[subscription URL hidden]"); }
        // mihomo can echo credentials from malformed node definitions.
        if let Ok(body) = fs::read_to_string(profile_path(sub, &sub.kind)) {
            match parse_profile(&body) {
                Ok(profile) => redact_credentials(&profile, &mut safe),
                Err(_) if sub.kind == "clash" => return "Profile parsing failed; see the command output".into(),
                Err(_) => {},
            }
        }
    }
    scrub_stored_error(&mut safe);
    safe.chars().filter(|c| !c.is_control() || *c == '\n').take(2000).collect()
}
fn redact_credentials(value: &Value, text: &mut String) {
    match value {
        Value::Mapping(map) => for (key, value) in map {
            let key = key.as_str().unwrap_or("").to_ascii_lowercase();
            if ["password", "uuid", "token", "private-key", "public-key", "short-id", "obfs-password", "age-secret-key", "auth", "auth-str", "header"].contains(&key.as_str()) {
                redact_strings(value, text);
            } else { redact_credentials(value, text); }
        },
        Value::Sequence(values) => for value in values { redact_credentials(value, text); },
        _ => {},
    }
}
fn redact_strings(value: &Value, text: &mut String) {
    match value {
        Value::String(secret) if !secret.is_empty() => *text = text.replace(secret, "[hidden]"),
        Value::Mapping(map) => for (_, value) in map { redact_strings(value, text); },
        Value::Sequence(values) => for value in values { redact_strings(value, text); },
        _ => {},
    }
}
fn store_failure_locked(stage: &str, sub: Option<&Sub>, reason: &str) {
    let failure = LastFailure { stage: stage.into(), time: now(), subscription: sub.map(|s| s.name.clone()).unwrap_or_default(), reason: safe_diagnostic(reason, sub), resolved: None };
    let mut state = load_state();
    state.last_failure = Some(failure.clone());
    let _ = save_json("vpn.json", &state);
    if let Err(error) = serde_json::to_vec_pretty(&failure).map_err(|e| e.to_string()).and_then(|bytes| write_private(failure_path().to_str().unwrap_or(""), &bytes)) {
        eprintln!("VPN: could not save diagnostic: {error}");
    }
}
pub fn record_failure(stage: &str, error: &str) {
    if let Ok(_lock) = vpn_config_lock(true) {
        let subs = load_subs().unwrap_or_default();
        store_failure_locked(stage, subs.list.iter().find(|s| s.id == subs.active), error);
    }
}
fn resolve_failure_locked() {
    if let Some(mut failure) = last_failure() {
        failure.resolved = Some(now());
        let mut state = load_state(); state.last_failure = Some(failure.clone());
        let _ = save_json("vpn.json", &state);
        if let Ok(bytes) = serde_json::to_vec_pretty(&failure) { let _ = write_private(failure_path().to_str().unwrap_or(""), &bytes); }
    }
}
pub fn resolve_failure() {
    if let Ok(_lock) = vpn_config_lock(true) { resolve_failure_locked(); }
}

pub fn load_state() -> VpnState {
    let mut state: VpnState = load_json("vpn.json");
    let mut changed = false;
    for sub in &mut state.subs {
        let safe_name = sanitize_profile_name(&sub.name);
        if safe_name != sub.name {
            sub.name = safe_name;
            changed = true;
        }
        changed |= scrub_stored_error(&mut sub.error);
    }
    if changed {
        let _ = save_json("vpn.json", &state);
    }
    state
}

fn publish_state(s: &Subs) {
    let mut st = load_state();
    st.subs = s
        .list
        .iter()
        .map(|x| {
            let mut error = x.error.clone();
            scrub_stored_error(&mut error);
            SubPub {
                id: x.id.clone(),
                name: sanitize_profile_name(&x.name),
                host: host_of(&mask_url(&x.url)).to_string(),
                updated: x.updated,
                nodes: x.nodes,
                info: x.info.clone(),
                error,
                active: x.id == s.active,
            }
        })
        .collect();
    let _ = save_json("vpn.json", &st);
}

// ======================= конфиг mihomo =======================

