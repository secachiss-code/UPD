"""Exercise the compiled launcher with a fake terminal; no VPN or elevation."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time


def check(binary):
    if os.geteuid() == 0:
        print("SKIP: the application launcher intentionally refuses root")
        return
    with tempfile.TemporaryDirectory(prefix="cm-launcher-") as directory:
        root = Path(directory)
        marker = root / "launched.jsonl"
        terminal = root / "cosmic-term"
        terminal.write_text(
            f"#!{sys.executable}\n"
            "import json, os, sys, time\n"
            "with open(os.environ['CM_LAUNCH_MARKER'], 'a') as output:\n"
            " output.write(json.dumps({'pid': os.getpid(), 'args': sys.argv[1:]}) + '\\n')\n"
            " output.flush()\n"
            "time.sleep(30)\n"
        )
        terminal.chmod(0o700)
        env = dict(os.environ, PATH=str(root), XDG_RUNTIME_DIR=str(root),
                   CM_LAUNCH_MARKER=str(marker), CM_CONF=str(root / "cm.conf"))

        def launches():
            return [json.loads(line) for line in marker.read_text().splitlines()] if marker.exists() else []

        def run(*args):
            subprocess.run([str(Path(binary).resolve()), *args], env=env, check=True,
                           capture_output=True, timeout=5)

        def wait_for(count):
            deadline = time.monotonic() + 3
            while len(launches()) < count and time.monotonic() < deadline:
                time.sleep(.01)
            assert len(launches()) == count, launches()

        def closed(pid):
            try:
                state = Path(f"/proc/{pid}/stat").read_text().rsplit(')', 1)[1].split()[0]
                return state in ("Z", "X")
            except FileNotFoundError:
                return True

        try:
            run()
            wait_for(1)
            assert launches()[0]["args"][0] == "--"
            assert launches()[0]["args"][-1] == "tui"
            run("--page", "vpn")
            run("--page", "updates", "--run", "check")
            assert len(launches()) == 1
            run("--new-window")
            wait_for(2)
            newest = launches()[1]["pid"]
            os.kill(newest, signal.SIGTERM)
            deadline = time.monotonic() + 3
            while not closed(newest) and time.monotonic() < deadline:
                time.sleep(.01)
            assert closed(newest)
            run()
            assert len(launches()) == 2, "older live window should be reused"
            first = launches()[0]["pid"]
            os.kill(first, signal.SIGTERM)
            deadline = time.monotonic() + 3
            while not closed(first) and time.monotonic() < deadline:
                time.sleep(.01)
            assert closed(first)
            run()
            wait_for(3)
        finally:
            for terminal in launches():
                try:
                    os.kill(terminal["pid"], signal.SIGTERM)
                except ProcessLookupError:
                    pass
        print("PASS: TUI entry, duplicate suppression, new window and stale process recovery")


if __name__ == "__main__":
    check(sys.argv[1])
