# I03.T03: private source artifacts and durable provenance

The typed artifact pipeline is implemented in `src/sources/artifact.rs`. It consumes an
accepted `Negotiated<T>` result or an explicit local body plus definitions that a trusted
classifier has already produced. It does not parse arbitrary URI lists, base64 subscriptions,
Mihomo YAML/JSON, provider declarations, or manual CLI input; those remain T04 work. These
records are therefore typed inputs, not a claim that an arbitrary native configuration has
passed a complete core schema validator.

`create_source` publishes a generation-1 Source, fresh Node IDs, the safe graph provenance,
and one immutable private artifact blob through `Store::commit`. `update_source` compares both
graph revision and source generation through `Store::update_source`. It retains Node IDs and
the blob when the raw body and actual winning User-Agent are unchanged. A User-Agent-only
refresh writes a new Source artifact while the immutable Nodes keep their previous artifact
references. A changed body creates generation+1 and fresh Node IDs; archived Nodes resolve
their complete definitions through their own generation's credential reference.

The private artifact stores raw body bytes, the actual winning User-Agent when negotiated,
full typed node definitions, constrained global defaults, format, source/generation, the
SHA-256 digests, and the exact pinned core version and full commit. It excludes the changing
accepted timestamp so a same-body/same-UA refresh can reuse the blob. The graph stores only
the safe provenance subset: format, accepted time, body digest, core pin, origin, and optional
User-Agent digest. Raw body, raw User-Agent, endpoint, and full definitions do not enter graph
records or status DTOs. `Debug` output redacts bodies, UAs, defaults, and definitions; explicit
getters expose artifact material to trusted consumers.

The raw body limit is 8 MiB, the serialized artifact limit is 32 MiB, the node limit is 4096,
and normalized JSON values are limited to depth 64 and one million total values. The borrowed
payload is size-checked before per-node hashing; final serialization uses the same bounded
writer before Store writes any blob. SHA-256 definition digests include protocol, transport,
the full normalized node definition, and all accepted defaults. The limited default subset
contains `mode`, `log-level`, `unified-delay`, `tcp-concurrent`, and
`global-client-fingerprint`; `log-level` permits `warning`, not `warn`.

`read_source_artifact` verifies the current safe provenance, Source, blob metadata, body digest,
User-Agent digest, pin, format, generation, and every current Node link. `read_node_definition`
resolves archived immutable Nodes using their own artifact reference and returns both the full
definition, defaults, original source generation, format, origin, and core pin that influenced its digest. Same-body updates that change format,
normalized definitions, or defaults are refused before publication. Store durability errors
propagate unchanged as typed Store errors; in particular, an indeterminate post-rename sync
is never reported as a successful rollback.

Synthetic checks in `tests/audit_i03_artifact.rs` cover actual-winner persistence across Store
reopen, safe graph/debug views, same-body reuse, User-Agent-only replacement, archived node
resolution, changed-body generation advance, changed-payload refusal, restricted fields, and
resource bounds. They are compile-checked only; **NOT_RUN**. No binary, service, network, parser,
or Mihomo core was started.

Sol also prepared three provenance-model checks and two independent artifact checks (typed
protocol/transport identity and aggregate size rejection). All ten T03 checks compile without
warnings; execution remains **NOT_RUN**. [Actual compile evidence](i03-evidence/provenance-review-2026-10-04/summary.json).
