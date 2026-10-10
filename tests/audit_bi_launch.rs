//! BI.L1–BI.L3: исполнитель передаёт браузеру ровно план (E15), Gecko пишет user.js (E16),
//! отказы различимы и не запускают браузер (E17). Браузер — фикстура из `recording_fixture`.

#[path = "bi_support/mod.rs"]
mod support;

use cm::common::contract_fixtures::TempDirGuard;
use cm::common::sys::euid;
use cm::identity::guard::load_state;
use cm::identity::launch::{LaunchError, launch};
use cm::identity::model::{BrowserIdentityProfile, Strategy};
use cm::identity::plan::{chromium_plan, gecko_plan};
use cm::identity::store::IdentityStore;
use cm::identity::validate::Violation;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use support::{
    Checks, chrome155, de_env, env_guard, librewolf157, profile, read_runs, recording_fixture,
    set_env, unset_env,
};

const LOCAL_USER_JS: &str = "// Managed by cm identity. Changes here are overwritten at launch.\nuser_pref(\"intl.accept_languages\", \"de-DE, de, en-US, en\");\nuser_pref(\"intl.locale.requested\", \"de-DE\");\nuser_pref(\"privacy.resistFingerprinting\", false);\n";

struct Lab {
    base: TempDirGuard,
    store: IdentityStore,
}

impl Lab {
    fn new(prefix: &str) -> Self {
        let base = TempDirGuard::new(prefix).expect("tmp");
        let store = IdentityStore::open(base.path().join("ids")).expect("open");
        // CM_STATE_DIR включает test_mode: launch от root в лаборатории не отказывает раньше времени.
        fs::create_dir_all(base.path().join("state")).expect("state");
        set_env(
            "CM_STATE_DIR",
            base.path().join("state").to_str().expect("utf-8"),
        );
        set_env(
            "CM_CONF",
            base.path().join("no-such.conf").to_str().expect("utf-8"),
        );
        Self { base, store }
    }

    fn bin(&self) -> PathBuf {
        let dir = self.base.path().join("bin");
        fs::create_dir_all(&dir).expect("bin");
        dir
    }

    fn create(
        &self,
        id: &str,
        browser: &Path,
        engine: cm::identity::engine::EngineInfo,
        env: Option<cm::identity::presets::EnvironmentProfile>,
        strategy: Strategy,
    ) -> BrowserIdentityProfile {
        let p = profile(id, strategy, browser, engine, env);
        self.store.create(&p).expect("create");
        p
    }
}

fn argv_of(run: &serde_json::Value) -> Vec<String> {
    run["argv"]
        .as_array()
        .expect("argv")
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_string())
        .collect()
}

fn env_of(run: &serde_json::Value) -> BTreeMap<String, String> {
    run["env"]
        .as_object()
        .expect("env")
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
        .collect()
}

fn process_env() -> BTreeMap<String, String> {
    std::env::vars().collect()
}

#[test]
fn e15_browser_gets_exactly_the_plan() {
    let _env = env_guard();
    let mut c = Checks::new("E15");
    let lab = Lab::new("cm-bi-e15");
    set_env("SECRET_TOKEN", "x");
    let bin = lab.bin();
    let browser = recording_fixture(&bin, "chrome", "Google Chrome 155.0.8059.39", 0);
    lab.create(
        "work",
        &browser,
        chrome155(),
        Some(de_env()),
        Strategy::Local,
    );

    let got = launch(&lab.store, "work", None, false);
    c.eq("launch(work DE local) → Ok(0)", Ok(0), got);

    let runs = read_runs(&bin);
    c.eq("ровно один запуск фикстуры", 1, runs.len());
    if let Some(run) = runs.first() {
        let profile = lab.store.load("work").expect("load");
        let expected = chromium_plan(
            &profile,
            &lab.store.profile_dir("work"),
            &process_env(),
            None,
            false,
        );
        c.eq("argv == plan.args", expected.args.clone(), argv_of(run));
        let got_env = env_of(run);
        c.eq("env == plan.env", expected.env.clone(), got_env.clone());
        c.holds(
            "SECRET_TOKEN не передан",
            !got_env.contains_key("SECRET_TOKEN"),
        );
        // Фикстура сама читает getsid(0) в момент запуска: сессия браузера совпадает с его pid.
        c.eq(
            "браузер в новой сессии: getsid(pid) == pid",
            run["pid"].clone(),
            run["sid"].clone(),
        );
    }

    let log_mode = fs::metadata(lab.store.dir("work").join("browser.log"))
        .map(|m| m.permissions().mode() & 0o777)
        .unwrap_or(0);
    c.eq("browser.log 0600", 0o600, log_mode);

    let state = load_state(&lab.store.dir("work"));
    c.holds(
        "state.json: last_launch записан",
        state.last_launch.is_some(),
    );
    c.holds("state.json: last_exit записан", state.last_exit.is_some());
    c.finish();
}

