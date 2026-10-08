#!/usr/bin/env python3
"""I15-R: согласованная подмена через CDP — проверка того же протокола, что i15r_lab.py.

Механизм: Chromium с `--remote-debugging-pipe` (fd 3 — в браузер, fd 4 — из браузера,
сообщения JSON с завершающим NUL). Браузер стартует на about:blank. Через
`Target.setAutoAttach(waitForDebuggerOnStart)` каждая новая цель — страница, вкладка,
dedicated Worker — останавливается до выполнения. В неё отправляются:
  * Emulation.setUserAgentOverride: UA, Accept-Language и userAgentMetadata (Client Hints);
  * Emulation.setTimezoneOverride и Emulation.setLocaleOverride;
затем Runtime.runIfWaitingForDebugger. Только после этого создаётся вкладка с URL лаборатории.

Маркеры, по которым видно применение: `CMLab` в UA, platformVersion `6.66.0`.

Запуск: python3 docs/design/cm-network-manager/tools/i15r_cdp_lab.py OUT_DIR
Результат: OUT_DIR/cdp-runs.json и OUT_DIR/cdp-observations.json.
"""

import json
import os
import select
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import i15r_lab as lab  # noqa: E402

UA_CDP = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36 CMLab/1.0"
PLATFORM_VERSION = "6.66.0"

# Высокоэнтропийные подсказки снимаются отдельно: окно и Worker.
SNAP_HE = """
async function snapHe(ctx){
  const o = snap(ctx);
  if (self.navigator.userAgentData) {
    try {
      const he = await navigator.userAgentData.getHighEntropyValues(['platformVersion', 'fullVersionList']);
      o.platformVersion = he.platformVersion;
      o.fullVersionList = (he.fullVersionList || []).map(b => b.brand + '/' + b.version);
    } catch (e) { o.he_error = String(e); }
  }
  return o;
}
function report(ctx){ return snapHe(ctx).then(o => fetch('/report', {method: 'POST', body: JSON.stringify(o)})); }
"""


class Pipe:
    def __init__(self, proc, write_fd, read_fd):
        self.proc = proc
        self.write = os.fdopen(write_fd, "wb", buffering=0)
        self.read_fd = read_fd
        self.buffer = b""
        self.next_id = 0

    def send(self, method, params=None, session=None):
        self.next_id += 1
        message = {"id": self.next_id, "method": method, "params": params or {}}
        if session:
            message["sessionId"] = session
        self.write.write(json.dumps(message).encode() + b"\0")
        return self.next_id

    def messages(self, timeout):
        ready, _, _ = select.select([self.read_fd], [], [], timeout)
        if not ready:
            return []
        chunk = os.read(self.read_fd, 1 << 20)
        if not chunk:
            raise EOFError
        self.buffer += chunk
        out = []
        while b"\0" in self.buffer:
            raw, self.buffer = self.buffer.split(b"\0", 1)
            out.append(json.loads(raw))
        return out


def metadata():
    brands = [
        {"brand": "Google Chrome", "version": "155"},
        {"brand": "Chromium", "version": "155"},
        {"brand": "Not(A:Brand", "version": "24"},
    ]
    full = [
        {"brand": "Google Chrome", "version": "155.0.8059.39"},
        {"brand": "Chromium", "version": "155.0.8059.39"},
        {"brand": "Not(A:Brand", "version": "24.0.0.0"},
    ]
    return {
        "brands": brands,
        "fullVersionList": full,
        "platform": "Linux",
        "platformVersion": PLATFORM_VERSION,
        "architecture": "x86",
        "model": "",
        "mobile": False,
        "bitness": "64",
        "wow64": False,
    }


def configure(pipe, session, kind):
    """Controls для одной остановленной цели, затем запуск."""
    if kind in ("page", "worker"):
        pipe.send("Network.enable", session=session)
        pipe.send(
            "Network.setUserAgentOverride",
            {"userAgent": UA_CDP, "acceptLanguage": lab.LANG, "userAgentMetadata": metadata()},
            session=session,
        )
    if kind == "page":
        pipe.send(
            "Emulation.setUserAgentOverride",
            {"userAgent": UA_CDP, "acceptLanguage": lab.LANG, "userAgentMetadata": metadata()},
            session=session,
        )
        pipe.send("Emulation.setTimezoneOverride", {"timezoneId": lab.TZ}, session=session)
        pipe.send("Emulation.setLocaleOverride", {"locale": lab.LANG}, session=session)
    pipe.send(
        "Target.setAutoAttach",
        {"autoAttach": True, "waitForDebuggerOnStart": True, "flatten": True},
        session=session,
    )
    pipe.send("Runtime.runIfWaitingForDebugger", session=session)


