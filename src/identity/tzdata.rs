//! Проверка часового пояса по tzdata `zone.tab`.

use std::fmt;
use std::fs;
use std::path::Path;

pub const SYSTEM_TZDIR: &str = "/usr/share/zoneinfo";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoneError {
    BadZoneName,
    TzdataUnavailable,
    ZoneNotInCountry,
    ZoneMissing,
}

impl ZoneError {
    pub fn code(self) -> &'static str {
        match self {
            Self::BadZoneName => "BadZoneName",
            Self::TzdataUnavailable => "TzdataUnavailable",
            Self::ZoneNotInCountry => "ZoneNotInCountry",
            Self::ZoneMissing => "ZoneMissing",
        }
    }
}

impl fmt::Display for ZoneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BadZoneName => t!("неверное имя часового пояса"),
            Self::TzdataUnavailable => t!("база tzdata недоступна"),
            Self::ZoneNotInCountry => t!("часовой пояс не относится к стране"),
            Self::ZoneMissing => t!("файл часового пояса отсутствует"),
        })
    }
}

pub fn verify_zone(tzdir: &Path, country: &str, zone: &str) -> Result<(), ZoneError> {
    if !valid_zone_name(zone) {
        return Err(ZoneError::BadZoneName);
    }
    let table =
        fs::read_to_string(tzdir.join("zone.tab")).map_err(|_| ZoneError::TzdataUnavailable)?;
    if !zone_listed(&table, country, zone) {
        return Err(ZoneError::ZoneNotInCountry);
    }
    if !tzdir.join(zone).is_file() {
        return Err(ZoneError::ZoneMissing);
    }
    Ok(())
}

fn valid_zone_name(zone: &str) -> bool {
    if zone.is_empty() || zone.len() > 64 || zone.starts_with('/') {
        return false;
    }
    let mut seen = false;
    for segment in zone.split('/') {
        seen = true;
        if segment.is_empty() {
            return false;
        }
        if !segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'-'))
        {
            return false;
        }
    }
    seen
}

fn zone_listed(table: &str, country: &str, zone: &str) -> bool {
    table.lines().any(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return false;
        }
        let mut columns = line.split('\t');
        let code = columns.next().unwrap_or("");
        let _coordinates = columns.next();
        let name = columns
            .next()
            .unwrap_or("")
            .split_whitespace()
            .next()
            .unwrap_or("");
        code == country && name == zone
    })
}
