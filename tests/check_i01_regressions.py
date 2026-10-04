"""Plan I01 regressions; --run executes local fixtures after installation.

No package building, installation, migration apply, or host service operation.
Real migration/systemd/network acceptance is a separate installation checklist.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MANIFEST = ROOT / "docs/design/cm-network-manager/i01-evidence/coder-t05-build-2026-10-04/summary.json"
CASES = [
    ("migration", ["cargo", "test", "--offline", "--locked", "--test", "audit_i01_migration"]),
    ("process_and_service_guards", ["cargo", "test", "--offline", "--locked", "--lib", "migration::manual::"]),
    ("startup_guard", ["cargo", "test", "--offline", "--locked", "--test", "audit_i01_install_guard", "c17a_"]),
    ("updates_prefetch_install", ["cargo", "test", "--offline", "--locked", "--bin", "cm"]),
    ("configuration_atomic_write", ["cargo", "test", "--offline", "--locked", "--lib", "common::"]),
    ("backend", ["cargo", "test", "--offline", "--locked", "--lib", "backend::"]),
    ("mirrors_prefetch", ["cargo", "test", "--offline", "--locked", "--lib", "mirrors::"]),
    ("snapshots", ["cargo", "test", "--offline", "--locked", "--lib", "extras::"]),
    ("helper", ["cargo", "test", "--offline", "--locked", "--lib", "helper::"]),
    ("tui", ["cargo", "test", "--offline", "--locked", "--bin", "cm", "tui::"]),
]


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main(cases=None, default_manifest=None, description=None):
    selected_cases = CASES if cases is None else cases
    parser = argparse.ArgumentParser(description=description or __doc__)
    parser.add_argument("--run", action="store_true")
    parser.add_argument("--manifest", type=Path, default=default_manifest or DEFAULT_MANIFEST)
    parser.add_argument("--installed-binary", type=Path)
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    if not args.run:
        print(json.dumps({"status": "NOT_RUN", "cases": selected_cases}, indent=2))
        return
    if os.geteuid() == 0:
        parser.error("run local fixtures as an unprivileged user")
    if args.installed_binary is None or args.evidence is None:
        parser.error("--run requires --installed-binary and a new --evidence directory")
    manifest = json.loads(args.manifest.read_text())
    if manifest.get("status") != "BUILD_PASS":
        parser.error("manifest must describe a successful binary build")
    if digest(args.installed_binary) != manifest["binary_sha256"]:
        parser.error("installed binary does not match the build manifest")
    for relative, expected in manifest["source_sha256"].items():
        if digest(ROOT / relative) != expected:
            parser.error(f"source changed since the build: {relative}")
    # A fresh evidence directory preserves every previous failed attempt.
    args.evidence.mkdir(parents=True, exist_ok=False)
    report = {"installed_binary_sha256": digest(args.installed_binary), "cases": []}
    (args.evidence / "summary.json").write_text(json.dumps(report, indent=2))
    for name, argv in selected_cases:
        started = time.time()
        with (args.evidence / f"{name}.stdout").open("wb") as stdout, (args.evidence / f"{name}.stderr").open("wb") as stderr:
            try:
                result = subprocess.run(argv, cwd=ROOT, stdout=stdout, stderr=stderr, timeout=600)
                status = "PASS" if result.returncode == 0 else "FAIL"
                code = result.returncode
            except subprocess.TimeoutExpired:
                status, code = "TIMEOUT", None
        counts = re.findall(r"test result: ok\. (\d+) passed", (args.evidence / f"{name}.stdout").read_text(errors="replace"))
        if status == "PASS" and not any(int(count) > 0 for count in counts):
            status = "INCONCLUSIVE_ZERO_TESTS"
        report["cases"].append({"id": name, "argv": argv, "started": started, "finished": time.time(), "status": status, "exit_code": code})
        (args.evidence / "summary.json").write_text(json.dumps(report, indent=2))
        if status != "PASS":
            raise SystemExit(1)


if __name__ == "__main__":
    main()
