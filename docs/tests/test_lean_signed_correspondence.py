"""The paired oracle must reject incomplete observations and runtime divergence."""

import contextlib
import importlib.util
import io
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

DOCS = Path(__file__).parents[1]
sys.path.insert(0, str(DOCS))
SPEC = importlib.util.spec_from_file_location("signed_correspondence", DOCS / "verify_lean_signed.py")
VERIFIER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFIER)


def observed(code=0, stdout="-2\n", stderr=""):
    return SimpleNamespace(returncode=code, stdout=stdout, stderr=stderr)


class SignedCorrespondenceTests(unittest.TestCase):
    def check(self, model, results, samples=None):
        samples = samples or [("div", -7, 3), ("mod", -7, 3)]
        with patch.object(VERIFIER, "cases", return_value=samples), \
             patch.object(VERIFIER, "run", side_effect=["", model]), \
             patch.object(VERIFIER.subprocess, "run", side_effect=results) as runtime, \
             contextlib.redirect_stdout(io.StringIO()):
            VERIFIER.verify("rz", "lake", 1)
            return runtime.call_args_list

    def test_both_backends_are_compared_in_explicit_wrap_mode(self):
        calls = self.check("case0=-2\ncase1=-1\n", [observed(), observed(), observed(stdout="-1\n"), observed(stdout="-1\n")])
        self.assertEqual(len(calls), 4)
        self.assertNotIn("--vm", calls[0].args[0])
        self.assertIn("--vm", calls[1].args[0])
        self.assertTrue(all(call.kwargs["env"]["RESILIENT_OVERFLOW_MODE"] == "wrap" for call in calls))

    def test_a_vm_mismatch_cannot_be_hidden_by_interpreter_success(self):
        with self.assertRaisesRegex(ValueError, "VM: signed arithmetic mismatch"):
            self.check("case0=-2\ncase1=-1\n", [observed(), observed(stdout="-3\n")])

    def test_missing_duplicate_and_extra_model_observations_are_rejected(self):
        for model in ("case0=-2\n", "case0=-2\ncase0=-2\ncase1=-1\n", "case0=-2\ncase1=-1\ncase2=0\n"):
            with self.subTest(model=model), self.assertRaisesRegex(ValueError, "observation"):
                self.check(model, [])

    def test_zero_divisor_requires_a_runtime_error_not_a_panic(self):
        for result in (observed(101, "", "Error: thread panicked: divide by zero"),
                       observed(1, "", "Error: thread panicked: division by zero"),
                       observed(1, "", "unrelated failure"), observed()):
            with self.subTest(result=result), self.assertRaisesRegex(ValueError, "zero-divisor failure"):
                self.check("case0=undefined\n", [result], [("div", 7, 0)])
        calls = self.check("case0=undefined\n", [observed(1, "", "Error: Division by zero")] * 2, [("div", 7, 0)])
        self.assertEqual(len(calls), 2)

    def test_unbounded_overflow_is_explicitly_outside_the_compared_domain(self):
        cases = VERIFIER.cases()
        self.assertNotIn(("div", VERIFIER.MINIMUM, -1), cases)
        self.assertIn(("mod", VERIFIER.MINIMUM, -1), cases)
        self.assertIn(("div", VERIFIER.MINIMUM, 1), cases)
        self.assertIn(("div", -7, 3), cases)


if __name__ == "__main__":
    unittest.main()
