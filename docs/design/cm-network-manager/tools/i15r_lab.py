#!/usr/bin/env python3
"""I15-R.T03.a: лабораторный протокол — когда и где действуют controls браузера.

Что проверяется для каждого браузера и механизма:
  * заголовки самого первого запроса (до того, как страница что-то выполнила);
  * значения в окне, в dedicated Worker и в новой вкладке (window.open);
  * холодный перезапуск с тем же профилем: тем же способом запуска и «голым» запуском.

Controls: User-Agent, язык (de-DE), часовой пояс (Asia/Tokyo).

Изоляция:
  * всё выполняется внутри `unshare -rn` — у браузера есть только lo, внешней сети нет;
  * каждый запуск получает временный HOME и временный профиль — профиль пользователя не трогается;
  * Chromium внутри user namespace запускается с --no-sandbox; это допустимо только в лаборатории.

Запуск:
  python3 docs/design/cm-network-manager/tools/i15r_lab.py OUT_DIR
Результат: OUT_DIR/runs.json (сырые наблюдения) и OUT_DIR/observations.json (таблица применимости).
"""

import json
import os
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
UA = "Mozilla/5.0 (X11; Linux x86_64) CMLab/1.0"
LANG = "de-DE"
# Стратегия local без подмены UA: так настроен браузер у жителя страны выхода.
LANG_LIST = "de-DE,de,en-US,en"
POSIX_LOCALE = "de_DE.UTF-8"
TZ = "Asia/Tokyo"
TZ_OFFSET = -540  # getTimezoneOffset() для Asia/Tokyo, минуты
WAIT_S = 20

SNAP = """
function snap(ctx){
  const o = {ctx, ua: navigator.userAgent, lang: navigator.language,
    langs: Array.from(navigator.languages || []),
    tz: Intl.DateTimeFormat().resolvedOptions().timeZone,
    off: new Date(2026, 0, 15).getTimezoneOffset(),
    locale: Intl.DateTimeFormat().resolvedOptions().locale,
    webdriver: self.navigator.webdriver === true};
  if (self.navigator.userAgentData) {
    o.uad = {brands: navigator.userAgentData.brands.map(b => b.brand + '/' + b.version),
             platform: navigator.userAgentData.platform};
  }
  return o;
}
function report(ctx){ return fetch('/report', {method: 'POST', body: JSON.stringify(snap(ctx))}); }
"""

PAGE = """<!doctype html><meta charset="utf-8"><title>lab</title><script>%s
report(%s).then(() => {
  if (%s) {
    const w = new Worker('/worker.js');
    window.open('/tab', '_blank');
  }
});
</script>"""

WORKER = SNAP + "report('worker');"


class Lab:
    def __init__(self):
        self.lock = threading.Lock()
        self.events = []

    def reset(self):
        with self.lock:
            self.events = []

    def add(self, event):
        with self.lock:
            event["seq"] = len(self.events)
            self.events.append(event)

    def reports(self):
        with self.lock:
            return [e for e in self.events if e["kind"] == "report"]


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

    def send(self, body, kind):
        data = body.encode()
        self.send_response(200)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(data)))
        # Client Hints, которые сервер вправе запросить у Chromium.
        self.send_header("Accept-CH", "Sec-CH-UA-Platform-Version, Sec-CH-UA-Full-Version-List")
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        LAB.add({"kind": "request", "path": self.path, "headers": self.headers_of_interest()})
        if self.path == "/":
            self.send(PAGE % (SNAP, "'window'", "true"), "text/html; charset=utf-8")
        elif self.path == "/tab":
            self.send(PAGE % (SNAP, "'tab'", "false"), "text/html; charset=utf-8")
        elif self.path == "/worker.js":
            self.send(WORKER, "text/javascript")
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(length) or b"{}")
        LAB.add({"kind": "report", "headers": self.headers_of_interest(), "values": body})
        self.send_response(204)
        self.end_headers()


def chrome_binary(name):
    paths = {
        "chrome": shutil.which("google-chrome-stable"),
        "brave": shutil.which("brave"),
    }
    return paths.get(name)


def launch(browser, profile, home, controls, mechanism="flags"):
    env = {
        "HOME": str(home),
        "XDG_CONFIG_HOME": str(home / ".config"),
        "XDG_CACHE_HOME": str(home / ".cache"),
        "PATH": os.environ.get("PATH", "/usr/bin"),
        "LANG": "C.UTF-8",
    }
    if controls:
        env["TZ"] = TZ
        if mechanism == "env-local":
            env["LANG"] = POSIX_LOCALE
    url = f"http://127.0.0.1:{PORT}/"
    if browser in ("chrome", "brave"):
        cmd = [
            chrome_binary(browser),
            "--headless=new",
            "--no-sandbox",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-popup-blocking",
            "--disable-gpu",
            f"--user-data-dir={profile}",
        ]
        if controls and mechanism == "env-local":
            cmd += [f"--lang={LANG}", f"--accept-lang={LANG_LIST}"]
        elif controls:
            cmd += [f"--user-agent={UA}", f"--lang={LANG}", f"--accept-lang={LANG}"]
        cmd.append(url)
    else:
        cmd = [shutil.which("librewolf"), "--headless", "--no-remote", "--profile", str(profile), url]
    return subprocess.Popen(
        cmd,
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )


def firefox_prefs(profile, rfp):
    """Controls для Firefox-движка — prefs в user.js профиля: они переживают перезапуск."""
    prefs = {
        "general.useragent.override": UA,
        "intl.accept_languages": LANG,
        "intl.locale.requested": LANG,
        "privacy.resistFingerprinting": rfp,
        "dom.disable_open_during_load": False,
        "browser.shell.checkDefaultBrowser": False,
        "datareporting.policy.dataSubmissionEnabled": False,
        "app.update.enabled": False,
    }
    lines = [f'user_pref("{k}", {json.dumps(v)});' for k, v in prefs.items()]
    (profile / "user.js").write_text("\n".join(lines) + "\n")


