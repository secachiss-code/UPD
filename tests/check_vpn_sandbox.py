"""Run VPN preparation with only VPN home writable (requires bubblewrap).

Usage: python3 tests/check_vpn_sandbox.py /absolute/path/to/upd
Uses a local profile and a fake core; does not start a VPN or use the network.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def check(binary):
    with tempfile.TemporaryDirectory(prefix="upd-vpn-readonly-") as directory:
        base = Path(directory)
        home, etc, state = base / "vpn", base / "etc", base / "state"
        for path in (home / "profiles", home / "bin", etc, state):
            path.mkdir(parents=True, exist_ok=True)
        conf = base / "upd.conf"
        conf.write_text("")
        (etc / "subs.json").write_text(json.dumps({"active": "fixture", "list": [{
            "id": "fixture", "name": "fixture", "url": "https://example.com/fixture", "kind": "clash",
        }]}))
        (home / "profiles/fixture.yaml").write_text(
            "proxies:\n  - {name: fixture, type: ss, server: 192.0.2.1, port: 443, "
            "cipher: aes-128-gcm, password: fixture}\nproxy-groups:\n"
            "  - {name: Proxy, type: select, proxies: [fixture]}\nrules:\n  - MATCH,Proxy\n"
        )
        core = home / "bin/mihomo"
        core.write_text('#!/bin/sh\nif [ "$1" = "-v" ]; then echo "mihomo v1.19.0"; exit 0; fi\n'
                        '[ "$1" = "-t" ] && [ -s "$5" ]\n')
        core.chmod(0o700)
        env = dict(os.environ, UPD_STATE_DIR=str(state), UPD_VPN_HOME=str(home),
                   UPD_VPN_ETC=str(etc), UPD_CONF=str(conf))
        result = subprocess.run([
            "bwrap", "--unshare-user", "--uid", "0", "--gid", "0", "--unshare-pid",
            "--unshare-net", "--ro-bind", "/", "/", "--bind", str(home), str(home),
            "--dev", "/dev", "--proc", "/proc", str(Path(binary).resolve()), "vpn", "prepare",
        ], env=env, text=True, capture_output=True, timeout=20)
        assert result.returncode == 0, result.stdout + result.stderr
        assert (home / "config.yaml").is_file()
        assert (home / ".vpn-config.lock").is_file()
        assert not list(state.iterdir())
        assert conf.read_text() == ""
        assert not (etc / "rules.txt").exists()
        print("PASS: VPN preparation writes only inside VPN home")


if __name__ == "__main__":
    check(sys.argv[1])
