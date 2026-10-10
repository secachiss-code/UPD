//! BI.G1–BI.G3: отпечаток профиля (E11), обнаружение запуска мимо CM (E12, E13b), подтверждение (E14).

#[path = "bi_support/mod.rs"]
mod support;

use cm::common::contract_fixtures::TempDirGuard;
use cm::identity::engine::Family;
use cm::identity::guard::{
    BypassPolicy, GuardError, GuardOutcome, LaunchMark, check_before_launch, confirm, fingerprint,
    load_state, record_exit, record_launch,
};
use cm::identity::model::{BrowserIdentityProfile, Strategy};
use cm::identity::store::IdentityStore;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use support::{Checks, TIME_NOW, chrome155, de_env, profile};

const PREFERENCES_EMPTY_SHA: &str =
    "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";

fn fresh(store: &IdentityStore, id: &str) -> BrowserIdentityProfile {
    let p = profile(
        id,
        Strategy::Local,
        Path::new("/usr/bin/google-chrome-stable"),
        chrome155(),
        Some(de_env()),
    );
    store.create(&p).expect("create");
    p
}

fn write_preferences(store: &IdentityStore, id: &str, body: &str) {
    let dir = store.profile_dir(id).join("Default");
    fs::create_dir_all(&dir).expect("Default/");
    fs::write(dir.join("Preferences"), body).expect("Preferences");
}

fn never_alive(_pid: u32) -> bool {
    false
}

