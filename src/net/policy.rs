//! Политика туннеля из каталога владельца. Нет файла — безопасные значения по умолчанию.

use serde::{Deserialize, Serialize};

use crate::profiles::{FakeIpPolicy, Ipv6Policy};

use super::plan::NetError;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifetime {
    #[default]
    Pinned,
    Refcount,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureMode {
    #[default]
    Block,
    Failover,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TunnelPolicy {
    pub ipv6: Ipv6Policy,
    pub fake_ip: FakeIpPolicy,
    pub lifetime: Lifetime,
    pub failure: FailureMode,
}

pub const MAX_POLICY_BYTES: usize = 1024;

pub fn parse(bytes: &[u8]) -> Result<TunnelPolicy, NetError> {
    if bytes.len() > MAX_POLICY_BYTES {
        return Err(NetError::Unsupported);
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| NetError::Unsupported)?;
    if !value.is_object() {
        return Err(NetError::Unsupported);
    }
    serde_json::from_value(value).map_err(|_| NetError::Unsupported)
}
