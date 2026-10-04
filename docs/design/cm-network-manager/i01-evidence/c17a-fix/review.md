# C17a preflight review

Reviewed `src/migration.rs`, the startup call sites in `src/main.rs`, and the focused disposable-root fixtures. The preflight uses a fixed absolute legacy inventory and does not consult `CM_*`/`UPD_*` overrides. `install` is checked independent of privilege; other commands are refused for privileged startup when a legacy/ambiguous entry exists. `main` checks before backend detection/escalation and again after escalation, before `Config::load` and its default save. Help/version return before the guard as intended.

Fixture coverage now includes clean and CM-only privileged roots, legacy config and state-only layouts, unprivileged read-only `lang`, unprivileged `install`, every dispatch command plus unknown, environment overrides, and an unsafe symlink ancestor. Focused C17a cases pass.

Limits: root behavior is exercised through the production preflight function with disposable roots, not a real euid-0 process, because required namespace creation is blocked in this environment. The W0 K0–K6 characterization remains BLOCKED/NOT_RUN; no migration acceptance or I01 completion follows. A separate existing helper-socket fixture in the full suite fails at UnixListener::bind with sandbox `Operation not permitted`.
