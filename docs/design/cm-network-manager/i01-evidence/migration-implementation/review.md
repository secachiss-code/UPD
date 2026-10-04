# I01 migration implementation: independent fixture review

Review scope: current source and offline fixture execution for the explicitly supported manual UPD 0.2.7 layout. The review did not change product code or invoke migration against the host.

The transaction executor passed 15 integration tests. The fixtures cover the manual migration policy using the pinned `dist/upd-linux-amd64` bytes without executing that binary; config and VPN data bytes, uid and mode; generated replacements and enablement links; committed-plan idempotency; negative old/new and unsafe-path cases; flock contention; observer failures at durable boundaries; child-process termination followed by recovery in a new process; service and reload failures; retry after incomplete rollback; tampered backup rejection; preservation of unrelated target edits; verified partial staging cleanup; refusal to erase unknown journal staging data; and a pre-mutation `locked` veto that leaves service callbacks and source inodes untouched.

Two defects were found during this work and fixed in the implementation before the final passing run:

1. Applying a root regular-file snapshot used `base.join("")`, producing a trailing slash and `ENOTDIR`. The supported manual layout migrates `/etc/upd.conf` as a file, so its first file step failed. The executor now resolves an empty relative node to the base path. The first failure and final passing rerun are preserved in the evidence logs.
2. Recovery deleted any private regular file at the fixed journal temporary path without checking its contents. A synthetic unrelated sentinel demonstrated silent deletion. Journal writes now use the common unique atomic writer and an old fixed-name staging conflict is refused and preserved. The regression verifies the recovery refuses and leaves the sentinel intact.

No remaining P0 counterexample was found in the reviewed fixture scope. The manual policy also preserves enabled-but-inactive VPN state by journaling the `multi-user.target.wants/cm-vpn.service` link and restoring service state through the adapter.

The final checks passed: 15/15 migration tests, 5/5 C17a startup-guard tests, 4/4 language regression tests, 12/12 actual help invocations across six locales, and offline `cm` binary build. Logs and localized-help JSON are in this directory: [migration suite](audit_i01_migration.final.stdout.log), [build](build.final.stderr.log), [C17a](c17a-regression.final.stdout.log), [language regressions](i18n-regression.final.stdout.log), [help results](help-locales.final.json). Earlier failure evidence remains available in [initial migration run](audit_i01_migration.stdout.log), [journal counterexample](audit_i01_migration.review-01.stdout.log), and [root-file counterexample](manual-policy-debug.stdout.log).

The result is fixture-level verification of the supported manual UPD 0.2.7 transaction, not live Linux migration acceptance. `Services` and package ownership are represented by fixtures in the migration tests. No host `systemd` stop/disable/enable/reload/start, package database ownership query, migration apply, helper race, or live UPD operation was run. The pinned UPD binary was read only as input bytes. The root CLI namespace/socket C17a case remains limited by the previously recorded sandbox NETLINK restriction.

Current artifacts and hashes are recorded in [summary.json](summary.json). The separate [initial failure logs](audit_i01_migration.stderr.log) and [retry pass](audit_i01_migration.retry-01.stdout.log) are retained rather than overwritten.
