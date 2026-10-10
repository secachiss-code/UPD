//! Представление туннеля из сессий и проверок текущего поколения. Оси не сводятся в «зелёный».

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::profiles::{
    ApplicationGroup, HostMode, HostPolicy, Session, SessionLifecycle, TunnelInstance, TunnelOwner,
    Verification, VerificationAxis, VerificationValue,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelView {
    pub name: String,
    pub host: String,
    pub apps: String,
    pub condition: String,
    pub axes: BTreeMap<String, String>,
    pub age_s: BTreeMap<String, Option<u64>>,
    pub failure: Option<String>,
    pub sessions: u32,
}

pub fn build(
    host: &HostPolicy,
    tunnel: &TunnelInstance,
    sessions: &[Session],
    verifications: &[Verification],
    _group: Option<&ApplicationGroup>,
    now_unix_ms: i64,
) -> TunnelView {
    let mut axes = BTreeMap::new();
    let mut age_s = BTreeMap::new();
    for axis in [
        VerificationAxis::Net,
        VerificationAxis::Region,
        VerificationAxis::State,
        VerificationAxis::App,
    ] {
        let name = axis_name(axis);
        let latest = verifications
            .iter()
            .filter(|item| {
                item.tunnel_instance_id == tunnel.id
                    && item.tunnel_generation == tunnel.generation
                    && item.axis == axis
            })
            .max_by_key(|item| item.evidence_at_unix_ms);
        match latest {
            Some(item) => {
                axes.insert(name.to_owned(), value_name(item.value).to_owned());
                let age = now_unix_ms.saturating_sub(item.evidence_at_unix_ms).max(0) as u64 / 1000;
                age_s.insert(name.to_owned(), Some(age));
            }
            None => {
                axes.insert(name.to_owned(), "unknown".to_owned());
                age_s.insert(name.to_owned(), None);
            }
        }
    }
    let condition = if axes.values().any(|value| value == "blocked") {
        "blocked"
    } else if axes
        .values()
        .any(|value| value == "error" || value == "partial")
    {
        "degraded"
    } else {
        "unknown"
    };
    let failure = if axes.get("net").map(String::as_str) == Some("error") {
        Some("api-down".to_owned())
    } else if axes.get("region").map(String::as_str) == Some("blocked") {
        Some("region-mismatch".to_owned())
    } else {
        None
    };
    let sessions = sessions
        .iter()
        .filter(|session| {
            session.tunnel_instance_id == tunnel.id && session.lifecycle == SessionLifecycle::Active
        })
        .count() as u32;
    TunnelView {
        name: tunnel.id.as_str().to_owned(),
        host: match host.mode {
            HostMode::Off => "off",
            HostMode::Proxy => "proxy",
            HostMode::Tunnel => "tunnel",
        }
        .to_owned(),
        apps: if matches!(tunnel.owner, TunnelOwner::Group { .. }) {
            "shared"
        } else {
            "separate"
        }
        .to_owned(),
        condition: condition.to_owned(),
        axes,
        age_s,
        failure,
        sessions,
    }
}

fn axis_name(axis: VerificationAxis) -> &'static str {
    match axis {
        VerificationAxis::Net => "net",
        VerificationAxis::Region => "region",
        VerificationAxis::State => "state",
        VerificationAxis::App => "app",
    }
}

fn value_name(value: VerificationValue) -> &'static str {
    match value {
        VerificationValue::Unknown => "unknown",
        VerificationValue::Partial => "partial",
        VerificationValue::Verified => "verified",
        VerificationValue::Blocked => "blocked",
        VerificationValue::Error => "error",
    }
}
