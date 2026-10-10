//! BI.C1: оси REGION/APP/STATE/NET строятся из `Report` и не выдают Verified без проверки (E13, E17b, E18).

#[path = "bi_support/mod.rs"]
mod support;

use cm::identity::axes::{Axes, AxisValue, axes};
use cm::identity::model::{BrowserIdentityProfile, Strategy};
use cm::identity::presets::EnvironmentProfile;
use cm::identity::validate::{Report, Violation, validate};
use std::path::Path;
use support::{Checks, ZONEINFO, brave154, chrome155, de_env, librewolf157, profile};

const BROWSER: &str = "/usr/bin/google-chrome-stable";

fn local_de() -> BrowserIdentityProfile {
    profile(
        "work",
        Strategy::Local,
        Path::new(BROWSER),
        chrome155(),
        Some(de_env()),
    )
}

fn english_env() -> EnvironmentProfile {
    EnvironmentProfile {
        matches_country: false,
        languages: vec!["en-US".into(), "en".into()],
        posix_locale: "en_US.UTF-8".into(),
        ..de_env()
    }
}

fn axes_of(p: &BrowserIdentityProfile, engine: &cm::identity::engine::EngineInfo) -> Axes {
    let report = validate(p, engine, Path::new(ZONEINFO), false);
    axes(p, &report)
}

#[test]
fn e13_guard_state_reaches_app_axis() {
    let mut c = Checks::new("E13");
    let mut p = local_de();
    p.bypass_suspected = true;
    let got = axes_of(&p, &chrome155());
    c.eq(
        "APP при bypass_suspected",
        AxisValue::Blocked,
        got.app.value,
    );
    c.eq(
        "APP reason при bypass_suspected",
        "профиль открывали мимо CM: нужно cm identity confirm",
        got.app.reason,
    );
    c.finish();
}

#[test]
fn e17b_axes_from_report_without_rechecking() {
    let mut c = Checks::new("E17b");
    let p = local_de();

    let warn_only = Report {
        errors: vec![],
        warnings: vec![Violation::UnverifiedVersion],
    };
    let got = axes(&p, &warn_only);
    c.eq(
        "Report(warnings UnverifiedVersion) → APP",
        AxisValue::Partial,
        got.app.value,
    );

    let forbidden = Report {
        errors: vec![Violation::ForbiddenArgument("--user-agent".into())],
        warnings: vec![],
    };
    let got = axes(&p, &forbidden);
    c.eq(
        "Report(ForbiddenArgument) → APP",
        AxisValue::Blocked,
        got.app.value,
    );
    c.eq(
        "Report(ForbiddenArgument) → APP reason",
        "личность не согласована",
        got.app.reason,
    );
    c.finish();
}

#[test]
fn e18_no_verified_without_check() {
    let mut c = Checks::new("E18");

    let clean = axes_of(&local_de(), &chrome155());
    c.eq("DE Chrome 155: NET", AxisValue::Unknown, clean.net.value);
    c.eq(
        "DE Chrome 155: REGION",
        AxisValue::Partial,
        clean.region.value,
    );
    c.eq(
        "DE Chrome 155: REGION reason",
        "страна задана вручную, выход туннеля не проверен",
        clean.region.reason,
    );
    c.eq(
        "DE Chrome 155: STATE",
        AxisValue::Verified,
        clean.state.value,
    );
    c.eq("DE Chrome 155: APP", AxisValue::Verified, clean.app.value);

    let mut english = local_de();
    english.environment = Some(english_env());
    let got = axes_of(&english, &chrome155());
    c.eq("DE English: REGION", AxisValue::Partial, got.region.value);
    c.eq(
        "DE English: REGION reason",
        "язык не из пресета страны",
        got.region.reason,
    );

    let crowd = profile(
        "crowd",
        Strategy::Crowd,
        Path::new("/usr/bin/librewolf"),
        librewolf157(),
        None,
    );
    let got = axes_of(&crowd, &librewolf157());
    c.eq(
        "Crowd LibreWolf: REGION reason",
        "crowd: часовой пояс UTC и язык en-US намеренно",
        got.region.reason,
    );
    c.eq(
        "Crowd LibreWolf: REGION",
        AxisValue::Partial,
        got.region.value,
    );
    c.eq(
        "Crowd LibreWolf 157: APP",
        AxisValue::Verified,
        got.app.value,
    );
    c.eq(
        "Crowd LibreWolf 157: APP reason",
        "версия проверена лабораторией",
        got.app.reason,
    );
    // Следующий мажор лаборатория ещё не видела: APP честно Partial.
    let next = support::engine(
        cm::identity::engine::Family::Gecko,
        cm::identity::engine::Brand::LibreWolf,
        158,
        "158.0",
    );
    let got = axes_of(&crowd, &next);
    c.eq(
        "Crowd LibreWolf 158: APP",
        AxisValue::Partial,
        got.app.value,
    );
    c.eq(
        "Crowd LibreWolf 158: APP reason",
        "версия браузера не проверена лабораторией",
        got.app.reason,
    );

    let mut zone_bad = local_de();
    if let Some(env) = zone_bad.environment.as_mut() {
        env.timezone = "Europe/Amsterdam".into();
    }
    let got = axes_of(&zone_bad, &chrome155());
    c.eq("ZoneInvalid: STATE", AxisValue::Blocked, got.state.value);
    c.eq("ZoneInvalid: APP", AxisValue::Blocked, got.app.value);

    // Инвариант: REGION и NET никогда не Verified, на всей сетке входов.
    let envs = [None, Some(de_env()), Some(english_env())];
    let engines = [chrome155(), librewolf157(), brave154()];
    let mut grid_rows = 0usize;
    let mut verified_leaks = Vec::new();
    for strategy in [Strategy::Local, Strategy::Crowd] {
        for env in &envs {
            for engine in &engines {
                for bypass in [false, true] {
                    let mut p = profile(
                        "g",
                        strategy,
                        Path::new(BROWSER),
                        engine.clone(),
                        env.clone(),
                    );
                    p.bypass_suspected = bypass;
                    let got = axes_of(&p, engine);
                    grid_rows += 1;
                    if got.region.value == AxisValue::Verified
                        || got.net.value == AxisValue::Verified
                    {
                        verified_leaks.push(format!(
                            "{strategy:?} {:?} {:?} bypass={bypass}",
                            engine.brand,
                            env.is_some()
                        ));
                    }
                }
            }
        }
    }
    c.eq(
        "сетка входов: REGION/NET не Verified",
        Vec::<String>::new(),
        verified_leaks,
    );
    c.holds("сетка входов не пуста", grid_rows > 0);
    c.finish();
}
