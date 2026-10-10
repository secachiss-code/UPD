//! BI.M1: модель личности — идентификаторы, сериализация, схема, история (E06).

#[path = "bi_support/mod.rs"]
mod support;

use cm::identity::model::{BrowserIdentityProfile, ModelError, Strategy, validate_id};
use std::path::Path;
use support::{Checks, chrome155, de_env, profile};

fn de_profile(id: &str) -> BrowserIdentityProfile {
    profile(
        id,
        Strategy::Local,
        Path::new("/usr/bin/google-chrome-stable"),
        chrome155(),
        Some(de_env()),
    )
}

#[test]
fn e06_model_roundtrip_schema_and_history() {
    let mut c = Checks::new("E06");

    let max_id = format!("a{}", "b".repeat(31));
    let too_long = format!("a{}", "b".repeat(32));
    for ok in ["work", "a", "a-1", max_id.as_str()] {
        c.eq(&format!("validate_id({ok:?})"), Ok(()), validate_id(ok));
    }
    for bad in ["Work", "-a", "a/b", "a b", "", too_long.as_str()] {
        c.eq(
            &format!("validate_id({:?}) — отказ", &bad[..bad.len().min(12)]),
            Err(ModelError::BadId),
            validate_id(bad),
        );
    }

    let original = de_profile("work");
    let text = serde_json::to_string(&original).expect("сериализация");
    c.eq(
        "JSON roundtrip DE local",
        Ok(original.clone()),
        BrowserIdentityProfile::from_json(&text),
    );

    let mut value = serde_json::to_value(&original).expect("значение");
    value["x"] = serde_json::json!(1);
    c.holds(
        "лишнее поле `x` отвергается",
        BrowserIdentityProfile::from_json(&value.to_string()).is_err(),
    );

    let mut future = serde_json::to_value(&original).expect("значение");
    future["schema_version"] = serde_json::json!(2);
    c.eq(
        "schema_version 2 → UnsupportedSchema",
        Err(ModelError::UnsupportedSchema),
        BrowserIdentityProfile::from_json(&future.to_string()).map(|_| ()),
    );

    let mut p = de_profile("work");
    for n in 1..=7 {
        p.record(1_700_000_000 + n, &format!("change-{n}"));
    }
    c.eq("generation после 7 записей", 7, p.generation);
    c.eq("history.len() после 7 записей", 5, p.history.len());
    c.eq(
        "history[0].generation — третья запись",
        3,
        p.history[0].generation,
    );
    c.eq(
        "history[0].change — третья запись",
        "change-3".to_string(),
        p.history[0].change.clone(),
    );

    let mut secret = de_profile("work");
    secret.extra_args = vec!["--ozone-platform=wayland-SECRETX".to_string()];
    let debug = format!("{secret:?}");
    c.holds(
        "Debug не содержит значений extra_args",
        !debug.contains("SECRETX"),
    );
    c.finish();
}
