//! BI.C2–BI.C3: команды `cm identity` — коды выхода, вывод, языки справки (E19).
//! Бинарник `cm` запускается в очищенном окружении с временным каталогом личностей.

#[path = "bi_support/mod.rs"]
mod support;

use cm::common::contract_fixtures::TempDirGuard;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use support::{Checks, read_runs, recording_fixture};

const CM: &str = env!("CARGO_BIN_EXE_cm");

struct Env {
    base: TempDirGuard,
    chrome: PathBuf,
}

impl Env {
    fn new() -> Self {
        let base = TempDirGuard::new("cm-bi-e19").expect("tmp");
        let bin = base.path().join("bin");
        fs::create_dir_all(&bin).expect("bin");
        let chrome = recording_fixture(&bin, "chrome", "Google Chrome 155.0.8059.39", 0);
        Self { base, chrome }
    }

    fn root(&self) -> PathBuf {
        self.base.path().join("ids")
    }

    fn command(&self, args: &[&str], extra: &[(&str, &str)]) -> Command {
        let mut command = Command::new(CM);
        command
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.base.path().join("home"))
            .env("CM_IDENTITY_ROOT", self.root())
            .env("CM_STATE_DIR", self.base.path().join("state"))
            .env("CM_CONF", self.base.path().join("no-such.conf"));
        for (key, value) in extra {
            command.env(key, value);
        }
        command
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        self.run_with(args, &[])
    }

    fn run_with(&self, args: &[&str], extra: &[(&str, &str)]) -> (i32, String, String) {
        let out = self.command(args, extra).output().expect("запуск cm");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn identity(&self, id: &str) -> serde_json::Value {
        let text =
            fs::read_to_string(self.root().join(id).join("identity.json")).expect("identity.json");
        serde_json::from_str(&text).expect("JSON")
    }
}

fn chrome_str(env: &Env) -> String {
    env.chrome.to_str().expect("utf-8").to_string()
}

fn has_cyrillic(text: &str) -> bool {
    text.chars()
        .any(|ch| ('\u{0400}'..='\u{04FF}').contains(&ch))
}

#[test]
fn e19_cli_codes_output_and_help_languages() {
    let mut c = Checks::new("E19");
    let env = Env::new();
    let chrome = chrome_str(&env);

    let (code, _out, err) = env.run(&[
        "identity",
        "create",
        "work",
        "--browser",
        &chrome,
        "--strategy",
        "local",
        "--country",
        "DE",
    ]);
    c.eq("create work DE local → код 0", 0, code);
    if code != 0 {
        eprintln!("E19 диагностика create: {err}");
    }
    let identity = env.identity("work");
    c.eq(
        "identity.json: environment.country",
        serde_json::json!("DE"),
        identity["environment"]["country"].clone(),
    );
    c.eq(
        "identity.json: engine.brand",
        serde_json::json!("Chrome"),
        identity["engine"]["brand"].clone(),
    );
    c.eq(
        "identity.json: engine.major",
        serde_json::json!(155),
        identity["engine"]["major"].clone(),
    );

    let (code, _, _) = env.run(&[
        "identity",
        "create",
        "work",
        "--browser",
        &chrome,
        "--strategy",
        "local",
        "--country",
        "DE",
    ]);
    c.eq("create work повторно → код 5", 5, code);
    let (code, _, _) = env.run(&[
        "identity",
        "create",
        "x",
        "--browser",
        &chrome,
        "--strategy",
        "local",
    ]);
    c.eq("create x local без --country → код 2", 2, code);

    let (code, _, err) = env.run(&[
        "identity",
        "create",
        "c",
        "--browser",
        &chrome,
        "--strategy",
        "crowd",
    ]);
    c.eq("create c crowd на Chrome → код 2", 2, code);
    c.holds(
        "вывод содержит [StrategyEngineUnsupported]",
        err.contains("[StrategyEngineUnsupported]"),
    );

    let (code, out, _) = env.run(&["identity", "check", "work"]);
    c.eq("check work → код 0", 0, code);
    for axis in ["NET", "REGION", "STATE", "APP"] {
        c.holds(&format!("check: вывод содержит {axis}"), out.contains(axis));
    }

    let (code, _, _) = env.run(&["identity", "launch", "work"]);
    c.eq("launch work (первый) → код 0", 0, code);

    let prefs_dir = env.root().join("work").join("profile").join("Default");
    fs::create_dir_all(&prefs_dir).expect("Default");
    fs::write(prefs_dir.join("Preferences"), "{\"moved\":true}").expect("Preferences");
    let (code, _, err) = env.run(&["identity", "launch", "work"]);
    c.eq("launch после обхода → код 3", 3, code);
    c.holds(
        "вывод содержит [BypassSuspected]",
        err.contains("[BypassSuspected]"),
    );
    let (code, _, _) = env.run(&["identity", "confirm", "work"]);
    c.eq("confirm work → код 0", 0, code);
    let (code, _, _) = env.run(&["identity", "launch", "work"]);
    c.eq("launch после confirm → код 0", 0, code);
    c.eq(
        "браузер запущен дважды (до и после confirm)",
        2,
        read_runs(&env.base.path().join("bin")).len(),
    );

    let (code, _, _) = env.run(&["identity", "launch", "work", "--lab"]);
    c.eq("launch --lab без CM_IDENTITY_LAB=1 → код 2", 2, code);

    let (code, _, err) = env.run(&[
        "identity",
        "create",
        "y",
        "--browser",
        &chrome,
        "--strategy",
        "local",
        "--country",
        "DE",
        "--",
        "--user-agent=x",
    ]);
    c.eq("create ... -- --user-agent=x → код 2", 2, code);
    c.holds(
        "вывод содержит [ForbiddenArgument]",
        err.contains("[ForbiddenArgument]"),
    );

    let conf = env.base.path().join("lang.conf");
    fs::write(&conf, "lang = auto\n").expect("lang.conf");
    let conf_str = conf.to_str().expect("utf-8").to_string();
    for (locale, russian) in [
        ("ru_RU.UTF-8", true),
        ("en_US.UTF-8", false),
        ("de_DE.UTF-8", false),
        ("it_IT.UTF-8", false),
        ("zh_CN.UTF-8", false),
        ("ar_EG.UTF-8", false),
    ] {
        let (code, out, err) = env_run_help(&env, &conf_str, locale);
        c.eq(&format!("справка {locale} → код 2"), 2, code);
        let text = format!("{out}{err}");
        c.holds(
            &format!("справка {locale}: непустая"),
            !text.trim().is_empty(),
        );
        if russian {
            c.holds("справка ru: кириллица присутствует", has_cyrillic(&text));
        } else {
            c.holds(
                &format!("справка {locale}: кириллицы нет"),
                !has_cyrillic(&text),
            );
        }
    }
    c.finish();
}

