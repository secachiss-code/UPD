# I01 legacy process quiescence code change

Status: implementation prepared; regression tests `NOT_RUN`.

`src/migration/manual.rs` now includes the owned legacy core at
`/var/lib/upd/vpn/bin/mihomo` in the process audit alongside the pinned CLI and
GUI. The audit compares executable device/inode and checks `/proc/<pid>/exe`
against the exact managed pathname, including its exact ` (deleted)` suffix.
It does not use process names, signal processes, or stop the core. The public
production entrypoint still fixes the paths to `/` and `/proc`; only its
private helper accepts synthetic paths for unit fixtures.

Identity lookup errors other than missing managed files fail closed. For a
missing `/proc/<pid>/exe`, the helper skips only a vanished PID, a zombie
(`stat` state `Z`), or a kernel thread (`stat` flags include `PF_KTHREAD`). It
parses `stat` after the final `)` so spaces and `)` in `comm` do not shift the
state or flags fields. Other missing, unreadable, or malformed process state
fails closed. If the PID disappears while its `stat` is read, the audit also
skips that vanished PID; other `stat` read errors fail closed.

Five private synthetic regression tests were added in the same source file:

- hard-linked legacy core detected by inode while an unrelated same-basename
  executable is accepted;
- exact deleted CLI, GUI, and core paths detected;
- missing executable for a live ordinary process refused;
- missing `stat` for a present PID refused;
- kernel-thread and zombie rows accepted, ordinary and malformed rows refused.

`rustfmt --edition 2021 src/migration/manual.rs` completed. Tests, build, the
application binary, real `/proc`, services, package managers, and network were
not run. These tests are not evidence of host behavior; real checks remain
deferred until the built binary is installed. The Grokbuild simulation and its
older evidence hashes were not edited or relabeled as passing this change.
