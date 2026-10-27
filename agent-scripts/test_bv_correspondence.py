#!/usr/bin/env python3
"""Strict descriptor validation independent of solver-equivalence outcomes."""
import hashlib
import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location(
    "correspondence", ROOT / "agent-scripts/proof-correspondence-checker.py")
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)
FIXTURES = ROOT / "resilient/tests/fixtures/proof_correspondence"


class BvDescriptors(unittest.TestCase):
    def check(self, value, expected=None, profile="vm-i64-wrap-v1"):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "descriptor.json"
            raw = json.dumps(value).encode()
            path.write_bytes(raw)
            args = SimpleNamespace(descriptor=str(path), profile=profile,
                                   descriptor_sha256=hashlib.sha256(raw).hexdigest())
            if expected:
                with self.assertRaises(CHECKER.core.Rejected) as caught:
                    CHECKER.descriptor(args)
                self.assertEqual(caught.exception.code, expected)
            else:
                self.assertEqual(CHECKER.descriptor(args), value)

    def fixture(self, name="if_else"):
        return json.loads((FIXTURES / f"bv_{name}.json").read_text())

    def test_four_reviewed_source_families(self):
        for name in ("straight", "if_else", "early_fallthrough", "first_match_guarded"):
            with self.subTest(name=name):
                self.check(self.fixture(name))

    def test_legacy_fixture_validation_is_preserved(self):
        for name in ("straight", "if_else", "early_fallthrough", "first_match_guarded"):
            value = json.loads((FIXTURES / f"legacy_{name}.json").read_text())
            self.check(value, profile="legacy-lia-identity-v1")

    def test_profile_mismatch_is_rejected(self):
        self.check(self.fixture(), "unsupported_profile", profile="legacy-lia-identity-v1")

    def test_narrow_width_integer_and_nonzero_arity_are_rejected(self):
        for declaration in (["declare-fun", "x", [], ["_", "BitVec", "32"]],
                            ["declare-const", "x", "Int"],
                            ["declare-fun", "x", [["_", "BitVec", "64"]], ["_", "BitVec", "64"]]):
            value = self.fixture()
            value["paths"][0]["commands"][1] = declaration
            self.check(value, "invalid_descriptor")

    def test_domain_and_exact_command_order_cannot_diverge(self):
        value = self.fixture()
        value["paths"][0]["domain"] = "true"
        self.check(value, "invalid_descriptor")
        value = self.fixture()
        value["paths"][0]["commands"].insert(2, ["assert", "true"])
        self.check(value, "invalid_descriptor")

    def test_nonidentity_goal_is_rejected(self):
        value = self.fixture()
        value["paths"][0]["returned_value"] = "#x0000000000000000"
        self.check(value, "invalid_descriptor")

    def test_unreviewed_domain_operations_are_rejected(self):
        value = self.fixture()
        value["paths"][0]["domain"] = ["arbitrary-predicate", "x"]
        self.check(value, "invalid_descriptor")


if __name__ == "__main__":
    unittest.main()
