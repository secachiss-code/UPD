# I01 read-only migration plan preflight

Status: implementation prepared; added regressions `NOT_RUN`.

`src/migration/transaction.rs` exposes `Executor::validate_plan`, which contains
the executor's existing filesystem-only validation loop. It uses snapshots
with backup storage disabled and creates no journal, blobs, locks, or other
filesystem state. `execute` keeps its journal-phase check first, then calls
this validator at the same point where the loop previously ran. This preserves
the committed-journal verification path.

`src/migration/manual.rs` invokes the shared validator before returning a plan.
The manual plan now rejects nested source symlinks, special or unsafe writable
entries, overlaps, and existing target/staging hazards before reporting the
plan. `src/main.rs` also runs the fixed production `/proc` audit immediately
after the manual plan is built and before printing its summary or applying it.
The existing locked and quiesced audits remain. The process-audit policy from
the preceding code change was not modified.

`tests/audit_i01_migration.rs` contains two added regressions: a clean minimal
manual layout validates without writes, while nested symlink and group-writable
data entries make `manual::plan` fail without creating a journal or changing
the synthetic credential/tree sentinels; a direct successful call to
`Executor::validate_plan` also leaves the fixture unchanged and journal absent.
The fixture reads the pinned legacy CLI bytes but never executes them.

Tests, build, application binary, services, package managers, and network were
not run. Real validation remains deferred until installation of the built
binary. Earlier evidence files and hashes were not rewritten or relabeled as
passing this change.
