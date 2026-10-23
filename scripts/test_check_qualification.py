"""Hermetic regression tests for evidence intake; fixtures are not real approval."""
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from check_qualification import COMMON, PROFILES, check


class QualificationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "evidence.txt").write_text("synthetic test evidence")
        artifact = {"path": "evidence.txt", "sha256": hashlib.sha256((self.root / "evidence.txt").read_bytes()).hexdigest()}
        self.package = {"schema_version": 1, "project": "fixture", "owner": "author", "intended_use": "test only",
                        "standards": list(PROFILES), "baseline": {"source_revision": "a" * 40, "compiler_sha256": "b" * 64,
                        "target": "fixture", "backend": "vm", "build_flags": "default", "solver_version": "none"},
                        "controls": [{"id": key, "status": "reviewed", "owner": "author", "reviewer": "reviewer",
                                      "review_record": "synthetic review", "artifacts": [copy.deepcopy(artifact)]}
                                     for key in sorted(COMMON | set().union(*PROFILES.values()))],
                        "traceability": [{"requirement": "REQ-1", "source": "example.rz:1", "test": "TEST-1",
                                          "result": "pass", "evidence_control": "validation"}], "open_issues": []}

    def test_complete_inventory(self):
        self.assertEqual(check(self.package, self.root), [])

    def test_missing_profile_control(self):
        for standard, controls in PROFILES.items():
            for ident in controls:
                with self.subTest(standard=standard, control=ident):
                    package = copy.deepcopy(self.package)
                    package["controls"] = [c for c in package["controls"] if c["id"] != ident]
                    self.assertIn(f"missing control: {ident}", check(package, self.root))

    def test_tampered_evidence(self):
        (self.root / "evidence.txt").write_text("modified")
        self.assertTrue(any("SHA-256 mismatch" in gap for gap in check(self.package, self.root)))

    def test_path_escape_and_symlink(self):
        for path in ("../outside", str(self.root / "evidence.txt"), "escape"):
            with self.subTest(path=path):
                (self.root / "escape").unlink(missing_ok=True)
                (self.root / "escape").symlink_to(self.root.parent)
                self.package["controls"][0]["artifacts"][0]["path"] = path
                self.assertTrue(any("stay within" in gap for gap in check(self.package, self.root)))

    def test_empty_or_malformed_package(self):
        for package in (None, [], {}, {"standards": [{}]}, {"controls": [None]}, {"traceability": [None]}):
            with self.subTest(package=package):
                self.assertTrue(check(package, self.root))

    def test_open_unreviewed_and_duplicate_records(self):
        mutations = [lambda p: p.update(open_issues=["unresolved"]),
                     lambda p: p["controls"][0].update(status="open"),
                     lambda p: p["controls"][0].update(reviewer="author"),
                     lambda p: p["controls"].append(copy.deepcopy(p["controls"][0])),
                     lambda p: p["traceability"][0].update(evidence_control="missing"),
                     lambda p: p["traceability"][0].update(result="unknown"),
                     lambda p: p.update(standards=["unknown"])]
        for mutate in mutations:
            package = copy.deepcopy(self.package)
            mutate(package)
            self.assertTrue(check(package, self.root))

    def test_cli_exit_and_never_certified(self):
        path = self.root / "package.json"
        for package, expected in ((self.package, 0), ({}, 1)):
            path.write_text(json.dumps(package))
            result = subprocess.run([sys.executable, str(Path(__file__).with_name("check_qualification.py")), str(path)], capture_output=True, text=True)
            self.assertEqual(result.returncode, expected)
            self.assertIs(json.loads(result.stdout)["certified"], False)

    def test_repository_template_is_incomplete(self):
        path = Path(__file__).resolve().parents[1] / "qualification/package-template.json"
        self.assertTrue(check(json.loads(path.read_text()), path.parent))


if __name__ == "__main__":
    unittest.main()