#[test]
fn e11_fingerprint_tracks_bytes_and_marks_survive_restart() {
    let mut c = Checks::new("E11");
    let guard = TempDirGuard::new("cm-bi-e11").expect("tmp");
    let profile_dir = guard.path().join("profile");
    fs::create_dir_all(&profile_dir).unwrap();

    let empty = fingerprint(&profile_dir, Family::Chromium);
    c.eq("пустой профиль: 2 записи", 2, empty.len());
    c.holds(
        "пустой профиль: все отсутствуют",
        empty.iter().all(|m| !m.present),
    );

    fs::create_dir_all(profile_dir.join("Default")).unwrap();
    fs::write(profile_dir.join("Default/Preferences"), "{}").unwrap();
    let one = fingerprint(&profile_dir, Family::Chromium);
    let prefs = one
        .iter()
        .find(|m| m.path == "Default/Preferences")
        .cloned();
    c.holds("Default/Preferences найден", prefs.is_some());
    if let Some(mark) = prefs {
        c.holds("Default/Preferences present", mark.present);
        c.eq("Default/Preferences size", 2, mark.size);
        c.eq(
            "Default/Preferences sha256",
            PREFERENCES_EMPTY_SHA.to_string(),
            mark.sha256.clone(),
        );
    }

    let again = fingerprint(&profile_dir, Family::Chromium);
    c.eq("без правок — отпечаток равен", one.clone(), again);

    fs::write(profile_dir.join("Default/Preferences"), "{ }").unwrap();
    let changed = fingerprint(&profile_dir, Family::Chromium);
    c.holds("правка одного байта меняет отпечаток", changed != one);

    let state_dir = guard.path().join("state");
    fs::create_dir_all(&state_dir).unwrap();
    record_launch(&state_dir, 4, 4242, TIME_NOW).expect("record_launch");
    c.eq(
        "load_state == записанному запуску",
        Some(LaunchMark {
            generation: 4,
            pid: 4242,
            started_at: TIME_NOW,
        }),
        load_state(&state_dir).last_launch,
    );
    let mode = fs::metadata(state_dir.join("state.json"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    c.eq("state.json 0600", 0o600, mode);
    c.finish();
}

#[test]
fn e12_guard_before_launch_is_unambiguous() {
    let mut c = Checks::new("E12");
    let guard = TempDirGuard::new("cm-bi-e12").expect("tmp");
    let store = IdentityStore::open(guard.path().join("ids")).expect("open");

    let mut first = fresh(&store, "n1");
    c.eq(
        "новая личность без state.json",
        Ok(GuardOutcome::FirstLaunch),
        check_before_launch(
            &store,
            &mut first,
            Family::Chromium,
            BypassPolicy::Block,
            &never_alive,
        ),
    );

    let mut clean = fresh(&store, "n2");
    write_preferences(&store, "n2", "{}");
    record_exit(
        &store.dir("n2"),
        &store.profile_dir("n2"),
        Family::Chromium,
        TIME_NOW,
    )
    .expect("exit");
    c.eq(
        "record_exit, файлы не менялись",
        Ok(GuardOutcome::Clean),
        check_before_launch(
            &store,
            &mut clean,
            Family::Chromium,
            BypassPolicy::Block,
            &never_alive,
        ),
    );

    let mut blocked = fresh(&store, "n3");
    write_preferences(&store, "n3", "{}");
    record_exit(
        &store.dir("n3"),
        &store.profile_dir("n3"),
        Family::Chromium,
        TIME_NOW,
    )
    .expect("exit");
    write_preferences(&store, "n3", "{\"changed\":true}");
    c.eq(
        "Block: Preferences изменён",
        Err(GuardError::BypassSuspected),
        check_before_launch(
            &store,
            &mut blocked,
            Family::Chromium,
            BypassPolicy::Block,
            &never_alive,
        ),
    );
    let saved = store.load("n3").expect("load n3");
    c.holds(
        "Block: bypass_suspected сохранён в хранилище",
        saved.bypass_suspected,
    );
    c.eq(
        "Block: последняя запись history",
        Some("bypass-suspected".to_string()),
        saved.history.last().map(|h| h.change.clone()),
    );
    c.eq(
        "Block повторно без изменений",
        Err(GuardError::BypassSuspected),
        check_before_launch(
            &store,
            &mut blocked,
            Family::Chromium,
            BypassPolicy::Block,
            &never_alive,
        ),
    );

    let mut warned = fresh(&store, "n4");
    write_preferences(&store, "n4", "{}");
    record_exit(
        &store.dir("n4"),
        &store.profile_dir("n4"),
        Family::Chromium,
        TIME_NOW,
    )
    .expect("exit");
    write_preferences(&store, "n4", "{\"changed\":true}");
    c.eq(
        "Warn: Ok(Warned)",
        Ok(GuardOutcome::Warned),
        check_before_launch(
            &store,
            &mut warned,
            Family::Chromium,
            BypassPolicy::Warn,
            &never_alive,
        ),
    );
    let saved_warned = store.load("n4").expect("load n4");
    c.holds(
        "Warn: bypass_suspected остаётся false",
        !saved_warned.bypass_suspected,
    );
    c.eq(
        "Warn: последняя запись history",
        Some("bypass-warned".to_string()),
        saved_warned.history.last().map(|h| h.change.clone()),
    );

    let mut locked = fresh(&store, "n5");
    symlink("host-4242", store.profile_dir("n5").join("SingletonLock")).expect("SingletonLock");
    c.eq(
        "SingletonLock host-4242, процесс жив",
        Err(GuardError::BrowserRunning),
        check_before_launch(
            &store,
            &mut locked,
            Family::Chromium,
            BypassPolicy::Block,
            &|pid| pid == 4242,
        ),
    );

    let mut stale = fresh(&store, "n5b");
    symlink("host-4242", store.profile_dir("n5b").join("SingletonLock")).expect("SingletonLock");
    c.eq(
        "SingletonLock, процесс мёртв — проверка продолжается",
        Ok(GuardOutcome::FirstLaunch),
        check_before_launch(
            &store,
            &mut stale,
            Family::Chromium,
            BypassPolicy::Block,
            &never_alive,
        ),
    );

    let mut gecko = fresh(&store, "n6");
    symlink("127.0.0.1:+4242", store.profile_dir("n6").join("lock")).expect("lock");
    c.eq(
        "Gecko lock 127.0.0.1:+4242, процесс жив",
        Err(GuardError::BrowserRunning),
        check_before_launch(
            &store,
            &mut gecko,
            Family::Gecko,
            BypassPolicy::Block,
            &|pid| pid == 4242,
        ),
    );

    let mut crashed = fresh(&store, "n7");
    record_launch(&store.dir("n7"), 1, 999, TIME_NOW).expect("launch mark");
    c.eq(
        "last_launch без last_exit, Block",
        Err(GuardError::BypassSuspected),
        check_before_launch(
            &store,
            &mut crashed,
            Family::Chromium,
            BypassPolicy::Block,
            &never_alive,
        ),
    );
    c.finish();
}

#[test]
fn e13b_confirm_reads_bypass_flag_from_storage() {
    let mut c = Checks::new("E13b");
    let guard = TempDirGuard::new("cm-bi-e13b").expect("tmp");
    let store = IdentityStore::open(guard.path().join("ids")).expect("open");

    let mut p = fresh(&store, "n3");
    write_preferences(&store, "n3", "{}");
    record_exit(
        &store.dir("n3"),
        &store.profile_dir("n3"),
        Family::Chromium,
        TIME_NOW,
    )
    .expect("exit");
    write_preferences(&store, "n3", "{\"changed\":true}");
    c.eq(
        "Block: Err(BypassSuspected)",
        Err(GuardError::BypassSuspected),
        check_before_launch(
            &store,
            &mut p,
            Family::Chromium,
            BypassPolicy::Block,
            &never_alive,
        ),
    );

    // Признак читается из identity.json на диске, а не из памяти процесса.
    let identity_text =
        fs::read_to_string(store.dir("n3").join("identity.json")).unwrap_or_default();
    let identity_json: serde_json::Value = serde_json::from_str(&identity_text).unwrap_or_default();
    c.eq(
        "identity.json содержит bypass_suspected: true",
        serde_json::json!(true),
        identity_json["bypass_suspected"].clone(),
    );
    c.finish();
}

#[test]
fn e14_confirm_clears_once_and_updates_reference() {
    let mut c = Checks::new("E14");
    let guard = TempDirGuard::new("cm-bi-e14").expect("tmp");
    let store = IdentityStore::open(guard.path().join("ids")).expect("open");

    let mut suspect = fresh(&store, "n8");
    write_preferences(&store, "n8", "{}");
    record_exit(
        &store.dir("n8"),
        &store.profile_dir("n8"),
        Family::Chromium,
        TIME_NOW,
    )
    .expect("exit");
    write_preferences(&store, "n8", "{\"changed\":true}");
    let _ = check_before_launch(
        &store,
        &mut suspect,
        Family::Chromium,
        BypassPolicy::Block,
        &never_alive,
    );

    let mut loaded = store.load("n8").expect("load n8");
    c.eq("до confirm: подозрение есть", true, loaded.bypass_suspected);
    c.eq(
        "confirm",
        Ok(()),
        confirm(&store, &mut loaded, TIME_NOW + 10),
    );
    c.holds(
        "после confirm bypass_suspected false",
        !loaded.bypass_suspected,
    );
    c.eq(
        "после confirm последняя запись history",
        Some("bypass-confirmed".to_string()),
        loaded.history.last().map(|h| h.change.clone()),
    );
    c.eq(
        "следующий check_before_launch → Clean",
        Ok(GuardOutcome::Clean),
        check_before_launch(
            &store,
            &mut loaded,
            Family::Chromium,
            BypassPolicy::Block,
            &never_alive,
        ),
    );

    let mut calm = fresh(&store, "n9");
    let generation = calm.generation;
    let history_len = calm.history.len();
    c.eq(
        "confirm без подозрения",
        Ok(()),
        confirm(&store, &mut calm, TIME_NOW + 20),
    );
    c.eq("generation не меняется", generation, calm.generation);
    c.eq("history не меняется", history_len, calm.history.len());
    c.finish();
}
