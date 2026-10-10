//! BI.R1–BI.R3: пресеты стран (E01), выбор окружения (E02), проверка зоны по tzdata (E03).
//! Значения — из `docs/design/cm-network-manager/BI-DAG.md`, а не из текущего кода.

#[path = "bi_support/mod.rs"]
mod support;

use cm::identity::presets::{EnvironmentProfile, LanguageChoice, PRESETS, PresetError, select};
use cm::identity::tzdata::{ZoneError, verify_zone};
use std::fs;
use std::path::{Path, PathBuf};
use support::{Checks, ZONEINFO};

fn zone_tab_lines() -> Vec<String> {
    fs::read_to_string(format!("{ZONEINFO}/zone.tab"))
        .expect("системная tzdata: zone.tab")
        .lines()
        .map(str::to_string)
        .collect()
}

/// `ll` или `ll-RR` (BCP 47 в объёме пресетов).
fn bcp47_ok(tag: &str) -> bool {
    let (lang, region) = match tag.split_once('-') {
        Some((lang, region)) => (lang, Some(region)),
        None => (tag, None),
    };
    let lang_ok = (2..=3).contains(&lang.len()) && lang.bytes().all(|b| b.is_ascii_lowercase());
    let region_ok =
        region.is_none_or(|r| r.len() == 2 && r.bytes().all(|b| b.is_ascii_uppercase()));
    lang_ok && region_ok
}

#[test]
fn e01_presets_are_valid_for_selection() {
    let mut c = Checks::new("E01");

    let mut countries: Vec<&str> = PRESETS.iter().map(|p| p.country).collect();
    countries.sort_unstable();
    c.eq(
        "набор стран",
        vec![
            "AT", "CA", "CH", "DE", "EE", "FI", "FR", "GB", "JP", "LT", "LV", "NL", "PL", "RU",
            "SE", "SG", "TR", "US",
        ],
        countries,
    );

    let table = zone_tab_lines();
    for preset in PRESETS {
        for zone in preset.zones {
            let listed = table.iter().any(|line| {
                if line.starts_with('#') {
                    return false;
                }
                let columns: Vec<&str> = line.split('\t').collect();
                columns.first() == Some(&preset.country) && columns.get(2) == Some(zone)
            });
            c.holds(
                &format!("{} {zone}: строка в zone.tab", preset.country),
                listed,
            );
            c.holds(
                &format!("{zone}: файл зоны существует"),
                Path::new(ZONEINFO).join(zone).is_file(),
            );
        }
    }

    let zones_of = |country: &str| -> Vec<&str> {
        PRESETS
            .iter()
            .find(|p| p.country == country)
            .map(|p| p.zones.to_vec())
            .unwrap_or_default()
    };
    c.eq("NL.zones", vec!["Europe/Amsterdam"], zones_of("NL"));
    c.eq("SE.zones", vec!["Europe/Stockholm"], zones_of("SE"));

    for preset in PRESETS {
        for set in preset.language_sets {
            let first = set.tags[0];
            let (ll, rr) = first.split_once('-').unwrap_or((first, ""));
            c.holds(
                &format!("{}: tags[0] в форме ll-RR: {first}", preset.country),
                rr.len() == 2 && ll.len() == 2,
            );
            c.eq(
                &format!("{}: posix для {first}", preset.country),
                format!("{ll}_{rr}.UTF-8"),
                set.posix.to_string(),
            );
            for tag in set.tags {
                c.holds(
                    &format!("{}: BCP 47 тег {tag}", preset.country),
                    bcp47_ok(tag),
                );
            }
        }
    }

    let de = PRESETS.iter().find(|p| p.country == "DE").expect("DE");
    c.eq(
        "DE.language_sets[0].tags",
        vec!["de-DE", "de", "en-US", "en"],
        de.language_sets[0].tags.to_vec(),
    );
    c.finish();
}

fn env(
    country: &str,
    timezone: &str,
    languages: &[&str],
    posix: &str,
    matches: bool,
) -> EnvironmentProfile {
    EnvironmentProfile {
        country: country.to_string(),
        timezone: timezone.to_string(),
        languages: languages.iter().map(|s| s.to_string()).collect(),
        posix_locale: posix.to_string(),
        matches_country: matches,
    }
}

