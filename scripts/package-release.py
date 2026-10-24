#!/usr/bin/env python3
"""Package an already-built native CLI and retain its exact build identity."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile

root = Path(__file__).resolve().parents[1]
target, tag = sys.argv[1:]
binary = root / 'resilient/target' / target / 'release/rz'
out = root / 'dist'
out.mkdir(exist_ok=True)
version = tag.removeprefix('v')
source = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()
tree = subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], cwd=root, text=True).strip()
banner = subprocess.check_output([str(binary), '--version', '--verbose'], text=True)
assert banner.startswith(f'rz {version}:'), banner
metadata = dict(line.strip().split(': ', 1) for line in banner.splitlines()[1:])
assert metadata['commit'] == source[:12], metadata
assert metadata['target'] == target and metadata['profile'] == 'release', metadata
features = metadata['features'].split(',')
assert {'lsp', 'z3', 'z3-static'}.issubset(features), metadata

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

archive = out / f'rz-{tag}-{target}.tar.gz'
with tarfile.open(archive, 'w:gz') as package:
    package.add(binary, arcname='rz')
# Check the bytes users will extract, rather than only the build-tree executable.
with tempfile.TemporaryDirectory(prefix='rz-release-extracted-') as temporary:
    with tarfile.open(archive) as package:
        package.extractall(temporary, filter='data')
    extracted = Path(temporary) / 'rz'
    assert digest(extracted) == digest(binary)
    subprocess.run(['bash', str(root/'scripts/release-smoke-test.sh'), str(extracted)], check=True, timeout=90)
    subprocess.run(['bash', str(root/'scripts/release-lsp-smoke-test.sh'), str(extracted)], check=True, timeout=90)
record = {
    'version': version, 'tag': tag, 'target': target,
    'source_commit': source, 'source_tree': tree,
    'profile': 'release', 'features': features,
    'rustc': metadata['rustc'], 'build_banner': banner,
    'z3_linkage': 'static', 'binary_sha256': digest(binary),
    'archive': archive.name, 'archive_sha256': digest(archive),
    'validation': ['extracted-binary-hash', 'extracted-z3-audit', 'extracted-lsp-initialize'],
    'scope': 'Native release CLI smoke evidence; separate from the debug qualification baseline. System C/C++ runtime dependencies are recorded separately.',
}
(out/f'rz-{tag}-{target}.json').write_text(json.dumps(record, sort_keys=True, indent=2)+'\n')
(out/(archive.name+'.sha256')).write_text(record['archive_sha256']+'  '+archive.name+'\n')
print(json.dumps(record, sort_keys=True, indent=2))