fn env_run_help(env: &Env, conf: &str, locale: &str) -> (i32, String, String) {
    env.run_with(&["identity"], &[("CM_CONF", conf), ("LC_ALL", locale)])
}

/// Ctrl+C у `cm identity launch` не оставляет браузер без присмотра: сигнал уходит
/// браузеру, метка выхода записана, следующий запуск не считается обходом.
#[test]
fn e17_stop_signal_reaches_the_browser_and_keeps_the_guard_clean() {
    let mut c = Checks::new("E17");
    let env = Env::new();
    let bin = env.base.path().join("bin");
    let chrome = recording_fixture(&bin, "slow", "Google Chrome 155.0.8059.39", 60);
    let (code, _, err) = env.run(&[
        "identity",
        "create",
        "sig",
        "--browser",
        chrome.to_str().expect("utf8"),
        "--strategy",
        "local",
        "--country",
        "DE",
    ]);
    c.eq(&format!("create sig → 0 ({err})"), 0, code);
    let mut child = env
        .command(&["identity", "launch", "sig"], &[])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("запуск cm");
    let started = std::time::Instant::now();
    while read_runs(&bin).is_empty() && started.elapsed() < std::time::Duration::from_secs(10) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let runs = read_runs(&bin);
    c.eq("браузер запущен", 1, runs.len());
    let browser_pid = runs
        .first()
        .and_then(|run| run["pid"].as_i64())
        .unwrap_or(0);
    let interrupted = Command::new("kill")
        .args(["-s", "INT", "--", &child.id().to_string()])
        .status()
        .expect("kill");
    c.holds("SIGINT отправлен cm", interrupted.success());
    let started = std::time::Instant::now();
    let mut exited = false;
    while started.elapsed() < std::time::Duration::from_secs(10) {
        if child.try_wait().expect("try_wait").is_some() {
            exited = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if !exited {
        let _ = child.kill();
    }
    let _ = child.wait();
    c.holds("cm завершился сам, а не по тайм-ауту теста", exited);
    let alive = Command::new("kill")
        .args(["-s", "0", "--", &browser_pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .expect("kill -0")
        .success();
    c.holds("браузер остановлен вместе с cm", !alive);
    let state = fs::read_to_string(env.root().join("sig/state.json")).unwrap_or_default();
    c.holds(
        "state.json содержит метку выхода",
        state.contains("\"last_exit\": {"),
    );
    recording_fixture(&bin, "slow", "Google Chrome 155.0.8059.39", 0);
    let (code, _, err) = env.run(&["identity", "launch", "sig"]);
    c.eq(
        &format!("следующий запуск не считается обходом ({err})"),
        0,
        code,
    );
    c.finish();
}
