# I03.T04 native parser: first reviewed slice

This slice is a pure parser. It reads no host paths, performs no provider fetches, and does
not publish Store state. It accepts at most 8 MiB, requires UTF-8, and applies a duplicate-
preserving visitor with 64-level depth and one-million map/sequence-entry limits. YAML goes
through the streaming event guard before `serde_yaml`; aliases, anchors, tags, merge keys,
directives, multiple documents, complex keys, and out-of-bound streams are refused before
the loader can expand or build them. JSON and YAML duplicate keys are refused.

The implemented protocol slice is intentionally narrower than the static capability matrix:

| Protocol | Accepted in this slice | Explicitly refused for now |
|---|---|---|
| VLESS | TCP, UUID, UDP flag, `tls: false` | TLS/REALITY, flow, packet modes, other transports and nested options |
| VMess | TCP, UUID, recognized cipher, `alterId: 0`, UDP flag, `tls: false` | TLS, nonzero alter ID, packet/padding modes, other transports and nested options |
| Shadowsocks | TCP, ordinary AEAD cipher subset, password, UDP flag | 2022 cipher variants, plugins, UDP-over-TCP and other plugin settings |
| HTTP | Plain TCP, optional paired username/password, bounded string headers | TLS and certificate options |
| SOCKS5 | Plain TCP, optional paired username/password, UDP flag | TLS and certificate options |
| Trojan, Hysteria2, TUIC, WireGuard | — | Entire protocol branch pending its schema review |

All accepted node fields are validated and preserved in the private typed definition. Header
names are checked as HTTP tokens and case-insensitive duplicates are rejected; header keys
such as `dns` and `plugin` are treated as headers, not host controls. Server values must be
hostnames or IP literals, never URLs. Unknown node fields, unsupported modes, malformed
credentials, host-control options and unsupported core versions fail with fixed safe error
codes that do not include input text.

The accepted top-level subset is `proxies`, `mode`, `log-level`, `unified-delay`, and
`tcp-concurrent`. Groups, rules and providers are rejected as whole sections until they can
be fully validated and preserved. The pinned core has removed `global-client-fingerprint`;
its config parser logs that the setting is removed, so this parser refuses it rather than
accepting an ineffective default.

The schemas were checked against the pinned primary sources: [VLESS](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/adapter/outbound/vless.go), [VMess](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/adapter/outbound/vmess.go), [Shadowsocks](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/adapter/outbound/shadowsocks.go), [HTTP](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/adapter/outbound/http.go), [SOCKS5](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/adapter/outbound/socks5.go), [shared BasicOption](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/adapter/outbound/base.go), and [native config fields](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.32/config/config.go) at v1.19.32 / commit `88dcbf7f1614a67c3b36b848ee3592dfa92ada36`.

This is not completion of T04. TLS/REALITY, remaining native protocol branches, URI/base64,
and manual entry remain for subsequent review. The public bound atomic publication and
native negotiation pipeline is now implemented in `src/sources/pipeline.rs`; its transport
is injected and must enforce its deadline/size/redirect contract. Synthetic
parser fixtures are compile-checked only; runtime and fixture execution remain NOT_RUN.

Work stopped by the user on 2026-10-05. [Frozen snapshot evidence](i03-evidence/stop-check-2026-10-05/summary.json)
records the final root compile of all test targets, including four streaming-guard fixtures
and three parser/publication fixtures. Runtime remains NOT_RUN.
