#!/usr/bin/env python3
"""Exercise the public installer with a bounded local release transport."""
import hashlib
import io
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]

class InstallerContract(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="rz-install-contract-")
        self.root = Path(self.temporary.name)
        self.transport = self.root / "release"
        self.transport.mkdir()
        self.tools = self.root / "tools"
        self.tools.mkdir()
        self.prefix = self.root / "prefix with spaces"
        self.profile = self.root / "profile"
        self.archive_name = f"rz-v1.2.0-{self.target()}.tar.gz"
        self.make_archive({"rz": b"#!/bin/sh\nprintf 'rz 1.2.0\\n'\n"})
        mock = self.tools / "curl"
        mock.write_text('''#!/usr/bin/env python3
import json,os,pathlib,sys
args=sys.argv[1:]
url=next(arg for arg in args if arg.startswith('https://'))
if url.endswith('/releases/latest'):
 print(json.dumps({'tag_name':'v1.2.0'}));sys.exit(0)
p=pathlib.Path(os.environ['RZ_TEST_RELEASE'])/url.rsplit('/',1)[-1]
if not p.is_file(): sys.exit(22)
if '-o' in args: pathlib.Path(args[args.index('-o')+1]).write_bytes(p.read_bytes())
elif '--output' in args: pathlib.Path(args[args.index('--output')+1]).write_bytes(p.read_bytes())
else: sys.stdout.buffer.write(p.read_bytes())
''')
        mock.chmod(0o755)
        self.env = dict(os.environ, RZ_PREFIX=str(self.prefix), RZ_PROFILE=str(self.profile), RZ_TEST_RELEASE=str(self.transport), PATH=str(self.tools)+os.pathsep+os.environ['PATH'])

    @staticmethod
    def target():
        system = subprocess.check_output(['uname','-s'], text=True).strip()
        arch = subprocess.check_output(['uname','-m'], text=True).strip()
        return ('aarch64' if arch in ('arm64','aarch64') else 'x86_64') + ('-apple-darwin' if system == 'Darwin' else '-unknown-linux-gnu')

    def make_archive(self, members):
        archive = self.transport / self.archive_name
        with tarfile.open(archive, 'w:gz') as output:
            for name, contents in members.items():
                info = tarfile.TarInfo(name)
                info.mode = 0o755
                info.size = len(contents)
                output.addfile(info, io.BytesIO(contents))
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        (self.transport/'SHA256SUMS').write_text(f'{digest}  {self.archive_name}\n')

    def install(self):
        return subprocess.run(['bash',str(REPO/'scripts/install.sh')], env=self.env, capture_output=True, text=True, timeout=30)

    def tearDown(self):
        self.temporary.cleanup()

    def test_install_configures_reusable_shell_without_manual_export(self):
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
        self.assertEqual(subprocess.check_output([str(self.prefix/'bin/rz')],text=True).strip(),'rz 1.2.0')
        self.assertTrue(self.profile.is_file(), result.stdout)
        command = subprocess.run(['bash','--noprofile','--norc','-c','. "$1"; command -v rz','--',str(self.profile)],env=self.env,capture_output=True,text=True)
        self.assertEqual(command.returncode,0,command.stderr)
        self.assertEqual(command.stdout.strip(),str(self.prefix/'bin/rz'))

    def test_profile_update_is_idempotent(self):
        self.assertEqual(self.install().returncode,0)
        self.assertEqual(self.install().returncode,0)
        self.assertTrue(self.profile.is_file())
        self.assertEqual(self.profile.read_text().count('# Resilient installer'),1)

    def test_corrupt_download_is_rejected_before_install(self):
        archive=self.transport/self.archive_name
        archive.write_bytes(archive.read_bytes()+b'corrupt')
        self.assertNotEqual(self.install().returncode,0)
        self.assertFalse((self.prefix/'bin/rz').exists())

    def test_missing_checksum_is_rejected(self):
        (self.transport/'SHA256SUMS').write_text('')
        self.assertNotEqual(self.install().returncode,0)
        self.assertFalse((self.prefix/'bin/rz').exists())

    def test_invalid_archive_layout_is_rejected(self):
        self.make_archive({'unexpected':b'not a compiler'})
        self.assertNotEqual(self.install().returncode,0)
        self.assertFalse((self.prefix/'bin/rz').exists())

    def test_failed_update_preserves_existing_binary(self):
        binary=self.prefix/'bin/rz'
        binary.parent.mkdir(parents=True)
        binary.write_text('previous installation')
        (self.transport/'SHA256SUMS').write_text('0'*64+'  '+self.archive_name+'\n')
        self.assertNotEqual(self.install().returncode,0)
        self.assertEqual(binary.read_text(),'previous installation')

if __name__ == '__main__':
    unittest.main(verbosity=2)
