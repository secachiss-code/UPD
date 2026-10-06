"""Plan I03 regressions; --run executes local fixtures after a matching install.

Dry-run (no --run) prints the case list and does not open the network, import a
subscription, or touch legacy VPN files. Real provider negotiation stays in X.04.
"""

from check_i01_regressions import ROOT, main

MANIFEST = ROOT / "docs/design/cm-network-manager/i03-evidence/build-2026-10-06/summary.json"
CASES = [
    ("native_parser", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_native"]),
    ("strict_options", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_t04_b"]),
    ("tls_and_protocols", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_tls"]),
    ("protocols", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_protocols"]),
    ("secrets", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_secrets"]),
    ("wireguard", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_t04_m"]),
    ("uri", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_uri"]),
    ("formats", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_formats"]),
    ("omissions", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_omissions"]),
    ("pipeline", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_pipeline"]),
    ("capabilities", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_capabilities"]),
    ("negotiation", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_negotiation"]),
    ("transport", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_transport"]),
    ("failures", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_failures"]),
    ("manual_server", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_manual"]),
    ("sources", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_sources"]),
    ("artifact", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_artifact"]),
    ("artifact_review", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_artifact_review"]),
    ("core_check", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_core_check"]),
    ("provenance", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_provenance"]),
    ("fetch_settings", ["cargo", "test", "--offline", "--locked", "--test", "audit_i03_fetch_settings"]),
]

if __name__ == "__main__":
    main(cases=CASES, default_manifest=MANIFEST, description=__doc__)
