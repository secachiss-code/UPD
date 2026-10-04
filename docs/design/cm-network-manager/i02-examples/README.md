# I02 schema-1 examples

All JSON files are complete `GraphSnapshot` values built from synthetic IDs, executable labels, timestamps, and credential metadata. They are model examples, not initialized or ready-to-open Store directories. Credential payload bytes are deliberately absent; private blobs are created only through the Store API, while `digest_sha256` and `size_bytes` describe metadata only.

- `two_sources_three_apps.json` is the baseline: two Sources, three applications, one shared group, one own tunnel, two active group Sessions, and Host mode off.
- `host_and_group.json` enables the host-owned tunnel on `profile-b` / `source-b` while the application group remains on `profile-a` / `source-a`.
- `source_generation2_net_blocked.json` is the single-revision transition from the baseline. Source A advances to generation 2 with a changed definition and a new current Node ID. The prior Node and active Session pins remain in history; only NET evidence becomes blocked with fresh timestamps.

`tests/audit_i02_examples.rs` contains deserialize, graph validation, and transition assertions for these files. Runtime status: `NOT_RUN`; these tests are compile-checked only until the installation-time test gate.
