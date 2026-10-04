"""C18: actual help reads only a private config and preserves six locales."""
from pathlib import Path
from datetime import datetime, timezone
import hashlib
import json
import os
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/x86_64-unknown-linux-musl/debug/cm"
EXPECTED = {
    "ru": ("управление Linux из консоли", "Переход UPD → CM выполняется явно: cm migration apply."),
    "en": ("Linux updates with automatic mirror selection", "UPD → CM migration requires an explicit cm migration apply command."),
    "de": ("Linux-Updates mit automatischer Spiegelwahl", "Die Migration UPD → CM erfolgt ausdrücklich mit cm migration apply."),
    "it": ("aggiornamento di Linux con scelta automatica dei mirror", "La migrazione UPD → CM richiede il comando esplicito cm migration apply."),
    "zh": ("自动选择镜像的 Linux 更新工具", "UPD → CM 迁移需要显式运行 cm migration apply。"),
    "ar": ("تحديث Linux مع اختيار تلقائي للمرايا", "يتطلب ترحيل UPD → CM تنفيذ الأمر cm migration apply صراحةً."),
}

report = {
    "utc": datetime.now(timezone.utc).isoformat(),
    "binary_sha256": hashlib.sha256(BINARY.read_bytes()).hexdigest(),
    "source_sha256": {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                      for p in [ROOT / "src/main.rs", ROOT / "src/i18n_table.rs"]},
    "scope": "actual help, private CM_CONF, synthetic external command sentinels",
    "cases": [],
}
for lang, (headline, notice) in EXPECTED.items():
    with tempfile.TemporaryDirectory(prefix="i01-help-") as directory:
        root = Path(directory)
        conf = root / "cm.conf"
        content = f"lang={lang}\n".encode()
        conf.write_bytes(content)
        conf.chmod(0o600)
        fakebin = root / "bin"
        fakebin.mkdir()
        marker = root / "unexpected-command"
        for name in ["sudo", "systemctl", "pacman", "apt-get", "gsettings", "nmcli"]:
            p = fakebin / name
            p.write_text(f"#!/bin/sh\n: > '{marker}'\nexit 91\n")
            p.chmod(0o700)
        before = sorted(str(p.relative_to(root)) for p in root.rglob("*"))
        env = {k: v for k, v in os.environ.items() if not k.startswith(("CM_", "UPD_"))}
        env.update(CM_CONF=str(conf), PATH=str(fakebin), LANG="ru_RU.UTF-8", LC_ALL="ru_RU.UTF-8")
        for arg in ["help", "--help"]:
            p = subprocess.run([str(BINARY), arg], env=env, capture_output=True, timeout=5)
            output = p.stdout.decode()
            unchanged = conf.read_bytes() == content and conf.stat().st_mode & 0o777 == 0o600
            unchanged = unchanged and before == sorted(str(p.relative_to(root)) for p in root.rglob("*"))
            good = p.returncode == 0 and headline in output.splitlines()[0] and notice in output
            good = good and not p.stderr and unchanged and not marker.exists()
            report["cases"].append({"lang": lang, "argv": [str(BINARY), arg],
                                    "exit_code": p.returncode, "localized_headline": headline in output.splitlines()[0],
                                    "localized_migration_notice": notice in output,
                                    "private_config_and_tree_unchanged": unchanged,
                                    "no_external_commands": not marker.exists(),
                                    "status": "PASS" if good else "FAIL"})
report["status"] = "PASS" if all(c["status"] == "PASS" for c in report["cases"]) else "FAIL"
report["finished_utc"] = datetime.now(timezone.utc).isoformat()
(ROOT / "docs/design/cm-network-manager/i01-evidence/help-locales.json").write_text(json.dumps(report, indent=2) + "\n")
print(f"{report['status']}: {len(report['cases'])} actual help cases, six locales, private configs")
raise SystemExit(0 if report["status"] == "PASS" else 1)
