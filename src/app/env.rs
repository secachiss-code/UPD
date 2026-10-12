//! Окружение одной сессии. Правка пресета уже созданную сессию не меняет.

use serde::{Deserialize, Serialize};

use crate::env::private::{self, DataState, IpcPolicy};
use crate::identity::tzdata::valid_zone_name;
use crate::profiles::{EnvironmentPreset, EnvironmentProfile, Id, SCHEMA_VERSION};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataRef {
    pub profile: String,
    pub state: DataState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionFile {
    pub timezone: String,
    pub locale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<DataRef>,
    #[serde(default)]
    pub ipc: IpcPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvError {
    Malformed,
    BadZone,
    BadLocale,
    BadProfile,
}

pub const MAX_SESSION_BYTES: usize = 4096;

pub fn parse_session(bytes: &[u8]) -> Result<SessionFile, EnvError> {
    let file: SessionFile = serde_json::from_slice(bytes).map_err(|_| EnvError::Malformed)?;
    if !valid_zone_name(&file.timezone) {
        return Err(EnvError::BadZone);
    }
    if !valid_locale(&file.locale) {
        return Err(EnvError::BadLocale);
    }
    if let Some(data) = &file.data
        && !private::valid_name(&data.profile)
    {
        return Err(EnvError::BadProfile);
    }
    Ok(file)
}

pub fn session_file(
    preset: &EnvironmentPreset,
    data: Option<DataRef>,
    ipc: IpcPolicy,
) -> SessionFile {
    SessionFile {
        timezone: preset.timezone.clone(),
        locale: preset.locale.clone(),
        data,
        ipc,
    }
}

pub fn environment_profile(
    id: Id,
    session_id: Id,
    preset: &EnvironmentPreset,
) -> EnvironmentProfile {
    EnvironmentProfile {
        schema_version: SCHEMA_VERSION,
        id,
        session_id,
        preset_id: preset.id.clone(),
        timezone: preset.timezone.clone(),
        locale: preset.locale.clone(),
        languages: preset.languages.clone(),
    }
}

fn valid_locale(locale: &str) -> bool {
    (1..=64).contains(&locale.len())
        && locale
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-' | b'@'))
}
