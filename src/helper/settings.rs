#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SettingsRuntime { NotNeeded, Applied, Pending { reason: String }, SavedButNotApplied { reason: String, retry: String } }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SettingsReply { pub config: Config, pub revision: String, pub runtime: SettingsRuntime }

impl SettingsReply {
    pub fn runtime_error(&self) -> Option<String> {
        match &self.runtime {
            SettingsRuntime::SavedButNotApplied { reason, retry } => Some(format!("settings saved, but not applied: {reason}; retry: {retry}")),
            _ => None,
        }
    }
}

/// Validate and commit first; runtime failure is represented separately from persistence.
pub fn apply_setting(b: &dyn Backend, key: &str, value: &str, user: Option<&UserContext>, log: Log) -> Result<SettingsReply, String> {
    let mut pending = None;
    let mut old_autostart = false;
    let c = Config::update(b.default_mirrors(), |candidate| {
        old_autostart = candidate.vpn_autostart;
        candidate.set(key, value)?;
        if key.starts_with("vpn_") {
            if vpn::load_subs()?.list.is_empty() {
                pending = Some("VPN subscription has not been added yet".to_string());
            } else {
                vpn::build_config(candidate)?;
            }
        }
        Ok(())
    })?;
    let revision = c.revision();
    let result = match key {
        "vpn_sub_update_h" | "vpn_core_check_h" => Ok(()),
        "vpn_autostart" if c.vpn_autostart != old_autostart && !unit_state(vpn::SERVICE).is_empty() => vpn::autostart(c.vpn_autostart),
        _ if pending.is_some() => Ok(()),
        "vpn_mode" => vpn::apply_saved_mode(&c, user, log),
        k if k.starts_with("vpn_") => vpn::apply(&c, user, log),
        _ => Ok(()),
    };
    let runtime = match result {
        Err(reason) => SettingsRuntime::SavedButNotApplied { reason, retry: format!("cm vpn restart (revision {revision})") },
        Ok(()) if pending.is_some() => SettingsRuntime::Pending { reason: pending.unwrap() },
        Ok(()) if key.starts_with("vpn_") && !vpn::service_active() => SettingsRuntime::Pending { reason: "VPN is not running; the saved configuration will be used on its next start".into() },
        Ok(()) if key.starts_with("vpn_") => SettingsRuntime::Applied,
        Ok(()) => SettingsRuntime::NotNeeded,
    };
    let current = Config::load(b.default_mirrors())?;
    let current_revision = current.revision();
    let runtime = if key.starts_with("vpn_") && current_revision != revision {
        SettingsRuntime::SavedButNotApplied { reason: "configuration changed during runtime apply".into(), retry: "cm vpn restart".into() }
    } else { runtime };
    Ok(SettingsReply { config: current, revision: current_revision, runtime })
}

