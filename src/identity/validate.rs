//! Согласованность личности до запуска.

use super::engine::{Brand, EngineInfo, Family};
use super::model::{BrowserIdentityProfile, Strategy};
use super::tzdata::verify_zone;
use super::verified::{self, Verification};
use std::fmt;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub errors: Vec<Violation>,
    pub warnings: Vec<Violation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Violation {
    StrategyEngineUnsupported,
    CrowdWithRegion,
    LocalNeedsEnvironment,
    ZoneInvalid,
    ForbiddenArgument(String),
    EngineChanged,
    UnverifiedVersion,
    LanguageNotRegional,
    BraveFarblesLanguages,
    BypassSuspected,
}

impl Violation {
    pub fn code(&self) -> &'static str {
        match self {
            Self::StrategyEngineUnsupported => "StrategyEngineUnsupported",
            Self::CrowdWithRegion => "CrowdWithRegion",
            Self::LocalNeedsEnvironment => "LocalNeedsEnvironment",
            Self::ZoneInvalid => "ZoneInvalid",
            Self::ForbiddenArgument(_) => "ForbiddenArgument",
            Self::EngineChanged => "EngineChanged",
            Self::UnverifiedVersion => "UnverifiedVersion",
            Self::LanguageNotRegional => "LanguageNotRegional",
            Self::BraveFarblesLanguages => "BraveFarblesLanguages",
            Self::BypassSuspected => "BypassSuspected",
        }
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::StrategyEngineUnsupported => {
                t!("стратегия crowd не поддерживается этим браузером")
            }
            Self::CrowdWithRegion => t!("стратегия crowd не задаёт страну"),
            Self::LocalNeedsEnvironment => t!("стратегия local требует окружение"),
            Self::ZoneInvalid => t!("часовой пояс не найден в tzdata"),
            Self::ForbiddenArgument(_) => t!("запрещённый аргумент"),
            Self::EngineChanged => t!("версия браузера изменилась"),
            Self::UnverifiedVersion => t!("версия браузера не проверена лабораторией"),
            Self::LanguageNotRegional => t!("язык не из пресета страны"),
            Self::BraveFarblesLanguages => t!("Brave сокращает языки профиля"),
            Self::BypassSuspected => t!("профиль открывали мимо CM: нужно cm identity confirm"),
        })
    }
}

const FORBIDDEN_ARGUMENTS: &[&str] = &[
    "--user-agent",
    "--remote-debugging-port",
    "--remote-debugging-pipe",
    "--enable-automation",
    "--headless",
    "--lang",
    "--accept-lang",
    "--user-data-dir",
    "--profile",
    "-profile",
    "--no-sandbox",
    "--marionette",
    "--remote-allow",
    // Overrides the zone that TZ sets: the page would see two different answers.
    "--time-zone-for-testing",
    // Another profile inside the same directory is outside the guard's fingerprint.
    "--profile-directory",
    "--ProfileManager",
    "-ProfileManager",
];

/// Gecko's profile switch is a bare `-P`; a prefix match would also catch `-Private-window`.
const FORBIDDEN_EXACT: &[&str] = &["-P"];

/// `lab` не меняет правила: флаги лаборатории добавляет план, не пользователь.
pub fn validate(
    profile: &BrowserIdentityProfile,
    engine: &EngineInfo,
    tzdir: &Path,
    lab: bool,
) -> Report {
    let _ = lab;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    if profile.strategy == Strategy::Crowd && engine.family == Family::Chromium {
        errors.push(Violation::StrategyEngineUnsupported);
    }
    if profile.strategy == Strategy::Crowd && profile.environment.is_some() {
        errors.push(Violation::CrowdWithRegion);
    }
    if profile.strategy == Strategy::Local && profile.environment.is_none() {
        errors.push(Violation::LocalNeedsEnvironment);
    }
    if profile.strategy == Strategy::Local
        && let Some(environment) = &profile.environment
        && verify_zone(tzdir, &environment.country, &environment.timezone).is_err()
    {
        errors.push(Violation::ZoneInvalid);
    }
    for argument in &profile.extra_args {
        if let Some(flag) = forbidden_argument(argument) {
            errors.push(Violation::ForbiddenArgument(flag.to_string()));
        }
    }
    if engine.brand != profile.engine.brand || engine.major != profile.engine.major {
        warnings.push(Violation::EngineChanged);
    }
    if verified::status(engine, profile.strategy) == Verification::Unverified {
        warnings.push(Violation::UnverifiedVersion);
    }
    if profile.strategy == Strategy::Local
        && profile
            .environment
            .as_ref()
            .is_some_and(|environment| !environment.matches_country)
    {
        warnings.push(Violation::LanguageNotRegional);
    }
    if profile.strategy == Strategy::Local && engine.brand == Brand::Brave {
        warnings.push(Violation::BraveFarblesLanguages);
    }
    if profile.bypass_suspected {
        errors.push(Violation::BypassSuspected);
    }
    Report { errors, warnings }
}

fn forbidden_argument(argument: &str) -> Option<&'static str> {
    if let Some(exact) = FORBIDDEN_EXACT
        .iter()
        .copied()
        .find(|flag| argument == *flag)
    {
        return Some(exact);
    }
    FORBIDDEN_ARGUMENTS
        .iter()
        .copied()
        .filter(|prefix| argument.starts_with(prefix))
        .max_by_key(|prefix| prefix.len())
}