def run(binary, profile, home, worker_tz, extra):
    lab.LAB.reset()
    r3, w3 = os.pipe()  # в браузер
    r4, w4 = os.pipe()  # из браузера
    env = {
        "HOME": str(home),
        "XDG_CONFIG_HOME": str(home / ".config"),
        "XDG_CACHE_HOME": str(home / ".cache"),
        "PATH": os.environ.get("PATH", "/usr/bin"),
        "LANG": "C.UTF-8",
    }
    if worker_tz:
        # Вариант «CDP + TZ лаунчера»: часовой пояс процесса для целей, куда Emulation не доходит.
        env["TZ"] = lab.TZ

    # Браузер ждёт pipe именно на fd 3 и 4; sh переставляет их перед exec.
    high_in, high_out = 100, 101
    os.dup2(r3, high_in)
    os.dup2(w4, high_out)
    os.close(r3)
    os.close(w4)
    proc = subprocess.Popen(
        [
            "sh",
            "-c",
            'exec "$@" 3<&100 4>&101 100<&- 101>&-',
            "sh",
            binary,
            "--headless=new",
            "--no-sandbox",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-popup-blocking",
            "--disable-gpu",
            "--remote-debugging-pipe",
            f"--user-data-dir={profile}",
            *extra,
            "about:blank",
        ],
        env=env,
        pass_fds=(high_in, high_out),
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    os.close(high_in)
    os.close(high_out)
    pipe = Pipe(proc, w3, r4)
    pipe.send(
        "Target.setAutoAttach",
        {"autoAttach": True, "waitForDebuggerOnStart": True, "flatten": True},
    )
    pipe.send("Target.setDiscoverTargets", {"discover": True})
    # Вкладка создаётся пустой; URL открывается только после того, как в её
    # сессию ушли все overrides. Иначе навигация успевает уйти с настоящими заголовками.
    create_id = None
    created_target = None
    sessions = {}
    navigated = False
    attached = []
    started = time.time()
    deadline = started + lab.WAIT_S
    try:
        while time.time() < deadline:
            for message in pipe.messages(0.2):
                if message.get("method") == "Target.attachedToTarget":
                    params = message["params"]
                    kind = params["targetInfo"]["type"]
                    attached.append(kind)
                    sessions[params["targetInfo"]["targetId"]] = params["sessionId"]
                    configure(pipe, params["sessionId"], kind)
                elif create_id is not None and message.get("id") == create_id:
                    created_target = message.get("result", {}).get("targetId")
            if create_id is None and time.time() > started + 1.0:
                create_id = pipe.send("Target.createTarget", {"url": "about:blank"})
            if not navigated and created_target in sessions:
                pipe.send(
                    "Page.navigate",
                    {"url": f"http://127.0.0.1:{lab.PORT}/"},
                    session=sessions[created_target],
                )
                navigated = True
            contexts = {r["values"].get("ctx") for r in lab.LAB.reports()}
            if {"window", "worker", "tab"} <= contexts:
                break
        time.sleep(0.5)
    except EOFError:
        pass
    finally:
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        proc.wait()
    with lab.LAB.lock:
        return list(lab.LAB.events), attached


def verdict(events):
    out = lab.verdict(events)
    first = next((e for e in events if e["kind"] == "request"), None)
    if first:
        hints = first["headers"]
        out["first_request"]["client_hints_consistent"] = '"Google Chrome";v="155"' in hints.get("sec-ch-ua", "")
    for ctx in ("window", "worker", "tab"):
        rep = next((e for e in events if e["kind"] == "report" and e["values"].get("ctx") == ctx), None)
        if rep and out.get(ctx):
            out[ctx]["platform_version_marker"] = rep["values"].get("platformVersion") == PLATFORM_VERSION
            out[ctx]["webdriver_visible"] = rep["values"].get("webdriver") is True
            out[ctx]["observed"]["platformVersion"] = rep["values"].get("platformVersion")
            out[ctx]["observed"]["fullVersionList"] = rep["values"].get("fullVersionList")
            out[ctx]["request_client_hints"] = {k: v for k, v in rep["headers"].items() if k.startswith("sec-ch-ua")}
    return out


def inner(out_dir):
    subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
    lab.UA = UA_CDP
    lab.SNAP = lab.SNAP.replace("function report(ctx)", "function report_plain(ctx)") + SNAP_HE
    lab.WORKER = lab.SNAP + "report('worker');"
    from http.server import ThreadingHTTPServer

    server = ThreadingHTTPServer(("127.0.0.1", lab.PORT), lab.Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    runs = []
    for browser in ("chrome", "brave"):
        binary = lab.chrome_binary(browser)
        if not binary:
            continue
        variants = (
            ("cdp-pipe", False, []),
            ("cdp-pipe+TZ-env", True, []),
            ("cdp-pipe+AutomationControlled-off", False, ["--disable-blink-features=AutomationControlled"]),
        )
        for mechanism, worker_tz, extra in variants:
            root = Path(tempfile.mkdtemp(prefix=f"i15r-cdp-{browser}-"))
            try:
                (root / "home").mkdir()
                (root / "profile").mkdir()
                events, attached = run(binary, root / "profile", root / "home", worker_tz, extra)
                runs.append({
                    "browser": browser,
                    "version": lab.version(browser),
                    "mechanism": mechanism,
                    "attached_targets": attached,
                    "verdict": verdict(events),
                    "events": events,
                })
                print(f"{browser} {mechanism}: attached {attached}", flush=True)
            finally:
                shutil.rmtree(root, ignore_errors=True)
    server.shutdown()
    (out_dir / "cdp-runs.json").write_text(json.dumps(runs, ensure_ascii=False, indent=2) + "\n")
    table = [{k: r[k] for k in ("browser", "version", "mechanism", "attached_targets", "verdict")} for r in runs]
    (out_dir / "cdp-observations.json").write_text(json.dumps(table, ensure_ascii=False, indent=2) + "\n")


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
    sys.exit(subprocess.call(["unshare", "-rn", "--", sys.executable, __file__, str(out_dir)], env=env))


if __name__ == "__main__":
    main()
