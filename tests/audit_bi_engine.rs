//! BI.E1–BI.E2: распознавание браузера по `--version` (E04, E04b) и реестр проверенных версий (E05).

#[path = "bi_support/mod.rs"]
mod support;

use cm::identity::engine::{Brand, EngineError, Family, detect, parse_version};
use cm::identity::model::Strategy;
use cm::identity::verified::{Verification, status};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use support::{
    Checks, brave154, chrome155, chrome156, chromium155, engine, env_guard, firefox157,
    librewolf157, set_env, unset_env, write_exec,
};

const REPO: &str = env!("CARGO_MANIFEST_DIR");

fn registry_rows() -> Vec<serde_json::Value> {
    let text = fs::read_to_string(Path::new(REPO).join("data/identity-verified.json"))
        .expect("data/identity-verified.json");
    serde_json::from_str(&text).expect("реестр — JSON-массив")
}

#[test]
fn e04_engine_describes_the_real_browser() {
    let mut c = Checks::new("E04");
    c.eq(
        "Google Chrome",
        Ok(engine(
            Family::Chromium,
            Brand::Chrome,
            155,
            "155.0.8059.39",
        )),
        parse_version("Google Chrome 155.0.8059.39 \n"),
    );
    c.eq(
        "Chromium Arch",
        Ok(engine(
            Family::Chromium,
            Brand::Chromium,
            155,
            "155.0.7000.1",
        )),
        parse_version("Chromium 155.0.7000.1 Arch Linux"),
    );
    c.eq(
        "Brave Browser",
        Ok(engine(Family::Chromium, Brand::Brave, 154, "154.1.96.61")),
        parse_version("Brave Browser 154.1.96.61"),
    );
    c.eq(
        "Mozilla Firefox",
        Ok(engine(Family::Gecko, Brand::Firefox, 157, "157.0")),
        parse_version("Mozilla Firefox 157.0"),
    );
    c.eq(
        "LibreWolf",
        Ok(engine(Family::Gecko, Brand::LibreWolf, 157, "157.0-1")),
        parse_version("LibreWolf 157.0-1"),
    );
    c.eq(
        "Opera",
        Err(EngineError::UnknownEngine),
        parse_version("Opera 100.0").map(|_| ()),
    );
    c.eq(
        "пустой вывод",
        Err(EngineError::UnknownEngine),
        parse_version("").map(|_| ()),
    );

    let dir = cm::common::contract_fixtures::TempDirGuard::new("cm-bi-e04").expect("tmp");
    c.eq(
        "detect относительный путь",
        Err(EngineError::NotAbsolute),
        detect(Path::new("chrome")).map(|_| ()),
    );

    let exit3 = dir.path().join("exit3");
    write_exec(&exit3, "#!/bin/sh\nexit 3\n");
    c.eq(
        "exit 3",
        Err(EngineError::VersionFailed),
        detect(&exit3).map(|_| ()),
    );

    let sleeper = dir.path().join("sleeper");
    write_exec(&sleeper, "#!/bin/sh\nsleep 30\n");
    let started = Instant::now();
    let timed_out = detect(&sleeper).map(|_| ());
    let elapsed = started.elapsed();
    c.eq("sleep 30 → Timeout", Err(EngineError::Timeout), timed_out);
    c.holds(
        &format!(
            "Timeout за ≤ 12 с (получено {:.1} с)",
            elapsed.as_secs_f64()
        ),
        elapsed <= Duration::from_secs(12),
    );

    // Родительский секрет не должен попасть в окружение браузера; PATH — единственное исключение.
    let _guard = env_guard();
    set_env("BI_E04_PARENT_SECRET", "parent-value");
    let probe = dir.path().join("probe");
    write_exec(
        &probe,
        "#!/bin/sh\nenv > \"$0.env\"\necho 'Google Chrome 155.0.8059.39'\n",
    );
    let got = detect(&probe);
    unset_env("BI_E04_PARENT_SECRET");
    c.eq("probe распознан", Ok(chrome155()), got);
    let env_text = fs::read_to_string(format!("{}.env", probe.display())).unwrap_or_default();
    c.holds(
        "родительский секрет не передан",
        !env_text.contains("BI_E04_PARENT_SECRET"),
    );
    c.holds(
        "HOME родителя не передан",
        !env_text.lines().any(|l| l.starts_with("HOME=")),
    );
    c.holds(
        "PATH == /usr/bin:/bin",
        env_text.lines().any(|l| l == "PATH=/usr/bin:/bin"),
    );
    c.finish();
}

#[test]
fn e04b_registry_keys_on_brand_and_major() {
    let mut c = Checks::new("E04b");
    let a = chrome155();
    let mut b = chrome155();
    b.version = "155.0.9999.1".to_string();
    c.eq(
        "одинаковый status для патч-версий",
        status(&a, Strategy::Local),
        status(&b, Strategy::Local),
    );

    let allowed = ["Chrome", "Chromium", "Brave", "Firefox", "LibreWolf"];
    for row in registry_rows() {
        let brand = row["brand"].as_str().unwrap_or("<нет>");
        c.holds(
            &format!("brand {brand:?} входит в перечень"),
            allowed.contains(&brand),
        );
        c.holds(&format!("major у {brand} — число"), row["major"].is_u64());
        let strategy = row["strategy"].as_str().unwrap_or("<нет>");
        c.holds(
            &format!("strategy {strategy:?} допустима"),
            matches!(strategy, "local" | "crowd"),
        );
    }
    c.finish();
}

#[test]
fn e05_verified_only_with_matching_evidence() {
    let mut c = Checks::new("E05");
    let evidence = "docs/design/cm-network-manager/bi-evidence/2026-10-10";
    c.eq(
        "Chrome 155 local",
        Verification::Verified {
            evidence: evidence.to_string(),
            note: String::new(),
        },
        status(&chrome155(), Strategy::Local),
    );
    match status(&brave154(), Strategy::Local) {
        Verification::Verified { note, .. } => c.holds(
            "Brave 154 local: note содержит farbling",
            note.contains("farbling"),
        ),
        Verification::Unverified => c.holds("Brave 154 local: Verified", false),
    }
    c.eq(
        "Chrome 156 local",
        Verification::Unverified,
        status(&chrome156(), Strategy::Local),
    );
    c.eq(
        "Chromium 155 local",
        Verification::Unverified,
        status(&chromium155(), Strategy::Local),
    );
    c.eq(
        "Firefox 157 local",
        Verification::Unverified,
        status(&firefox157(), Strategy::Local),
    );
    match status(&librewolf157(), Strategy::Crowd) {
        Verification::Verified { note, .. } => c.holds(
            "LibreWolf 157 crowd: note называет RFP",
            note.contains("RFP"),
        ),
        Verification::Unverified => c.holds("LibreWolf 157 crowd: Verified", false),
    }
    c.eq(
        "LibreWolf 157 local",
        Verification::Verified {
            evidence: evidence.to_string(),
            note: String::new(),
        },
        status(&librewolf157(), Strategy::Local),
    );
    c.eq(
        "LibreWolf 158 crowd",
        Verification::Unverified,
        status(
            &engine(Family::Gecko, Brand::LibreWolf, 158, "158.0"),
            Strategy::Crowd,
        ),
    );
    c.eq(
        "Chrome 155 crowd",
        Verification::Unverified,
        status(&chrome155(), Strategy::Crowd),
    );

    for row in registry_rows() {
        let evidence = row["evidence"].as_str().unwrap_or("");
        c.holds(
            &format!("каталог evidence существует: {evidence}"),
            PathBuf::from(REPO).join(evidence).is_dir(),
        );
    }
    c.finish();
}
