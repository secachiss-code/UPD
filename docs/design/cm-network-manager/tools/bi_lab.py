#!/usr/bin/env python3
"""E20 · BI.C3 → BI.Z (L3): лабораторная приёмка `cm identity` на настоящих браузерах.

Что проверяется (значения — из TESTS-BI.md, E20):
  * Chrome 155 и Brave 154, `create ... --strategy local --country DE`: первый запрос,
    окно, Worker и новая вкладка (window.open) — tz, Intl-локаль, languages, webdriver,
    UA без изменений;
  * LibreWolf 157, `--strategy crowd`: UA, tz, languages, webdriver;
  * LibreWolf 157, `--strategy local --country DE`: UA без изменений, tz, Intl-локаль,
    Accept-Language;
  * голый запуск браузера мимо CM на том же профиле, затем `cm identity launch --lab`
    → код 3 и `[BypassSuspected]` (Chrome local и LibreWolf local).

Механизм запуска — тот, что строит `src/identity/plan.rs`; флаги лаборатории
(`--headless`, `--no-sandbox`) добавляет план при `--lab`, пользователь их не задаёт.

Изоляция:
  * весь прогон внутри `unshare -rn` (у процессов есть только lo, внешней сети нет);
  * у каждого кейса временные CM_IDENTITY_ROOT, CM_STATE_DIR, HOME и XDG-каталоги,
    каталог удаляется после кейса; профили пользователя не читаются;
  * CM_CONF указывает на несуществующий файл — пользовательская конфигурация не влияет;
  * после каждого шага браузеры этого профиля завершаются (SIGTERM, затем SIGKILL).

Запуск (бинарник cm собирается заранее; сеть для сборки не нужна):
  cargo build --locked --offline --bin cm
  CM_BIN=$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/debug/cm \\
    python3 docs/design/cm-network-manager/tools/bi_lab.py OUT_DIR

Результат в OUT_DIR:
  runs.json         — сырые события сервера и вывод cm по каждому шагу;
  observations.json — значения окна, Worker, вкладки и первого запроса (без вердиктов);
  checks.json       — ожидаемые значения E20, фактические значения и PASS/FAIL;
  meta.json         — версии браузеров, хеш бинарника cm, окружение.
Код возврата: 0 — все проверки PASS, 1 — есть FAIL, 2 — ошибка вызова.
"""

import hashlib
import json
import os
import platform
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

PORT = 18765
URL = f"http://127.0.0.1:{PORT}/"
# Пусто в основном прогоне. CM_BI_LAB_EXTRA="--disable-popup-blocking" — только диагностика
# причины пропавшей вкладки; такой прогон не является приёмкой E20.
EXTRA_ARGS = os.environ.get("CM_BI_LAB_EXTRA", "").split()
WAIT_S = 30
KILL_WAIT_S = 6
CM_TIMEOUT_S = 90
CONTEXTS = ("window", "worker", "tab")

SNAP_JS = """
function snap(ctx) {
  var dtf = Intl.DateTimeFormat().resolvedOptions();
  return {
    ctx: ctx,
    ua: navigator.userAgent,
    lang: navigator.language,
    languages: Array.from(navigator.languages || []),
    tz: dtf.timeZone,
    intl_locale: dtf.locale,
    offset_jan: new Date(2026, 0, 15).getTimezoneOffset(),
    webdriver: navigator.webdriver === true
  };
}
function report(ctx) {
  return fetch('/report', {method: 'POST', body: JSON.stringify(snap(ctx))});
}
"""

PAGE = (
    '<!doctype html><meta charset="utf-8"><title>bi-lab</title><script>'
    + SNAP_JS
    + "report('window').then(function () {"
    + " new Worker('/worker.js');"
    + " window.open('/tab', '_blank');"
    + "});</script>"
)
TAB_PAGE = (
    '<!doctype html><meta charset="utf-8"><title>bi-lab tab</title><script>'
    + SNAP_JS
    + "report('tab');</script>"
)
WORKER_JS = SNAP_JS + "report('worker');\n"

