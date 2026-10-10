//! Модель личности браузера.

use super::engine::EngineInfo;
use super::presets::EnvironmentProfile;
use serde::Deserialize;
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

/// Гипотеза Q27: менять только решением пользователя.
pub const HISTORY_LIMIT: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Strategy {
    Local,
    Crowd,
}

impl Strategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Crowd => "crowd",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub generation: u32,
    pub at: i64,
    pub change: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserIdentityProfile {
    #[serde(deserialize_with = "deserialize_schema_version")]
    pub schema_version: u32,
    pub id: String,
    pub strategy: Strategy,
    pub browser: PathBuf,
    pub engine: EngineInfo,
    pub environment: Option<EnvironmentProfile>,
    pub extra_args: Vec<String>,
    pub created_at: i64,
    pub generation: u32,
    pub history: Vec<HistoryEntry>,
    pub bypass_suspected: bool,
}

impl BrowserIdentityProfile {
    pub fn from_json(text: &str) -> Result<Self, ModelError> {
        match serde_json::from_str(text) {
            Ok(profile) => Ok(profile),
            Err(error) if error.to_string().contains("UnsupportedSchema") => {
                Err(ModelError::UnsupportedSchema)
            }
            Err(_) => Err(ModelError::Invalid),
        }
    }

    pub fn record(&mut self, at: i64, change: &str) {
        self.generation = self.generation.saturating_add(1);
        self.history.push(HistoryEntry {
            generation: self.generation,
            at,
            change: change.to_string(),
        });
        if self.history.len() > HISTORY_LIMIT {
            let extra = self.history.len() - HISTORY_LIMIT;
            self.history.drain(0..extra);
        }
    }
}

impl fmt::Debug for BrowserIdentityProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrowserIdentityProfile")
            .field("schema_version", &self.schema_version)
            .field("id", &self.id)
            .field("strategy", &self.strategy)
            .field("browser", &self.browser.is_absolute())
            .field("engine", &self.engine)
            .field("environment", &self.environment)
            .field("extra_args", &self.extra_args.len())
            .field("created_at", &self.created_at)
            .field("generation", &self.generation)
            .field("history", &self.history)
            .field("bypass_suspected", &self.bypass_suspected)
            .finish()
    }
}

fn deserialize_schema_version<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let version = u32::deserialize(deserializer)?;
    if version != 1 {
        return Err(serde::de::Error::custom("UnsupportedSchema"));
    }
    Ok(version)
}

pub fn validate_id(id: &str) -> Result<(), ModelError> {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > 32 {
        return Err(ModelError::BadId);
    }
    let first = bytes[0];
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(ModelError::BadId);
    }
    if bytes[1..]
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
    {
        Ok(())
    } else {
        Err(ModelError::BadId)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelError {
    UnsupportedSchema,
    BadId,
    Invalid,
}

impl ModelError {
    pub fn code(self) -> &'static str {
        match self {
            Self::UnsupportedSchema => "UnsupportedSchema",
            Self::BadId => "BadId",
            Self::Invalid => "Invalid",
        }
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedSchema => t!("неподдерживаемая схема личности"),
            Self::BadId => t!("неверный идентификатор личности"),
            Self::Invalid => t!("неверные данные личности"),
        })
    }
}
