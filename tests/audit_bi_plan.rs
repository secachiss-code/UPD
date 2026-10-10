//! BI.L1–BI.L2: чистые планы запуска Chromium (E09) и Gecko с user.js (E10).

#[path = "bi_support/mod.rs"]
mod support;

use cm::identity::model::{BrowserIdentityProfile, Strategy};
use cm::identity::plan::{LaunchPlan, chromium_plan, gecko_plan};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use support::{Checks, chrome155, de_env, librewolf157, parent_env, profile};

const PROFILE: &str = "/r/work/profile";
const BROWSER: &str = "/usr/bin/google-chrome-stable";

fn strs(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| s.to_string()).collect()
}

fn env_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn local_de() -> BrowserIdentityProfile {
    profile(
        "work",
        Strategy::Local,
        Path::new(BROWSER),
        chrome155(),
        Some(de_env()),
    )
}

fn crowd_firefox() -> BrowserIdentityProfile {
    profile(
        "crowd",
        Strategy::Crowd,
        Path::new("/usr/bin/librewolf"),
        librewolf157(),
        None,
    )
}

const LOCAL_USER_JS: &str = "// Managed by cm identity. Changes here are overwritten at launch.\nuser_pref(\"intl.accept_languages\", \"de-DE, de, en-US, en\");\nuser_pref(\"intl.locale.requested\", \"de-DE\");\nuser_pref(\"privacy.resistFingerprinting\", false);\n";

const CROWD_USER_JS: &str = "// Managed by cm identity. Changes here are overwritten at launch.\nuser_pref(\"intl.accept_languages\", \"en-US, en\");\nuser_pref(\"intl.locale.requested\", \"en-US\");\nuser_pref(\"privacy.resistFingerprinting\", true);\n";