#[test]
fn e16_gecko_writes_user_js_and_nothing_else() {
    let _env = env_guard();
    let mut c = Checks::new("E16");
    let lab = Lab::new("cm-bi-e16");
    let bin = lab.bin();
    let browser = recording_fixture(&bin, "librewolf", "LibreWolf 157.0-1", 0);
    let identity = lab.create(
        "lw",
        &browser,
        librewolf157(),
        Some(de_env()),
        Strategy::Local,
    );

    let profile_dir = lab.store.profile_dir("lw");
    fs::write(profile_dir.join("prefs.js"), "pref-original\n").expect("prefs.js");

    c.eq(
        "launch(lw) → Ok(0)",
        Ok(0),
        launch(&lab.store, "lw", None, false),
    );

    let user_js = fs::read_to_string(profile_dir.join("user.js")).unwrap_or_default();
    c.eq(
        "user.js == строка E10 (Local DE)",
        LOCAL_USER_JS.to_string(),
        user_js,
    );
    let mode = fs::metadata(profile_dir.join("user.js"))
        .map(|m| m.permissions().mode() & 0o777)
        .unwrap_or(0);
    c.eq("user.js 0600", 0o600, mode);
    let prefs = fs::read_to_string(profile_dir.join("prefs.js")).unwrap_or_default();
    c.eq("prefs.js не изменён", "pref-original\n".to_string(), prefs);

    let runs = read_runs(&bin);
    if let Some(run) = runs.first() {
        let expected = gecko_plan(&identity, &profile_dir, &process_env(), None, false);
        c.eq(
            "argv == plan.args (Gecko)",
            expected.args.clone(),
            argv_of(run),
        );
    } else {
        c.holds("фикстура Gecko запущена", false);
    }
    c.finish();
}

#[test]
fn e17_refusals_are_distinct_and_do_not_start_the_browser() {
    let _env = env_guard();
    let mut c = Checks::new("E17");

    // AlreadyRunning: первый запуск спит 2 с и держит run.lock; второй должен отказать сразу.
    let lab = Lab::new("cm-bi-e17-busy");
    let bin = lab.bin();
    let slow = recording_fixture(&bin, "slow-chrome", "Google Chrome 155.0.8059.39", 2);
    lab.create("slow", &slow, chrome155(), Some(de_env()), Strategy::Local);
    let store = &lab.store;
    let (first, second) = thread::scope(|scope| {
        let first = scope.spawn(|| launch(store, "slow", None, false));
        thread::sleep(Duration::from_millis(600));
        let second = launch(store, "slow", None, false);
        (first.join().expect("поток первого запуска"), second)
    });
    c.eq(
        "второй launch пока первый ждёт → AlreadyRunning",
        Err(LaunchError::AlreadyRunning),
        second,
    );
    c.eq("первый launch завершился → Ok(0)", Ok(0), first);
    c.eq("второй браузер не запускался", 1, read_runs(&bin).len());

    // Invalid: запрещённый флаг; argv-файл не должен появиться.
    let lab = Lab::new("cm-bi-e17-forbidden");
    let bin = lab.bin();
    let browser = recording_fixture(&bin, "chrome", "Google Chrome 155.0.8059.39", 0);
    let mut bad = lab.create("fa", &browser, chrome155(), Some(de_env()), Strategy::Local);
    bad.extra_args = vec!["--user-agent=x".to_string()];
    lab.store.save(&bad).expect("save");
    c.eq(
        "ForbiddenArgument → Invalid",
        Err(LaunchError::Invalid(vec![Violation::ForbiddenArgument(
            "--user-agent".into(),
        )])),
        launch(&lab.store, "fa", None, false),
    );
    c.holds("argv-файл не создан", read_runs(&bin).is_empty());

    // Версия сменилась: профиль сохраняет новый движок и запись в истории.
    let lab = Lab::new("cm-bi-e17-engine");
    let bin = lab.bin();
    let browser = recording_fixture(&bin, "chrome", "Google Chrome 155.0.8059.39", 0);
    lab.create(
        "ver",
        &browser,
        chrome155(),
        Some(de_env()),
        Strategy::Local,
    );
    recording_fixture(&bin, "chrome", "Google Chrome 156.0.1.1", 0);
    let _ = launch(&lab.store, "ver", None, false);
    let saved = lab.store.load("ver").expect("load ver");
    c.eq("движок сохранён как 156", 156, saved.engine.major);
    c.eq(
        "history: engine Chrome 156",
        Some("engine Chrome 156".to_string()),
        saved.history.last().map(|h| h.change.clone()),
    );

    // Патч-версия того же мажора: снимок обновлён, поколение и история не тратятся.
    let lab = Lab::new("cm-bi-e17-patch");
    let bin = lab.bin();
    let browser = recording_fixture(&bin, "chrome", "Google Chrome 155.0.8059.39", 0);
    lab.create(
        "patch",
        &browser,
        chrome155(),
        Some(de_env()),
        Strategy::Local,
    );
    let before = lab.store.load("patch").expect("load patch");
    recording_fixture(&bin, "chrome", "Google Chrome 155.0.9999.1", 0);
    c.eq(
        "патч-версия: запуск проходит",
        Ok(0),
        launch(&lab.store, "patch", None, false),
    );
    let saved = lab.store.load("patch").expect("load patch");
    c.eq(
        "патч-версия: снимок обновлён",
        "155.0.9999.1".to_string(),
        saved.engine.version.clone(),
    );
    c.eq(
        "патч-версия: поколение не растёт",
        before.generation,
        saved.generation,
    );
    c.eq(
        "патч-версия: история не тратится",
        before.history.len(),
        saved.history.len(),
    );

    // RunAsRoot: проверяется только при запуске от root без test_mode.
    if euid() == 0 {
        let root_lab = Lab::new("cm-bi-e17-root");
        let root_bin = root_lab.bin();
        let browser = recording_fixture(&root_bin, "chrome", "Google Chrome 155.0.8059.39", 0);
        root_lab.create("r", &browser, chrome155(), Some(de_env()), Strategy::Local);
        // Без test_mode запуск от root обязан отказать до запуска браузера.
        unset_env("CM_STATE_DIR");
        c.eq(
            "euid 0 без test_mode → RunAsRoot",
            Err(LaunchError::RunAsRoot),
            launch(&root_lab.store, "r", None, false),
        );
    } else {
        eprintln!(
            "E17 NOT_APPLICABLE: RunAsRoot — тест запущен не от root (euid {})",
            euid()
        );
    }
    c.finish();
}
