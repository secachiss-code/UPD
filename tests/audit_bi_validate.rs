//! BI.M3: валидатор личности — каждое правило таблицы срабатывает ровно на своём условии (E08).

#[path = "bi_support/mod.rs"]
mod support;

use cm::identity::engine::EngineInfo;
use cm::identity::model::{BrowserIdentityProfile, Strategy};
use cm::identity::presets::EnvironmentProfile;
use cm::identity::validate::{Violation, validate};
use std::path::Path;
use support::{Checks, ZONEINFO, brave154, chrome155, chrome156, de_env, librewolf157, profile};

const BROWSER: &str = "/usr/bin/google-chrome-stable";

struct Row {
    name: &'static str,
    profile: BrowserIdentityProfile,
    engine: EngineInfo,
    errors: Vec<Violation>,
    /// `None` — спецификация не фиксирует предупреждения этой строки; проверяются только ошибки.
    warnings: Option<Vec<Violation>>,
}

fn english_env() -> EnvironmentProfile {
    EnvironmentProfile {
        matches_country: false,
        languages: vec!["en-US".into(), "en".into()],
        posix_locale: "en_US.UTF-8".into(),
        ..de_env()
    }
}

fn with_args(mut p: BrowserIdentityProfile, args: &[&str]) -> BrowserIdentityProfile {
    p.extra_args = args.iter().map(|s| s.to_string()).collect();
    p
}

fn rows() -> Vec<Row> {
    let browser = std::path::Path::new(BROWSER);
    let local_de = || {
        profile(
            "work",
            Strategy::Local,
            browser,
            chrome155(),
            Some(de_env()),
        )
    };
    let forbidden = |args: &[&str], flag: &str| Row {
        name: "запрещённый аргумент",
        profile: with_args(local_de(), args),
        engine: chrome155(),
        errors: vec![Violation::ForbiddenArgument(flag.to_string())],
        warnings: Some(vec![]),
    };
    let mut bypass = local_de();
    bypass.bypass_suspected = true;
    let mut zone_bad = local_de();
    if let Some(env) = zone_bad.environment.as_mut() {
        env.timezone = "Europe/Amsterdam".into();
    }
    vec![
        Row {
            name: "Local DE Chrome 155 — совпадают",
            profile: local_de(),
            engine: chrome155(),
            errors: vec![],
            warnings: Some(vec![]),
        },
        Row {
            name: "Crowd Chrome 155 без среды",
            profile: profile("c", Strategy::Crowd, browser, chrome155(), None),
            engine: chrome155(),
            errors: vec![Violation::StrategyEngineUnsupported],
            warnings: None,
        },
        Row {
            name: "Crowd LibreWolf 157 с DE",
            profile: profile(
                "c",
                Strategy::Crowd,
                browser,
                librewolf157(),
                Some(de_env()),
            ),
            engine: librewolf157(),
            errors: vec![Violation::CrowdWithRegion],
            warnings: Some(vec![]),
        },
        Row {
            name: "Local без среды",
            profile: profile("l", Strategy::Local, browser, chrome155(), None),
            engine: chrome155(),
            errors: vec![Violation::LocalNeedsEnvironment],
            warnings: None,
        },
        Row {
            name: "Local DE, зона Europe/Amsterdam подложена",
            profile: zone_bad,
            engine: chrome155(),
            errors: vec![Violation::ZoneInvalid],
            warnings: None,
        },
        forbidden(&["--user-agent=x"], "--user-agent"),
        forbidden(&["--remote-debugging-pipe"], "--remote-debugging-pipe"),
        forbidden(&["--headless=new"], "--headless"),
        forbidden(&["--lang=ru"], "--lang"),
        forbidden(&["--profile"], "--profile"),
        forbidden(
            &["--time-zone-for-testing=Asia/Tokyo"],
            "--time-zone-for-testing",
        ),
        forbidden(&["--profile-directory=Other"], "--profile-directory"),
        forbidden(&["-P"], "-P"),
        forbidden(&["--ProfileManager"], "--ProfileManager"),
        Row {
            name: "разрешённый -private-window (не -P)",
            profile: with_args(local_de(), &["-private-window"]),
            engine: chrome155(),
            errors: vec![],
            warnings: Some(vec![]),
        },
        Row {
            name: "разрешённый --ozone-platform",
            profile: with_args(local_de(), &["--ozone-platform=wayland"]),
            engine: chrome155(),
            errors: vec![],
            warnings: Some(vec![]),
        },
        Row {
            name: "профиль Chrome 155, движок Chrome 156",
            profile: local_de(),
            engine: chrome156(),
            errors: vec![],
            warnings: Some(vec![Violation::EngineChanged, Violation::UnverifiedVersion]),
        },
        Row {
            name: "Local DE English",
            profile: profile(
                "e",
                Strategy::Local,
                browser,
                chrome155(),
                Some(english_env()),
            ),
            engine: chrome155(),
            errors: vec![],
            warnings: Some(vec![Violation::LanguageNotRegional]),
        },
        Row {
            name: "Local DE Brave 154",
            profile: profile("b", Strategy::Local, browser, brave154(), Some(de_env())),
            engine: brave154(),
            errors: vec![],
            warnings: Some(vec![Violation::BraveFarblesLanguages]),
        },
        Row {
            name: "bypass_suspected",
            profile: bypass,
            engine: chrome155(),
            errors: vec![Violation::BypassSuspected],
            warnings: None,
        },
    ]
}

#[test]
fn e08_each_rule_fires_on_its_own_condition() {
    let mut c = Checks::new("E08");
    for row in rows() {
        let report = validate(&row.profile, &row.engine, Path::new(ZONEINFO), false);
        c.eq(
            &format!("{}: errors", row.name),
            row.errors.clone(),
            report.errors.clone(),
        );
        if let Some(expected) = row.warnings {
            c.eq(
                &format!("{}: warnings", row.name),
                expected,
                report.warnings.clone(),
            );
        }
    }
    c.finish();
}
