#!/usr/bin/env python3
"""Verify real package metadata and embedded binaries after ./package.sh. Requires bsdtar."""
import hashlib, json, struct, subprocess
from pathlib import Path
root = Path(__file__).resolve().parents[1]
manifest = dict((line.rstrip('\n').split('\t', 1) for line in (root / 'dist/package-manifest.tsv').read_text().splitlines()))
expected = {name: hashlib.sha256(Path(manifest[key]).read_bytes()).hexdigest() for name, key in [('cm', 'cli'), ('cm-cosmic', 'gui')] if key in manifest}

def tar_list(path=None, data=None):
    return subprocess.check_output(['bsdtar', '-tf', str(path) if path else '-'], input=data).decode().splitlines()

def tar_get(member, path=None, data=None):
    return subprocess.check_output(['bsdtar', '-xOf', str(path) if path else '-', member], input=data)

def rpm_header(data, offset):
    assert data[offset:offset + 3] == b'\x8e\xad\xe8'
    count, size = struct.unpack_from('>II', data, offset + 8)
    assert count < 4096 and size < 1 << 20
    store = offset + 16 + count * 16
    fields = {}
    for i in range(count):
        tag, kind, relative, n = struct.unpack_from('>IIII', data, offset + 16 + i * 16)
        if kind == 6:
            fields[tag] = data[store + relative:store + size].split(b'\x00', 1)[0].decode()
    return (fields, store + size)
results = []
for name in expected:
    for suffix in [f"-{manifest['version']}-1-x86_64.pkg.tar.zst", f"_{manifest['version']}-1_amd64.deb", f"-{manifest['version']}-1.x86_64.rpm"]:
        path = root / 'dist' / (name + suffix)
        outer = tar_list(path)
        if path.suffix == '.deb':
            control = tar_get(next((n for n in outer if n.startswith('control.tar'))), path)
            metadata = tar_get(next((n for n in tar_list(data=control) if n.endswith('control'))), data=control).decode()
            assert 'Package: ' + name + '\n' in metadata and 'Version: ' + manifest['version'] + '-1\n' in metadata
            assert 'Architecture: amd64\n' in metadata
            data = tar_get(next((n for n in outer if n.startswith('data.tar'))), path)
            members = tar_list(data=data)
            binary = tar_get(next((n for n in members if n.lstrip('./') == 'usr/bin/' + name)), data=data)
        else:
            members = outer
            binary = tar_get(next((n for n in members if n.lstrip('./') == 'usr/bin/' + name)), path)
            if path.suffix == '.rpm':
                raw = path.read_bytes()
                assert raw[:4] == b'\xed\xab\xee\xdb'
                _, offset = rpm_header(raw, 96)
                fields, _ = rpm_header(raw, (offset + 7) // 8 * 8)
                assert (fields[1000], fields[1001], fields[1002], fields[1022]) == (name, manifest['version'], '1', 'x86_64')
            else:
                metadata = tar_get('.PKGINFO', path).decode()
                assert 'pkgname = ' + name + '\n' in metadata and 'pkgver = ' + manifest['version'] + '-1\n' in metadata and ('arch = x86_64\n' in metadata)
        digest = hashlib.sha256(binary).hexdigest()
        assert digest == expected[name], f'stale binary: {path.name}'
        results.append({'package': path.name, 'binary_sha256': digest, 'name_version_arch_checked': True})
print(json.dumps(results, indent=2))
(root / 'dist/real-package-proof.json').write_text(json.dumps(results, indent=2) + '\n')
