"""Plan I02 regressions; --run executes synthetic cases after binary installation.

The driver verifies the installed binary and source manifest, preserves evidence,
and stops at the first failure. It does not manage host services or make requests.
"""

from check_i01_regressions import ROOT, main

MANIFEST = ROOT / "docs/design/cm-network-manager/i02-evidence/coder-t05-build-2026-10-04/summary.json"
CASES = [
    ("typed_model", ["cargo", "test", "--offline", "--locked", "--test", "audit_i02_profiles"]),
    ("private_store", ["cargo", "test", "--offline", "--locked", "--test", "audit_i02_store"]),
    ("publication_and_removal_failures", ["cargo", "test", "--offline", "--locked", "--lib", "profiles::store::fault_tests::"]),
    ("schema_examples", ["cargo", "test", "--offline", "--locked", "--test", "audit_i02_examples"]),
]

if __name__ == "__main__":
    main(cases=CASES, default_manifest=MANIFEST, description=__doc__)