#[test]
fn e09_chromium_plan_passes_only_what_validator_allows() {
    let mut c = Checks::new("E09");
    let profile = local_de();
    let plan: LaunchPlan = chromium_plan(&profile, Path::new(PROFILE), &parent_env(), None, false);

    c.eq(
        "argv Chromium DE local",
        strs(&[
            "--user-data-dir=/r/work/profile",
            "--lang=de-DE",
            "--accept-lang=de-DE,de,en-US,en",
            "--no-first-run",
            "--no-default-browser-check",
        ]),
        plan.args.clone(),
    );
    c.eq(
        "env Chromium DE local — ровно 9 ключей",
        env_map(&[
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
            ("DISPLAY", ":0"),
            ("HOME", "/h"),
            ("LANG", "de_DE.UTF-8"),
            ("PATH", "/usr/bin"),
            ("TZ", "Europe/Berlin"),
            ("WAYLAND_DISPLAY", "wayland-1"),
            ("XAUTHORITY", "/h/.Xauthority"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ]),
        plan.env.clone(),
    );

    let mut without_path = parent_env();
    without_path.remove("PATH");
    let no_path = chromium_plan(&profile, Path::new(PROFILE), &without_path, None, false);
    c.eq(
        "PATH по умолчанию",
        Some(&"/usr/bin:/bin".to_string()),
        no_path.env.get("PATH"),
    );

    let with_url = chromium_plan(
        &profile,
        Path::new(PROFILE),
        &parent_env(),
        Some("http://127.0.0.1:18765/"),
        false,
    );
    c.eq(
        "url последним аргументом",
        Some("http://127.0.0.1:18765/".to_string()),
        with_url.args.last().cloned(),
    );

    let mut with_flag = profile.clone();
    with_flag.extra_args = strs(&["--ozone-platform=wayland"]);
    let flagged = chromium_plan(
        &with_flag,
        Path::new(PROFILE),
        &parent_env(),
        Some("http://127.0.0.1:18765/"),
        false,
    );
    let n = flagged.args.len();
    c.eq(
        "extra перед url",
        Some("--ozone-platform=wayland".to_string()),
        flagged.args.get(n.saturating_sub(2)).cloned(),
    );

    let lab = chromium_plan(&profile, Path::new(PROFILE), &parent_env(), None, true);
    let anchor = lab
        .args
        .iter()
        .position(|a| a == "--no-default-browser-check")
        .expect("флаг присутствует");
    c.eq(
        "lab: флаги после --no-default-browser-check",
        strs(&["--headless=new", "--no-sandbox", "--disable-popup-blocking"]),
        lab.args[anchor + 1..].to_vec(),
    );
    c.holds(
        "без lab блокировщик окон не отключается",
        !plan.args.iter().any(|a| a == "--disable-popup-blocking"),
    );

    c.eq(
        "program — путь личности",
        PathBuf::from(BROWSER),
        plan.program.clone(),
    );
    let forbidden = ["--user-agent", "--remote-debugging-", "--enable-automation"];
    let leaked: Vec<String> = plan
        .args
        .iter()
        .filter(|a| forbidden.iter().any(|f| a.starts_with(f)))
        .cloned()
        .collect();
    c.eq(
        "argv без UA, debugging и automation",
        Vec::<String>::new(),
        leaked,
    );
    c.eq("files пусты для Chromium", 0, plan.files.len());
    c.finish();
}

#[test]
fn e10_gecko_user_js_is_fully_strategy_defined() {
    let mut c = Checks::new("E10");
    let penv = parent_env();

    let local = gecko_plan(&local_de(), Path::new(PROFILE), &penv, None, false);
    c.eq(
        "Local DE: единственный файл user.js",
        vec![PathBuf::from("/r/work/profile/user.js")],
        local.files.iter().map(|f| f.path.clone()).collect(),
    );
    c.eq(
        "Local DE: user.js",
        LOCAL_USER_JS.to_string(),
        local.files[0].contents.clone(),
    );
    c.eq(
        "Local DE: env TZ и LANG",
        Some(("Europe/Berlin".to_string(), "de_DE.UTF-8".to_string())),
        Some((
            local.env.get("TZ").cloned().unwrap_or_default(),
            local.env.get("LANG").cloned().unwrap_or_default(),
        )),
    );
    c.eq(
        "Local DE: args",
        strs(&["--profile", "/r/work/profile", "--no-remote"]),
        local.args.clone(),
    );
    let local_lab = gecko_plan(&local_de(), Path::new(PROFILE), &penv, None, true);
    c.eq(
        "Local DE lab: args + --headless",
        strs(&["--profile", "/r/work/profile", "--no-remote", "--headless"]),
        local_lab.args.clone(),
    );

    c.eq(
        "Local DE lab: user.js разрешает окно лаборатории",
        format!("{LOCAL_USER_JS}user_pref(\"dom.disable_open_during_load\", false);\n"),
        local_lab.files[0].contents.clone(),
    );
    c.holds(
        "без lab user.js не трогает блокировщик окон",
        !local.files[0]
            .contents
            .contains("dom.disable_open_during_load"),
    );

    let crowd_profile = crowd_firefox();
    let crowd = gecko_plan(&crowd_profile, Path::new(PROFILE), &penv, None, false);
    c.eq(
        "Crowd: user.js",
        CROWD_USER_JS.to_string(),
        crowd.files[0].contents.clone(),
    );
    c.eq(
        "Crowd: LANG en_US.UTF-8",
        Some("en_US.UTF-8".to_string()),
        crowd.env.get("LANG").cloned(),
    );
    c.holds("Crowd: нет TZ", !crowd.env.contains_key("TZ"));

    let mut sample = Vec::new();
    sample.extend(local.files.iter().map(|f| f.contents.clone()));
    sample.extend(crowd.files.iter().map(|f| f.contents.clone()));
    for contents in &sample {
        c.holds(
            "user.js не задаёт useragent",
            !contents.contains("useragent"),
        );
    }
    c.finish();
}
