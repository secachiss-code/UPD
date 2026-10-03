"""Build/package contracts with local executables, without installing or downloading packages."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

class BuildArtifacts(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="cm build & fixture ")
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name) / "repo with spaces"
        self.repo.mkdir()
        for file in ["build.sh", "package.sh", "LICENSE"]:
            shutil.copy2(ROOT / file, self.repo / file)
        shutil.copytree(ROOT / "packaging", self.repo / "packaging")
        (self.repo / "cosmic").mkdir()
        (self.repo / "Cargo.toml").write_text('version = "1.2.3"\n')
        (self.repo / "dist").mkdir()
        (self.repo / "dist/cm-cosmic-linux-amd64").write_text("stale GUI")
        self.bin = Path(self.temp.name) / "fake bin"
        self.bin.mkdir()
        self.env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}", CM_NO_GUI="0", STUB_LIBS="1", STUB_FAIL="0", STUB_LOG=str(self.repo / "nfpm.jsonl"))
        self.executable("pkg-config", '#!/bin/sh\n[ "$STUB_LIBS" = 1 ]\n')
        self.executable("cargo", '''#!/usr/bin/python3
import os, pathlib, sys
assert '--locked' in sys.argv
assert '--target' in sys.argv
root = pathlib.Path.cwd()
gui = root.name == 'cosmic'
if os.environ['STUB_FAIL'] == ('gui' if gui else 'cli'):
    print('intentional cargo build failure', file=sys.stderr); sys.exit(42)
target = sys.argv[sys.argv.index('--target') + 1]
out = root / 'target' / target / 'release' / ('cm-cosmic' if gui else 'cm')
out.parent.mkdir(parents=True, exist_ok=True)
if gui:
    body = '#!/bin/sh\\nprintf "cm-cosmic 9.8.7\\\\n"\\n'
else:
    body = '#!/bin/sh\\ncase "$1" in --version) printf "cm 9.8.7\\\\n" ;; gen-files) mkdir -p "$2/usr/lib/systemd" "$2/usr/share/cm" ;; *) exit 3 ;; esac\\n'
out.write_text(body); out.chmod(0o755)
''')
        self.executable("nfpm", '''#!/usr/bin/python3
import json, os, pathlib, sys
args = sys.argv
config = pathlib.Path(args[args.index('-f') + 1]).read_text()
assert 'stale GUI' not in config
name = next(line.split(': ', 1)[1] for line in config.splitlines() if line.startswith('name: '))
for line in config.splitlines():
    if line.strip().startswith('- src: ') and '/.build.' in line:
        assert pathlib.Path(line.split('src: ', 1)[1]).is_file()
with open(os.environ['STUB_LOG'], 'a') as out:
    out.write(json.dumps({'name': name, 'version': os.environ['VERSION'], 'config': config}) + '\\n')
p = pathlib.Path(args[args.index('-t') + 1]); p.mkdir(parents=True, exist_ok=True)
(p / (name + '-' + os.environ['VERSION'] + '-' + args[args.index('-p') + 1] + '.pkg')).write_text('fresh package')
''')
    def executable(self, name, body):
        path = self.bin / name
        path.write_text(body)
        path.chmod(0o755)
    def package(self, **env):
        return subprocess.run([str(self.repo / "package.sh")], cwd=self.repo, env=dict(self.env, **env), text=True, capture_output=True)
    def entries(self):
        path = self.repo / "nfpm.jsonl"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
    def test_disabled_gui_excludes_stale_binary(self):
        result = self.package(CM_NO_GUI="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([e['name'] for e in self.entries()], ['cm'] * 3)
        self.assertTrue(all(e['version'] == '9.8.7' for e in self.entries()))
        self.assertFalse((self.repo / 'dist/cm-cosmic-linux-amd64').exists())
    def test_missing_libs_exclude_stale_gui(self):
        result = self.package(STUB_LIBS="0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([e['name'] for e in self.entries()], ['cm'] * 3)
    def test_enabled_gui_uses_fresh_staging_and_build_version(self):
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([e['name'] for e in self.entries()], ['cm', 'cm-cosmic'] * 3)
        self.assertTrue(all(e['version'] == '9.8.7' for e in self.entries()))
        self.assertTrue(all('/.build.' in e['config'] for e in self.entries() if e['name'] == 'cm-cosmic'))
    def test_failed_cli_or_gui_build_stops_packaging_and_preserves_log(self):
        for component in ['cli', 'gui']:
            with self.subTest(component=component):
                result = self.package(STUB_FAIL=component)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('intentional cargo build failure', result.stderr)
                self.assertEqual(self.entries(), [])
                self.assertTrue(list((self.repo / 'dist').glob('package-build.*.log')))

if __name__ == '__main__':
    unittest.main()
