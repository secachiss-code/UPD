#!/bin/sh
# Local regression gates only; no installation, privileged host mutation or VM acceptance.
set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cargo build --locked --offline --target x86_64-unknown-linux-musl
cargo test --locked --offline --target x86_64-unknown-linux-musl
(cd cosmic && cargo build --locked --offline --target x86_64-unknown-linux-gnu && cargo test --locked --offline --target x86_64-unknown-linux-gnu)
python3 tests/test_build_artifacts.py
sh -n build.sh package.sh
# New standalone modules; existing dense style is deliberately preserved.
rustfmt --edition 2024 --check src/common/probe.rs cosmic/src/jobs.rs cosmic/src/launch.rs cosmic/src/notifications.rs cosmic/src/tui_launch.rs tests/audit_contracts.rs
python3 tests/test_tui_launcher.py cosmic/target/x86_64-unknown-linux-gnu/debug/cm-cosmic
