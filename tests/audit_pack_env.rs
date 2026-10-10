//! M09–M12: exit country from fresh agreeing sources, preset checks, region invalidation.

use std::fs;

use cm::common::contract_fixtures::TempDirGuard;
use cm::env::{
    EnvIssue, Invalidation, MAX_AGE_MS, MIN_SOURCES, Observation, RegionDecision, axis_after,
    check_preset, decide, normalize_locale, on_region_change, parse_locale_list, region_axis,
};
use cm::profiles::{EnvironmentPreset, Id, SCHEMA_VERSION, VerificationValue};
use serde_json::Value;

fn decision(value: &Value) -> RegionDecision {
    if value == "stale" {
        return RegionDecision::Stale;
    }
    if let Some(agreed) = value.get("agreed") {
        return RegionDecision::Agreed {
            country: agreed[0].as_str().unwrap().to_owned(),
            sources: agreed[1].as_u64().unwrap() as usize,
        };
    }
    if let Some(countries) = value.get("disagree") {
        return RegionDecision::Disagree {
            countries: countries
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item.as_str().unwrap().to_owned())
                .collect(),
        };
    }
    RegionDecision::Insufficient {
        fresh: value["insufficient"].as_u64().unwrap() as usize,
    }
}

#[test]
fn m09_country_needs_fresh_agreeing_sources() {
    assert_eq!((MIN_SOURCES, MAX_AGE_MS), (2, 15 * 60 * 1000));
    let fixture: Value = serde_json::from_str(include_str!("fixtures/pack/region.json")).unwrap();
    let now = fixture["now_unix_ms"].as_i64().unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 11);
    for case in cases {
        let observations: Vec<Observation> = case["observations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| Observation {
                source: item[0].as_str().unwrap().to_owned(),
                country: item[1].as_str().unwrap().to_owned(),
                at_unix_ms: now - item[2].as_i64().unwrap() * 1000,
            })
            .collect();
        assert_eq!(
            decide(&observations, now),
            decision(&case["decision"]),
            "{}",
            case["name"]
        );
        // The order of observations does not change the answer.
        let reversed: Vec<Observation> = observations.into_iter().rev().collect();
        assert_eq!(
            decide(&reversed, now),
            decision(&case["decision"]),
            "{}",
            case["name"]
        );
    }
}

fn agreed(country: &str) -> RegionDecision {
    RegionDecision::Agreed {
        country: country.to_owned(),
        sources: 2,
    }
}

fn undecided() -> [RegionDecision; 3] {
    [
        RegionDecision::Disagree {
            countries: vec!["DE".to_owned(), "NL".to_owned()],
        },
        RegionDecision::Insufficient { fresh: 1 },
        RegionDecision::Stale,
    ]
}

#[test]
fn m09_m11_region_axis_and_invalidation() {
    assert_eq!(
        region_axis(&agreed("DE"), "DE"),
        VerificationValue::Verified
    );
    assert_eq!(region_axis(&agreed("DE"), "NL"), VerificationValue::Blocked);
    let [disagree, insufficient, stale] = undecided();
    assert_eq!(region_axis(&disagree, "DE"), VerificationValue::Partial);
    assert_eq!(region_axis(&insufficient, "DE"), VerificationValue::Unknown);
    assert_eq!(region_axis(&stale, "DE"), VerificationValue::Unknown);

    let keep = on_region_change("DE", &agreed("NL"), &agreed("DE"));
    assert_eq!(keep, Invalidation::Keep);
    assert_eq!(axis_after(&keep), VerificationValue::Verified);
    let mismatch = on_region_change("DE", &agreed("DE"), &agreed("NL"));
    assert_eq!(
        mismatch,
        Invalidation::RegionMismatch {
            expected: "DE".to_owned(),
            observed: "NL".to_owned()
        }
    );
    assert_eq!(axis_after(&mismatch), VerificationValue::Blocked);
    // An earlier confirmation is never carried over.
    for current in undecided() {
        let unknown = on_region_change("DE", &agreed("DE"), &current);
        assert_eq!(unknown, Invalidation::RegionUnknown, "{current:?}");
        assert_eq!(axis_after(&unknown), VerificationValue::Unknown);
    }
}

