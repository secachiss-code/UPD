# I03.T01: capability matrix for source import

This is CM's deliberately bounded import policy for the pinned **mihomo v1.19.32**, commit `88dcbf7f1614a67c3b36b848ee3592dfa92ada36`. It is a static policy derived from the pinned upstream source review, not a claim about the version installed on a host and not runtime/core acceptance. See the [pinned source review](I03-PINNED-SOURCE-REVIEW-2026-10-04.md) and the [mihomo v1.19.32 release](https://github.com/MetaCubeX/mihomo/releases/tag/v1.19.32).

The version gate accepts exactly `1.19.32` or `v1.19.32`; all other strings produce the safe unit error `UnsupportedCoreVersion`. CM has not started a source parser, network import, or mihomo process in this task. Runtime status: **NOT_RUN**.

## Import formats

| `ImportFormat` | Policy |
|---|---|
| `UriList` | Explicit supported URI lines from the schemes below. |
| `Base64UriList` | Base64 representation of the same URI-list subset. |
| `MihomoYaml` | Native mihomo YAML, subject to the field policy below. |
| `MihomoJson` | Native mihomo JSON, subject to the same field policy below. |

Native Xray and sing-box formats are unsupported until their own adapters exist. WireGuard is accepted only as a native mihomo YAML/JSON outbound; CM does not invent a WireGuard URI scheme.

## Protocol and transport subset

| Model protocol | Accepted transport enum values | URI schemes |
|---|---|---|
| VLESS | TCP, WS, HTTP, H2, gRPC, XHTTP | `vless` |
| VMess | TCP, WS, HTTP, H2, gRPC | `vmess` |
| Trojan | TCP, WS, gRPC | `trojan` |
| Shadowsocks | TCP; UDP is a separate flag, not a stream transport | `ss` |
| SOCKS5 | TCP | `socks5`, `socks` |
| HTTP proxy | TCP | `http`, `https` |
| Hysteria2 | QUIC | `hysteria2`, `hy2` |
| TUIC | QUIC | `tuic` |
| WireGuard | Native WireGuard transport | None |
| Other | Rejected | None |

URI scheme matching is case-insensitive. The matrix is the CM import subset, not a statement that every upstream option, plugin, security mode, or endpoint combination works. A protocol/transport pair accepted here still needs later parse-time option validation and core validation before it can be used.

## Native config fields

`classify_native_config_field` applies a closed top-level policy. The separate `classify_node_option` identifies special restricted or unsupported per-node controls; it does not replace the future protocol-specific whitelist for ordinary node fields.

| Disposition | Keys and limits |
|---|---|
| `ProxyDefinitions` | `proxies` |
| `ProxyGroups` | `proxy-groups` |
| `Rules` | `rules` |
| `SubRules` | `sub-rules` |
| `ProviderDeclarations` | `proxy-providers`, `rule-providers`; the explicit local `file` and `inline` modes is capability-approved at this stage. Path, size, and content policy remain for the parser stage. |
| `ConstrainedDefault` | `mode`, `log-level`, `unified-delay`, `tcp-concurrent`, `global-client-fingerprint`. Classification does not accept arbitrary values; later parser policy must validate each field's narrow value set. |
| `RestrictedNative` | Listeners, TUN, port bindings, controllers, host routes/firewall and DNS controls, plus `interface-name`, `routing-mark`, `dialer-proxy`, `ip-stack`, and `remote-dns-resolve`. These fields do not transfer host-network ownership to an imported profile. |

Known but out-of-subset ECH, ShadowTLS, Restls, JLS, TLS mirror, Mekya/MKCP, and plugin options return `UnsupportedFeature`. `reality-opts` is a preserved **node** option whose protocol/value compatibility still requires later parsing; it is not an accepted top-level config field. Unknown native keys return `UnsupportedField`. Errors are unit-valued and never echo an unchecked field name or value; a future parser must reject the source instead of silently dropping an unsupported field.

The exact capability API is in [`src/sources/capabilities.rs`](../../../src/sources/capabilities.rs). Regression cases are prepared in [`tests/audit_i03_capabilities.rs`](../../../tests/audit_i03_capabilities.rs) and remain unexecuted until the installation-time test gate.