def run_once(browser, profile, home, controls, mechanism="flags"):
    LAB.reset()
    proc = launch(browser, profile, home, controls, mechanism)
    deadline = time.time() + WAIT_S
    while time.time() < deadline:
        contexts = {r["values"].get("ctx") for r in LAB.reports()}
        if {"window", "worker", "tab"} <= contexts:
            time.sleep(0.5)
            break
        time.sleep(0.2)
    try:
        os.killpg(proc.pid, signal.SIGTERM)
        proc.wait(timeout=5)
    except (ProcessLookupError, subprocess.TimeoutExpired):
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    with LAB.lock:
        return list(LAB.events)


def verdict(events):
    """Для каждой точки наблюдения: применён ли каждый control."""
    first = next((e for e in events if e["kind"] == "request"), None)
    out = {}
    if first:
        h = first["headers"]
        out["first_request"] = {
            "ua": UA in h.get("user-agent", ""),
            "lang": h.get("accept-language", "").startswith(LANG),
            "client_hints": {k: v for k, v in h.items() if k.startswith("sec-ch-ua")},
        }
    for ctx in ("window", "worker", "tab"):
        rep = next((e for e in events if e["kind"] == "report" and e["values"].get("ctx") == ctx), None)
        if rep is None:
            out[ctx] = None
            continue
        v = rep["values"]
        out[ctx] = {
            "ua": UA in v.get("ua", ""),
            "ua_untouched": "CMLab" not in v.get("ua", ""),
            "intl_locale_matches": v.get("locale", "").split("-")[0] == LANG.split("-")[0],
            "webdriver": v.get("webdriver") is True,
            "lang": v.get("lang", "").startswith(LANG),
            "tz": v.get("tz") == TZ,
            "offset": v.get("off") == TZ_OFFSET,
            "observed": {k: v.get(k) for k in ("ua", "lang", "tz", "off", "locale", "uad")},
            "request_ua": UA in rep["headers"].get("user-agent", ""),
        }
    for path, name in (("/worker.js", "worker_script_request"), ("/tab", "tab_request")):
        req = next((e for e in events if e["kind"] == "request" and e["path"] == path), None)
        if req:
            out[name] = {
                "ua": UA in req["headers"].get("user-agent", ""),
                "lang": req["headers"].get("accept-language", "").startswith(LANG),
            }
    return out


def version(browser):
    binary = chrome_binary(browser) if browser in ("chrome", "brave") else shutil.which("librewolf")
    if not binary:
        return None
    try:
        return subprocess.run([binary, "--version"], capture_output=True, text=True, timeout=20).stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        return None


def scenarios():
    """(браузер, механизм, rfp, [(шаг, controls)])."""
    out = []
    steps = [
        ("first-launch", True),
        ("cold-restart-same-launcher", True),
        ("cold-restart-plain", False),
    ]
    for browser in ("chrome", "brave"):
        if chrome_binary(browser):
            out.append((browser, "env-local", None, steps))
    for browser in ("chrome", "brave"):
        if chrome_binary(browser):
            out.append((browser, "cli-flags+TZ-env", None, [
                ("first-launch", True),
                ("cold-restart-same-launcher", True),
                ("cold-restart-plain", False),
            ]))
    if shutil.which("librewolf"):
        for rfp in (True, False):
            out.append(("librewolf", "user.js-prefs+TZ-env", rfp, [
                ("first-launch", True),
                ("cold-restart-same-launcher", True),
                ("cold-restart-plain", False),
            ]))
    return out


def inner(out_dir):
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
    server = ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    runs = []
    for browser, mechanism, rfp, steps in scenarios():
        root = Path(tempfile.mkdtemp(prefix=f"i15r-{browser}-"))
        try:
            home = root / "home"
            profile = root / "profile"
            home.mkdir()
            profile.mkdir()
            if browser == "librewolf":
                firefox_prefs(profile, rfp)
            for step, controls in steps:
                events = run_once(browser, profile, home, controls, mechanism)
                runs.append({
                    "browser": browser,
                    "version": version(browser),
                    "mechanism": mechanism,
                    "resist_fingerprinting": rfp,
                    "step": step,
                    "controls_from_launcher": controls,
                    "verdict": verdict(events),
                    "events": events,
                })
                print(f"{browser} {mechanism} rfp={rfp} {step}: {len([e for e in events if e['kind']=='report'])} reports", flush=True)
        finally:
            shutil.rmtree(root, ignore_errors=True)
    server.shutdown()
    (out_dir / "runs.json").write_text(json.dumps(runs, ensure_ascii=False, indent=2) + "\n")
    summary = [{k: r[k] for k in ("browser", "version", "mechanism", "resist_fingerprinting", "step", "controls_from_launcher", "verdict")} for r in runs]
    for row in summary:
        for ctx in ("window", "worker", "tab"):
            if row["verdict"].get(ctx):
                row["verdict"][ctx].pop("observed", None)
    (out_dir / "observations.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n")


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(2)
    out_dir = Path(sys.argv[1]).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    if os.environ.get("CM_I15R_INNER") == "1":
        inner(out_dir)
        return
    env = dict(os.environ, CM_I15R_INNER="1")
    code = subprocess.call(["unshare", "-rn", "--", sys.executable, __file__, str(out_dir)], env=env)
    sys.exit(code)


if __name__ == "__main__":
    main()