fn preset(timezone: &str, locale: &str, languages: &[&str]) -> EnvironmentPreset {
    EnvironmentPreset {
        schema_version: SCHEMA_VERSION,
        id: Id::new("preset-1").unwrap(),
        timezone: timezone.to_owned(),
        locale: locale.to_owned(),
        languages: languages.iter().map(|item| (*item).to_owned()).collect(),
    }
}

#[test]
fn m10_preset_check_names_every_problem() {
    let dir = TempDirGuard::new("cm-pack-m10").unwrap();
    let tzdir = dir.path();
    fs::create_dir_all(tzdir.join("Europe")).unwrap();
    fs::write(tzdir.join("Europe/Berlin"), b"TZif").unwrap();
    let before: Vec<_> = fs::read_dir(tzdir.join("Europe")).unwrap().collect();
    let locales = vec!["de_DE.utf8".to_owned(), "en_US.utf8".to_owned()];
    let good = preset("Europe/Berlin", "de_DE.UTF-8", &["de-DE", "de"]);
    assert_eq!(check_preset(&good, tzdir, &locales), []);
    assert_eq!(
        check_preset(
            &preset("Europe/Paris", "de_DE.UTF-8", &["de"]),
            tzdir,
            &locales
        ),
        [EnvIssue::ZoneMissing]
    );
    fs::write(dir.path().join("passwd"), b"x").unwrap();
    for zone in ["../etc/passwd", "/etc/passwd", "Europe/../passwd", ""] {
        assert_eq!(
            check_preset(&preset(zone, "de_DE.UTF-8", &["de"]), tzdir, &locales),
            [EnvIssue::ZoneMissing],
            "{zone}"
        );
    }
    assert_eq!(
        check_preset(&good, tzdir, &["en_US.utf8".to_owned()]),
        [EnvIssue::LocaleNotInstalled]
    );
    assert_eq!(
        check_preset(
            &preset("Europe/Berlin", "de_DE.UTF-8", &[]),
            tzdir,
            &locales
        ),
        [EnvIssue::LanguagesEmpty]
    );
    assert_eq!(
        check_preset(
            &preset("Europe/Berlin", "de_DE.UTF-8", &["fr-FR"]),
            tzdir,
            &locales
        ),
        [EnvIssue::LocaleLanguageMismatch]
    );
    assert_eq!(
        check_preset(&preset("Nowhere/Zone", "xx_XX.UTF-8", &[]), tzdir, &locales),
        [
            EnvIssue::ZoneMissing,
            EnvIssue::LocaleNotInstalled,
            EnvIssue::LanguagesEmpty
        ]
    );
    assert_eq!(
        parse_locale_list("C\nC.utf8\nde_DE.utf8\n"),
        ["C", "C.utf8", "de_DE.utf8"]
    );
    assert_eq!(normalize_locale("de_DE.UTF-8"), "de_DE.utf8");
    assert_eq!(normalize_locale("de_DE.utf8"), "de_DE.utf8");
    assert_eq!(normalize_locale("de-DE.UTF8"), "de_DE.utf8");
    assert_eq!(normalize_locale("sr_RS.UTF-8@latin"), "sr_RS.utf8@latin");
    assert_eq!(normalize_locale("C"), "C");
    assert_eq!(
        fs::read_dir(tzdir.join("Europe")).unwrap().count(),
        before.len()
    );
}

#[test]
fn m12_region_functions_are_pure() {
    for source in [
        include_str!("../src/env/geo.rs"),
        include_str!("../src/env/invalidate.rs"),
    ] {
        for forbidden in ["std::fs", "std::net", "SystemTime", "Command", "std::env"] {
            assert!(!source.contains(forbidden), "{forbidden}");
        }
    }
}
