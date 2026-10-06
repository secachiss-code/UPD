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
rustfmt --edition 2024 --check src/common/probe.rs cosmic/src/jobs.rs cosmic/src/launch.rs cosmic/src/notifications.rs cosmic/src/tui_launch.rs tests/audit_contracts.rs
if [ "${CM_SKIP_COSMIC:-0}" != 1 ]; then
  python3 tests/test_tui_launcher.py cosmic/target/x86_64-unknown-linux-gnu/debug/cm-cosmic
fi

NEW_MODULE_FMT=$(find src/sources src/profiles src/migration -name '*.rs' -print)
# shellcheck disable=SC2086
rustfmt --edition 2024 --check $NEW_MODULE_FMT

check_new_module_clippy() {
  set +e
  output=$(cargo clippy --locked --offline --all-targets --message-format=short 2>&1)
  status=$?
  set -e
  # Short format: "src/sources/x.rs:12:5: warning: ..." (also "error:").
  pattern='^src/(sources|profiles|migration)/[^:]+:[0-9]+:[0-9]+: (warning|error)'
  if printf '%s\n' "$output" | grep -Eq "$pattern"; then
    printf '%s\n' "$output" | grep -E "$pattern" >&2 || true
    echo "clippy: warnings in src/sources, src/profiles, or src/migration" >&2
    exit 1
  fi
  if [ "$status" -ne 0 ]; then
    printf '%s\n' "$output" >&2
    exit "$status"
  fi
}
check_new_module_clippy
