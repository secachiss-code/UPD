//! Чистые планы запуска Chromium и Gecko.

use super::model::{BrowserIdentityProfile, Strategy};
use super::presets::EnvironmentProfile;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MANAGED_HEADER: &str = "// Managed by cm identity. Changes here are overwritten at launch.\n";

const PARENT_ENV_KEYS: &[&str] = &[
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "HOME",
    "PATH",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "PULSE_SERVER",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileWrite {
    pub path: PathBuf,
    pub contents: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchPlan {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub files: Vec<FileWrite>,
}

pub fn chromium_plan(
    profile: &BrowserIdentityProfile,
    profile_dir: &Path,
    parent_env: &BTreeMap<String, String>,
    url: Option<&str>,
    lab: bool,
) -> LaunchPlan {
    let (primary, accept, timezone, lang) = match &profile.environment {
        Some(environment) => (
            environment.primary().to_string(),
            environment.accept_language(),
            Some(environment.timezone.clone()),
            environment.posix_locale.clone(),
        ),
        None => (String::new(), String::new(), None, String::new()),
    };
    let mut args = vec![
        format!("--user-data-dir={}", path_text(profile_dir)),
        format!("--lang={primary}"),
        format!("--accept-lang={accept}"),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
    ];
    if lab {
        args.push("--headless=new".to_string());
        args.push("--no-sandbox".to_string());
        // The lab page opens its tab by script, without a user gesture.
        args.push("--disable-popup-blocking".to_string());
    }
    args.extend(profile.extra_args.iter().cloned());
    if let Some(url) = url {
        args.push(url.to_string());
    }
    LaunchPlan {
        program: profile.browser.clone(),
        args,
        env: plan_env(parent_env, timezone.as_deref(), &lang),
        files: Vec::new(),
    }
}

pub fn gecko_plan(
    profile: &BrowserIdentityProfile,
    profile_dir: &Path,
    parent_env: &BTreeMap<String, String>,
    url: Option<&str>,
    lab: bool,
) -> LaunchPlan {
    let mut args = vec![
        "--profile".to_string(),
        path_text(profile_dir),
        "--no-remote".to_string(),
    ];
    if lab {
        args.push("--headless".to_string());
    }
    args.extend(profile.extra_args.iter().cloned());
    if let Some(url) = url {
        args.push(url.to_string());
    }
    let (env, mut contents) = match profile.strategy {
        Strategy::Crowd => (plan_env(parent_env, None, "en_US.UTF-8"), crowd_user_js()),
        Strategy::Local => match &profile.environment {
            Some(environment) => (
                plan_env(
                    parent_env,
                    Some(&environment.timezone),
                    &environment.posix_locale,
                ),
                local_user_js(environment),
            ),
            None => (plan_env(parent_env, None, ""), local_user_js_parts("", "")),
        },
    };
    if lab {
        // The lab page opens its tab by script, without a user gesture.
        contents.push_str("user_pref(\"dom.disable_open_during_load\", false);\n");
    }
    LaunchPlan {
        program: profile.browser.clone(),
        args,
        env,
        files: vec![FileWrite {
            path: profile_dir.join("user.js"),
            contents,
        }],
    }
}

fn plan_env(
    parent: &BTreeMap<String, String>,
    timezone: Option<&str>,
    lang: &str,
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for key in PARENT_ENV_KEYS {
        if let Some(value) = parent.get(*key) {
            env.insert((*key).to_string(), value.clone());
        }
    }
    env.entry("PATH".to_string())
        .or_insert_with(|| "/usr/bin:/bin".to_string());
    if let Some(timezone) = timezone {
        env.insert("TZ".to_string(), timezone.to_string());
    }
    env.insert("LANG".to_string(), lang.to_string());
    env
}

fn local_user_js(environment: &EnvironmentProfile) -> String {
    local_user_js_parts(&environment.languages.join(", "), environment.primary())
}

fn local_user_js_parts(languages: &str, primary: &str) -> String {
    format!(
        "{MANAGED_HEADER}user_pref(\"intl.accept_languages\", \"{languages}\");\nuser_pref(\"intl.locale.requested\", \"{primary}\");\nuser_pref(\"privacy.resistFingerprinting\", false);\n"
    )
}

fn crowd_user_js() -> String {
    format!(
        "{MANAGED_HEADER}user_pref(\"intl.accept_languages\", \"en-US, en\");\nuser_pref(\"intl.locale.requested\", \"en-US\");\nuser_pref(\"privacy.resistFingerprinting\", true);\n"
    )
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