# Ожидаемые значения E20. Browser — имя исполняемого файла в PATH.
CASES = [
    {
        "id": "chrome-local-de",
        "browser": "google-chrome-stable",
        "family": "chromium",
        "strategy": "local",
        "country": "DE",
        "ua_mark": "Chrome/155",
        "accept_prefix": "de-DE,de;q=0.9",
        "languages": ["de-DE", "de", "en-US", "en"],
        "tz": "Europe/Berlin",
        "intl": "de",
        "bypass": True,
    },
    {
        "id": "brave-local-de",
        "browser": "brave",
        "family": "chromium",
        "strategy": "local",
        "country": "DE",
        "ua_mark": "Chrome/154",
        # Brave сам случайно меняет q («farbling»): проверяется только первый язык.
        "accept_prefix": "de-DE",
        "languages": ["de-DE"],
        "tz": "Europe/Berlin",
        "intl": "de",
        "bypass": False,
    },
    {
        "id": "librewolf-crowd",
        "browser": "librewolf",
        "family": "gecko",
        "strategy": "crowd",
        "country": None,
        "ua_mark": "Firefox/157.0",
        "accept_prefix": None,
        "languages": ["en-US", "en"],
        "tz_any": ["Atlantic/Reykjavik", "UTC"],
        "intl": None,
        "bypass": False,
    },
    {
        "id": "librewolf-local-de",
        "browser": "librewolf",
        "family": "gecko",
        "strategy": "local",
        "country": "DE",
        "ua_mark": "Firefox/157.0",
        "accept_prefix": "de-DE",
        "languages": None,
        "tz": "Europe/Berlin",
        "intl": "de",
        "bypass": True,
    },
]


class Lab:
    def __init__(self):
        self.lock = threading.Lock()
        self.events = []

    def add(self, event):
        with self.lock:
            self.events.append(event)

    def clear(self):
        with self.lock:
            self.events.clear()

    def snapshot(self):
        with self.lock:
            return list(self.events)


LAB = Lab()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def headers_of_interest(self):
        keep = {}
        for key, value in self.headers.items():
            low = key.lower()
            if low in ("user-agent", "accept-language") or low.startswith("sec-ch-ua"):
                keep[low] = value
        return keep

    def send(self, status, body, content_type):
        data = body.encode()
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        LAB.add({"kind": "request", "path": self.path, "headers": self.headers_of_interest()})
        if self.path == "/":
            self.send(200, PAGE, "text/html; charset=utf-8")
        elif self.path == "/tab":
            self.send(200, TAB_PAGE, "text/html; charset=utf-8")
        elif self.path == "/worker.js":
            self.send(200, WORKER_JS, "text/javascript")
        else:
            self.send(404, "", "text/plain")

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(length) or b"{}")
        LAB.add({"kind": "report", "headers": self.headers_of_interest(), "values": body})
        self.send(204, "", "text/plain")


def cm_env(root):
    home = root / "home"
    return {
        "PATH": "/usr/bin:/bin",
        "HOME": str(home),
        "XDG_CONFIG_HOME": str(home / ".config"),
        "XDG_CACHE_HOME": str(home / ".cache"),
        "LC_ALL": "C.UTF-8",
        "CM_IDENTITY_ROOT": str(root / "ids"),
        "CM_STATE_DIR": str(root / "state"),
        "CM_CONF": str(root / "no-such.conf"),
        "CM_IDENTITY_LAB": "1",
    }


def bare_env(root):
    """Среда «мимо CM»: тот же HOME и профиль, но без TZ и без языков CM."""
    home = root / "home"
    return {
        "PATH": "/usr/bin:/bin",
        "HOME": str(home),
        "XDG_CONFIG_HOME": str(home / ".config"),
        "XDG_CACHE_HOME": str(home / ".cache"),
        "LANG": "C.UTF-8",
    }


