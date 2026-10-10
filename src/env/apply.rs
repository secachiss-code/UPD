//! Проверка окружения региона. Система не меняется.

use std::path::Path;

use crate::identity::tzdata::valid_zone_name;
use crate::profiles::EnvironmentPreset;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvIssue {
    ZoneMissing,
    LocaleNotInstalled,
    LanguagesEmpty,
    LocaleLanguageMismatch,
}

pub fn check_preset(
    preset: &EnvironmentPreset,
    tzdir: &Path,
    installed_locales: &[String],
) -> Vec<EnvIssue> {
    let mut issues = Vec::new();
    if !valid_zone_name(&preset.timezone) || !tzdir.join(&preset.timezone).is_file() {
        issues.push(EnvIssue::ZoneMissing);
    }
    let wanted = normalize_locale(&preset.locale);
    if !installed_locales
        .iter()
        .any(|locale| normalize_locale(locale) == wanted)
    {
        issues.push(EnvIssue::LocaleNotInstalled);
    }
    if preset.languages.is_empty() {
        issues.push(EnvIssue::LanguagesEmpty);
    } else {
        let locale_lang = preset.locale.split('_').next().unwrap_or("");
        let first_lang = preset.languages[0].split('-').next().unwrap_or("");
        if locale_lang != first_lang {
            issues.push(EnvIssue::LocaleLanguageMismatch);
        }
    }
    issues
}

pub fn parse_locale_list(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn normalize_locale(name: &str) -> String {
    let mut text = name.replace('-', "_");
    if let Some(suffix) = text
        .strip_suffix(".UTF-8")
        .or_else(|| text.strip_suffix(".utf-8"))
        .or_else(|| text.strip_suffix(".UTF8"))
    {
        text = format!("{suffix}.utf8");
    }
    text
}