#[test]
fn e02_select_is_deterministic_and_errors_are_plain() {
    let mut c = Checks::new("E02");

    let de = env(
        "DE",
        "Europe/Berlin",
        &["de-DE", "de", "en-US", "en"],
        "de_DE.UTF-8",
        true,
    );
    let got = select("DE", None, &LanguageChoice::Local(0));
    c.eq("select(DE, None, Local(0))", Ok(de.clone()), got.clone());
    c.eq(
        "accept_language DE",
        "de-DE,de,en-US,en".to_string(),
        got.as_ref()
            .map(|e| e.accept_language())
            .unwrap_or_default(),
    );
    c.eq(
        "select(de) == select(DE)",
        got.clone(),
        select("de", None, &LanguageChoice::Local(0)),
    );

    c.eq(
        "select(Germany)",
        Err(PresetError::UnknownCountry),
        select("Germany", None, &LanguageChoice::Local(0)).map(|_| ()),
    );

    c.eq(
        "US Chicago English",
        Ok(env(
            "US",
            "America/Chicago",
            &["en-US", "en"],
            "en_US.UTF-8",
            true,
        )),
        select("US", Some("America/Chicago"), &LanguageChoice::English),
    );
    c.eq(
        "US Berlin (чужая зона)",
        Err(PresetError::ZoneNotInCountry),
        select("US", Some("Europe/Berlin"), &LanguageChoice::English).map(|_| ()),
    );

    c.eq(
        "CH Local(1)",
        Ok(env(
            "CH",
            "Europe/Zurich",
            &["fr-CH", "fr", "en-US", "en"],
            "fr_CH.UTF-8",
            true,
        )),
        select("CH", None, &LanguageChoice::Local(1)),
    );
    c.eq(
        "CH Local(2)",
        Err(PresetError::NoSuchLanguageSet),
        select("CH", None, &LanguageChoice::Local(2)).map(|_| ()),
    );

    c.eq(
        "DE English",
        Ok(env(
            "DE",
            "Europe/Berlin",
            &["en-US", "en"],
            "en_US.UTF-8",
            false,
        )),
        select("DE", None, &LanguageChoice::English),
    );

    c.eq(
        "JP Custom(ru-RU)",
        Ok(env(
            "JP",
            "Asia/Tokyo",
            &["ru-RU", "ru"],
            "ru_RU.UTF-8",
            false,
        )),
        select("JP", None, &LanguageChoice::Custom("ru-RU".into())),
    );
    c.eq(
        "JP Custom(ru)",
        Ok(env("JP", "Asia/Tokyo", &["ru"], "ru_RU.UTF-8", false)),
        select("JP", None, &LanguageChoice::Custom("ru".into())),
    );
    for bad in ["ru_RU", "RU-ru", ""] {
        c.eq(
            &format!("JP Custom({bad:?})"),
            Err(PresetError::BadLanguageTag),
            select("JP", None, &LanguageChoice::Custom(bad.into())).map(|_| ()),
        );
    }

    let errors = [
        select("Germany", None, &LanguageChoice::Local(0)).err(),
        select("US", Some("Europe/Berlin"), &LanguageChoice::English).err(),
        select("JP", None, &LanguageChoice::Custom("ru_RU".into())).err(),
    ];
    for (input, error) in ["Germany", "Europe/Berlin", "ru_RU"].iter().zip(errors) {
        let text = error.map(|e| e.to_string()).unwrap_or_default();
        c.holds(
            &format!("текст ошибки не содержит ввод {input:?}"),
            !text.contains(input),
        );
    }
    c.finish();
}

fn check_zone(tzdir: &Path, country: &str, zone: &str) -> Result<(), ZoneError> {
    verify_zone(tzdir, country, zone)
}

#[test]
fn e03_verify_zone_uses_only_tzdata() {
    let mut c = Checks::new("E03");
    let system = Path::new(ZONEINFO);

    c.eq(
        "системная DE Europe/Berlin",
        Ok(()),
        check_zone(system, "DE", "Europe/Berlin"),
    );
    c.eq(
        "системная DE Europe/Amsterdam",
        Err(ZoneError::ZoneNotInCountry),
        check_zone(system, "DE", "Europe/Amsterdam"),
    );

    let dir = cm::common::contract_fixtures::TempDirGuard::new("cm-bi-e03").expect("tmp");
    let fixture: PathBuf = dir.path().to_path_buf();
    fs::write(
        fixture.join("zone.tab"),
        "NL\t+5222+00454\tEurope/Nowhere\n",
    )
    .unwrap();
    c.eq(
        "фикстура: строка есть, файла зоны нет",
        Err(ZoneError::ZoneMissing),
        check_zone(&fixture, "NL", "Europe/Nowhere"),
    );

    // Зона есть в системной tzdata, но не в фикстуре: проверка не выходит за свой tzdir.
    fs::write(fixture.join("zone.tab"), "DE\t+5230+01322\tEurope/Berlin\n").unwrap();
    c.eq(
        "фикстура: системный файл не используется",
        Err(ZoneError::ZoneMissing),
        check_zone(&fixture, "DE", "Europe/Berlin"),
    );

    let empty = cm::common::contract_fixtures::TempDirGuard::new("cm-bi-e03-empty").expect("tmp");
    c.eq(
        "без zone.tab",
        Err(ZoneError::TzdataUnavailable),
        check_zone(empty.path(), "DE", "Europe/Berlin"),
    );

    let long = "A".repeat(65);
    for bad in [
        "../../etc/passwd",
        "/Europe/Berlin",
        "Europe/Ber lin",
        long.as_str(),
    ] {
        c.eq(
            &format!("имя зоны {:?}", &bad[..bad.len().min(24)]),
            Err(ZoneError::BadZoneName),
            check_zone(system, "DE", bad),
        );
    }
    c.finish();
}