def profile_dir(root, ident):
    return root / "ids" / ident / "profile"


def browser_argv(case, binary, profile, url, bare):
    if case["family"] == "chromium":
        argv = [
            binary,
            "--headless=new",
            "--no-sandbox",
            "--no-first-run",
            "--no-default-browser-check",
            f"--user-data-dir={profile}",
        ]
    else:
        argv = [binary, "--headless", "--no-remote", "--profile", str(profile)]
    return argv + [url]


def profile_pids(needle):
    pids = []
    own = os.getpid()
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit() or int(entry.name) == own:
            continue
        try:
            cmdline = (entry / "cmdline").read_bytes()
        except OSError:
            continue
        if needle in cmdline:
            pids.append(int(entry.name))
    return pids


def kill_profile(needle):
    """Завершает все процессы браузера, у которых в аргументах путь каталога этого кейса."""
    sent = set()
    for sig, wait in ((signal.SIGTERM, KILL_WAIT_S), (signal.SIGKILL, KILL_WAIT_S)):
        deadline = time.time() + wait
        while True:
            pids = profile_pids(needle)
            if not pids:
                return sorted(sent)
            if time.time() > deadline:
                break
            for pid in pids:
                try:
                    os.kill(pid, sig)
                    sent.add(pid)
                except ProcessLookupError:
                    pass
            time.sleep(0.2)
    return sorted(sent)


def wait_contexts(proc, deadline_s):
    deadline = time.time() + deadline_s
    while time.time() < deadline and proc.poll() is None:
        got = {e["values"].get("ctx") for e in LAB.snapshot() if e["kind"] == "report"}
        if set(CONTEXTS) <= got:
            time.sleep(0.5)
            return True
        time.sleep(0.2)
    return False


def run_cm(cm_bin, root, args):
    return subprocess.run(
        [cm_bin, "identity", *args],
        env=cm_env(root),
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        timeout=CM_TIMEOUT_S,
    )


def launch_with_browser(cm_bin, root, ident, needle):
    LAB.clear()
    proc = subprocess.Popen(
        [cm_bin, "identity", "launch", ident, URL, "--lab"],
        env=cm_env(root),
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        start_new_session=True,
    )
    reached = wait_contexts(proc, WAIT_S)
    killed = kill_profile(needle)
    try:
        out, err = proc.communicate(timeout=CM_TIMEOUT_S)
    except subprocess.TimeoutExpired:
        proc.kill()
        out, err = proc.communicate()
    return {
        "cm_rc": proc.returncode,
        "stdout": out,
        "stderr": err,
        "contexts_reached": reached,
        "killed_pids": killed,
        "events": LAB.snapshot(),
    }


