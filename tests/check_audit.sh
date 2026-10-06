#!/bin/sh
# Local regression gates only; no installation, privileged host mutation or VM acceptance.
set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cargo build --locked --offline --target x86_64-unknown-linux-musl
cargo test --locked --offline --target x86_64-unknown-linux-musl
# CM_SKIP_COSMIC=1: CI job without the COSMIC/Wayland system libraries (D9); COSMIC runs in its own job.
if [ "${CM_SKIP_COSMIC:-0}" != 1 ]; then
  (cd cosmic && cargo build --locked --offline --target x86_64-unknown-linux-gnu && cargo test --locked --offline --target x86_64-unknown-linux-gnu)
fi
python3 tests/test_build_artifacts.py
sh -n build.sh package.sh
# New standalone modules; existing dense style is deliberately preserved.
rustfmt --edition 2024 --check src/common/probe.rs src/common/sys.rs cosmic/src/jobs.rs cosmic/src/launch.rs cosmic/src/notifications.rs cosmic/src/tui_launch.rs tests/audit_contracts.rs tests/audit_v02_source.rs src/core/mihomo/mod.rs src/core/mihomo/config.rs src/core/mihomo/validate.rs src/core/legacy_host.rs tests/audit_i04_mihomo_config.rs tests/audit_h11_core.rs tests/audit_i04_legacy_host.rs
if [ "${CM_SKIP_COSMIC:-0}" != 1 ]; then
  python3 tests/test_tui_launcher.py cosmic/target/x86_64-unknown-linux-gnu/debug/cm-cosmic
fi

NEW_MODULE_FMT=$(find src/sources src/profiles src/migration -name '*.rs' -print)
# shellcheck disable=SC2086
rustfmt --edition 2024 --check $NEW_MODULE_FMT

# H.03 made the whole crate clippy-clean; H.09: every unsafe block carries a SAFETY comment.
cargo clippy --locked --offline --all-targets -- -D warnings -D clippy::undocumented_unsafe_blocks
