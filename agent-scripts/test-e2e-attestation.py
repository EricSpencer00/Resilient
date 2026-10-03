#!/usr/bin/env python3
"""Public CLI adversarial cases for linked red/green evidence."""
import json
import hashlib
import importlib.util
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
RUNNER = ROOT / 'agent-scripts/e2e-evidence.py'
INPUT = 'agent-scripts/test-e2e-attestation.py'


class AttestationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(dir=ROOT / 'target')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.fixture = self.root / 'fixture.py'

    def cli(self, *args):
        return subprocess.run([sys.executable, str(RUNNER), *map(str, args)], cwd=ROOT,
                              capture_output=True, text=True, timeout=30)

    def bundle(self, name, failed=False, incomplete=False, extra_failed=False, input_path=INPUT):
        statuses = [('regression', 'FAILED' if failed else 'ok')]
        if extra_failed:
            statuses.append(('unrelated', 'FAILED'))
        lines = ['test ' + test + ' ... ' + status for test, status in statuses]
        if not incomplete:
            count = sum(status == 'FAILED' for _, status in statuses)
            lines.append(f"test result: {'FAILED' if count else 'ok'}. {len(statuses)-count} passed; {count} failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s")
        code = 101 if failed or incomplete else 0
        self.fixture.write_text('print(' + repr('\n'.join(lines)) + ')\nraise SystemExit(' + str(code) + ')\n')
        out = self.root / name
        result = self.cli('run', '--regression-test', 'regression', '--output', out,
                          '--input', input_path, '--', sys.executable, self.fixture)
        self.assertEqual(result.returncode, code, result.stderr)
        self.assertEqual(self.cli('verify', out).returncode, 0)
        return out

    def pair(self, red, green, output='pair'):
        return self.cli('attest', '--baseline', red, '--final', green, '--test', 'regression', '--output', self.root / output)

    def test_portable_pair_and_deterministic_checksums(self):
        red, green = self.bundle('red', True), self.bundle('green')
        result = self.pair(red, green)
        self.assertEqual(result.returncode, 0, result.stderr)
        pair = self.root / 'pair'
        self.assertEqual(self.cli('verify', pair).returncode, 0)
        manifest = json.loads((pair / 'manifest.json').read_text())
        self.assertEqual(manifest['test'], 'regression')
        self.assertEqual(manifest['baseline']['result']['status'], 'failed')
        checksums = (pair / 'checksums.sha256').read_bytes()
        self.assertEqual(self.pair(red, green, 'second').returncode, 0)
        self.assertEqual(checksums, (self.root / 'second/checksums.sha256').read_bytes())
        moved = self.root / 'moved'
        shutil.copytree(pair, moved)
        shutil.rmtree(red)
        shutil.rmtree(green)
        self.assertEqual(self.cli('verify', moved).returncode, 0)

    def test_incomplete_harness_and_unrelated_test_failure_do_not_count_as_red(self):
        green = self.bundle('green')
        for red in [self.bundle('incomplete', True, incomplete=True),
                    self.bundle('unrelated', True, extra_failed=True), self.bundle('passing')]:
            self.assertNotEqual(self.pair(red, green).returncode, 0)

    def test_changed_inputs_command_or_toolchain_and_duplicate_outcomes_are_rejected(self):
        red = self.bundle('red', True)
        green = self.bundle('green', input_path='Cargo.toml')
        self.assertNotEqual(self.pair(red, green).returncode, 0)
        green = self.bundle('matching')
        for field in ['command', 'toolchain', 'outcomes']:
            with self.subTest(field=field):
                corrupted = self.root / field
                shutil.copytree(green, corrupted)
                path = corrupted / 'manifest.json'
                manifest = json.loads(path.read_text())
                if field == 'command':
                    manifest[field].append('changed')
                elif field == 'toolchain':
                    manifest[field]['rustc'] = 'changed'
                else:
                    manifest['result']['outcomes'].append({'name': 'regression', 'status': 'passed'})
                # Recompute valid individual-bundle checksums, so pairing must
                # reject the semantic mismatch rather than ordinary corruption.
                spec = importlib.util.spec_from_file_location('evidence_runner', RUNNER)
                runner = importlib.util.module_from_spec(spec)
                spec.loader.exec_module(runner)
                manifest['run_id'] = runner.run_id_for({key: manifest[key] for key in ['source', 'command', 'toolchain', 'inputs']})
                summary = runner.summary_text(manifest)
                (corrupted / 'summary.md').write_text(summary)
                manifest['evidence_sha256'] = {'summary.md': hashlib.sha256(summary.encode()).hexdigest()}
                path.write_bytes(runner.canonical_json(manifest))
                (corrupted / 'checksums.sha256').write_text(runner.checksums_text(corrupted))
                self.assertEqual(self.cli('verify', corrupted).returncode, 0)
                self.assertNotEqual(self.pair(red, corrupted).returncode, 0)

    def test_altered_nested_bundle_or_summary_is_rejected(self):
        red, green = self.bundle('red', True), self.bundle('green')
        self.assertEqual(self.pair(red, green).returncode, 0)
        pair = self.root / 'pair'
        (pair / 'baseline/summary.md').write_text('altered')
        self.assertNotEqual(self.cli('verify', pair).returncode, 0)


if __name__ == '__main__':
    unittest.main(verbosity=2)
