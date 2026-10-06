"""I03.T05.d: release build and its manifest, with a reproducibility rebuild.

Runs `build.sh` offline (musl CLI + COSMIC), records the git revision, tree hash,
Cargo.lock sha256, rustc and binary hashes, then rebuilds the CLI in a separate
target directory and compares the hash. Only the summary goes to git (D8); build
logs stay under dist/.

Usage: python3 tests/i03_build_manifest.py OUT_DIR
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
CLI_TARGET = "x86_64-unknown-linux-musl"
SOURCE_GLOBS = ["Cargo.toml", "Cargo.lock", ".cargo/config.toml", "build.sh", "src", "cosmic/Cargo.toml", "cosmic/Cargo.lock", "cosmic/src"]


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True, text=True).stdout.strip()


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, CARGO_NET_OFFLINE="true")
    env.pop("CARGO_TARGET_DIR", None)  # build.sh copies from ./target
    started = time.time()
    build = subprocess.run(["sh", "build.sh"], cwd=ROOT, env=env, capture_output=True, text=True)
    if build.returncode != 0:
        sys.stderr.write(build.stdout + build.stderr)
        sys.exit("build.sh failed")
    cli = ROOT / "dist/cm-linux-amd64"
    gui = ROOT / "dist/cm-cosmic-linux-amd64"
    tracked = git("ls-files", "--", *SOURCE_GLOBS).splitlines()
    manifest = {
        "status": "BUILD_PASS",
        "task": "I03.T05.d",
        "git_revision": git("rev-parse", "HEAD"),
        "git_tree": git("rev-parse", "HEAD^{tree}"),
        "dirty_tree": bool(git("status", "--porcelain", "--", *SOURCE_GLOBS)),
        "rustc": subprocess.run(["rustc", "--version"], capture_output=True, text=True, check=True).stdout.strip(),
        "cargo_lock_sha256": digest(ROOT / "Cargo.lock"),
        "cli_target": CLI_TARGET,
        "binary": "dist/cm-linux-amd64",
        "binary_sha256": digest(cli),
        "version": subprocess.run([str(cli), "--version"], capture_output=True, text=True, check=True).stdout.strip(),
        "gui_binary": "dist/cm-cosmic-linux-amd64" if gui.exists() else None,
        "gui_binary_sha256": digest(gui) if gui.exists() else None,
        "source_sha256": {path: digest(ROOT / path) for path in tracked},
        "started": started,
        "runtime_tests": "NOT_RUN",
        "installation_services_live_network": "NOT_RUN",
    }
    # Reproducibility: the same sources in a fresh target directory.
    with tempfile.TemporaryDirectory(dir=Path.home() / ".cache", prefix="cm-repro-") as target:
        rebuild = subprocess.run(
            ["cargo", "build", "--release", "--locked", "--offline", "--target", CLI_TARGET],
            cwd=ROOT, env=dict(env, CARGO_TARGET_DIR=target), capture_output=True, text=True,
        )
        if rebuild.returncode != 0:
            manifest["reproducible_cli"] = {"status": "REBUILD_FAILED"}
        else:
            again = digest(Path(target) / CLI_TARGET / "release/cm")
            manifest["reproducible_cli"] = {
                "status": "MATCH" if again == manifest["binary_sha256"] else "DIFFERS",
                "rebuild_sha256": again,
                "rebuild_target_dir": "fresh temporary directory under ~/.cache",
            }
    manifest["finished"] = time.time()
    (out / "summary.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(out / "summary.json", manifest["reproducible_cli"]["status"])


if __name__ == "__main__":
    main()
