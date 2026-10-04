# Simulation fixes: compile-only evidence

Status: **COMPILE_PASS**. Runtime tests: **NOT_RUN**.

`manual::Systemd::set` now runs a pure admission guard immediately after
`inspect`. It refuses an observed active `upd-*.service` or
`upd-helper.socket` before issuing any systemctl state-changing command.
Inactive legacy services remain allowed; active legacy timers/path units and
active CM services remain allowed, including CM rollback stops. Three pure
unit regressions cover these cases. No application binary or service was run.

The guard narrows the activation window after the earlier audits; it does not
freeze systemd state atomically. Another actor may activate a legacy service
after `inspect` and before the subsequent systemctl command. Live systemd
behavior remains unverified.

`cargo check --offline --locked --tests` completed with exit status 0. This
compiled the test targets without executing them. Logs:
[stdout](cargo-check.stdout), [stderr](cargo-check.stderr).

Source SHA-256 at this check:

| File | SHA-256 |
| --- | --- |
| `src/main.rs` | `9a63c421e4963d1bfe0940eda76bf743bdc2e07db0fdb6fb64ad81c7ae9f5183` |
| `src/migration/manual.rs` | `fb90270a519b0c261ebd988358b0f27c9b4aadc27cc0f75fe6f5bf9815c6fd56` |
| `src/migration/transaction.rs` | `ae4a94bfb28b858f82499618b9d6b1322b2a867652c652a2b50894843e0570f7` |
| `tests/audit_i01_migration.rs` | `5e9a1e9e39b0f504f2553b4f33cd758779ba204a86ee9286b207b96e8f386a6b` |

Existing `target/x86_64-unknown-linux-musl/debug/cm` remains
`OLD_NOT_REBUILT`; its SHA-256 is unchanged at
`b46c5d9381ddadc93b8a1f2b428ca71f9772ceef814795df2c44e4d179498b3b`.
No earlier evidence files or hashes were edited.