def bare_launch(case, binary, root, ident, needle):
    LAB.clear()
    argv = browser_argv(case, binary, profile_dir(root, ident), URL, bare=True)
    proc = subprocess.Popen(
        argv,
        env=bare_env(root),
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    reached = wait_contexts(proc, WAIT_S)
    killed = kill_profile(needle)
    try:
        proc.wait(timeout=KILL_WAIT_S)
    except subprocess.TimeoutExpired:
        pass
    return {"contexts_reached": reached, "killed_pids": killed, "events": LAB.snapshot()}


def version_of(binary, root):
    try:
        res = subprocess.run([binary, "--version"], env=bare_env(root), capture_output=True, text=True, timeout=30)
        return (res.stdout or res.stderr).strip()
    except (OSError, subprocess.TimeoutExpired):
        return None


def last_report(events, ctx):
    found = None
    for event in events:
        if event["kind"] == "report" and event["values"].get("ctx") == ctx:
            found = event
    return found


def first_root_request(events):
    return next((e for e in events if e["kind"] == "request" and e["path"] == "/"), None)


def observe(events):
    """Значения без вердиктов: первый запрос документа и отчёты по контекстам."""
    obs = {}
    first = first_root_request(events)
    obs["first_request"] = (
        {"accept_language": first["headers"].get("accept-language"), "user_agent": first["headers"].get("user-agent")}
        if first
        else None
    )
    for ctx in CONTEXTS:
        rep = last_report(events, ctx)
        obs[ctx] = rep["values"] if rep else None
    return obs


def check(checks, name, expected, actual):
    ok = actual == expected
    checks.append({"check": name, "expected": expected, "actual": actual, "result": "PASS" if ok else "FAIL"})


def check_present(checks, name, actual_value, predicate, expected_text):
    ok = actual_value is not None and predicate(actual_value)
    checks.append({"check": name, "expected": expected_text, "actual": actual_value, "result": "PASS" if ok else "FAIL"})


def evaluate_step(checks, case, step, events):
    prefix = f"E20.{case['id']}.{step}"
    obs = observe(events)
    first = obs["first_request"]
    if case["accept_prefix"] is not None:
        actual = first["accept_language"] if first else None
        check_present(
            checks,
            f"{prefix}.first-request.accept-language",
            actual,
            lambda v: v.startswith(case["accept_prefix"]),
            f"начинается с {case['accept_prefix']!r}",
        )
    ua_expected_text = f"содержит {case['ua_mark']!r}, без CMLab"
    first_ua = first["user_agent"] if first else None
    check_present(
        checks,
        f"{prefix}.first-request.user-agent",
        first_ua,
        lambda v: case["ua_mark"] in v and "CMLab" not in v,
        ua_expected_text,
    )
    for ctx in CONTEXTS:
        values = obs[ctx]
        name = f"{prefix}.{ctx}"
        if values is None:
            checks.append({"check": name, "expected": "отчёт получен", "actual": None, "result": "FAIL"})
            continue
        check_present(
            checks,
            f"{name}.user-agent",
            values.get("ua"),
            lambda v: case["ua_mark"] in v and "CMLab" not in v,
            ua_expected_text,
        )
        if "tz" in case:
            check(checks, f"{name}.timezone", case["tz"], values.get("tz"))
        if "tz_any" in case:
            check_present(
                checks,
                f"{name}.timezone",
                values.get("tz"),
                lambda v: v in case["tz_any"],
                f"одна из {case['tz_any']}",
            )
        if case["intl"] is not None:
            locale = values.get("intl_locale") or ""
            check(checks, f"{name}.intl-locale-language", case["intl"], locale.split("-")[0])
        if case["languages"] is not None:
            check(checks, f"{name}.languages", case["languages"], values.get("languages"))
        check(checks, f"{name}.webdriver", False, values.get("webdriver"))
        if case["id"].startswith("chrome-local") or case["id"].startswith("brave-local"):
            check(checks, f"{name}.offset-january", -60, values.get("offset_jan"))


def inner(out, cm_bin):
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
    server = ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()

    runs = []
    observations = []
    checks = []
    meta = {
        "browsers": {},
        "cm_sha256": hashlib.sha256(Path(cm_bin).read_bytes()).hexdigest(),
        "extra_args": EXTRA_ARGS,
    }

    for case in CASES:
        binary = shutil.which(case["browser"])
        if binary is None:
            checks.append({"check": f"E20.{case['id']}.browser-present", "expected": "найден", "actual": None, "result": "FAIL"})
            continue
        root = Path(tempfile.mkdtemp(prefix="bi-lab-"))
        needle = str(root).encode()
        try:
            for sub in ("home", "state", "ids"):
                (root / sub).mkdir(parents=True)
            meta["browsers"][case["browser"]] = {"path": binary, "version": version_of(binary, root)}
            ident = case["id"]
            argv = ["create", ident, "--browser", binary, "--strategy", case["strategy"]]
            if case["country"]:
                argv += ["--country", case["country"]]
            if EXTRA_ARGS:
                # Диагностика: дополнительные аргументы браузера через `cm identity create -- …`.
                argv += ["--", *EXTRA_ARGS]
            created = run_cm(cm_bin, root, argv)
            check(checks, f"E20.{case['id']}.create-rc", 0, created.returncode)
            if created.returncode != 0:
                runs.append({"case": case["id"], "step": "create", "rc": created.returncode, "stderr": created.stderr})
                continue

            for step in ("first-launch", "cold-restart-same-launcher"):
                result = launch_with_browser(cm_bin, root, ident, needle)
                runs.append({"case": case["id"], "step": step, **result})
                check(checks, f"E20.{case['id']}.{step}.contexts-reached", True, result["contexts_reached"])
                evaluate_step(checks, case, step, result["events"])
                observations.append({"case": case["id"], "step": step, "observed": observe(result["events"])})

            if case["bypass"]:
                bare = bare_launch(case, binary, root, ident, needle)
                runs.append({"case": case["id"], "step": "bare-launch-outside-cm", **bare})
                after = run_cm(cm_bin, root, ["launch", ident, "--lab"])
                runs.append(
                    {
                        "case": case["id"],
                        "step": "launch-after-bare",
                        "cm_rc": after.returncode,
                        "stdout": after.stdout,
                        "stderr": after.stderr,
                        "events": [],
                    }
                )
                check(checks, f"E20.{case['id']}.launch-after-bare.rc", 3, after.returncode)
                check_present(
                    checks,
                    f"E20.{case['id']}.launch-after-bare.code",
                    after.stderr,
                    lambda v: "[BypassSuspected]" in v,
                    "вывод содержит [BypassSuspected]",
                )
        finally:
            shutil.rmtree(root, ignore_errors=True)

    server.shutdown()
    passed = sum(1 for c in checks if c["result"] == "PASS")
    failed = sum(1 for c in checks if c["result"] == "FAIL")
    meta["counters"] = {"checks": len(checks), "pass": passed, "fail": failed}
    meta["platform"] = platform.platform()
    meta["python"] = sys.version.split()[0]
    (out / "runs.json").write_text(json.dumps(runs, ensure_ascii=False, indent=2) + "\n")
    (out / "observations.json").write_text(json.dumps(observations, ensure_ascii=False, indent=2) + "\n")
    (out / "checks.json").write_text(json.dumps(checks, ensure_ascii=False, indent=2) + "\n")
    (out / "meta.json").write_text(json.dumps(meta, ensure_ascii=False, indent=2) + "\n")
    print(f"E20 lab: checks {len(checks)}, PASS {passed}, FAIL {failed}", flush=True)
    for c in checks:
        if c["result"] == "FAIL":
            print(f"  FAIL {c['check']}: ожидалось {c['expected']!r}, получено {c['actual']!r}", flush=True)
    sys.exit(0 if failed == 0 else 1)


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(2)
    out = Path(sys.argv[1]).resolve()
    out.mkdir(parents=True, exist_ok=True)
    cm_bin = os.environ.get("CM_BIN", "")
    if os.environ.get("CM_BI_LAB_INNER") == "1":
        inner(out, cm_bin)
        return
    if not cm_bin or not os.access(cm_bin, os.X_OK):
        print("CM_BIN: укажите собранный бинарник cm", file=sys.stderr)
        sys.exit(2)
    if shutil.which("unshare") is None:
        print("unshare не найден", file=sys.stderr)
        sys.exit(2)
    env = dict(os.environ, CM_BI_LAB_INNER="1", CM_BIN=str(Path(cm_bin).resolve()))
    code = subprocess.call(["unshare", "-rn", "--", sys.executable, str(Path(__file__).resolve()), str(out)], env=env)
    sys.exit(code)


if __name__ == "__main__":
    main()
