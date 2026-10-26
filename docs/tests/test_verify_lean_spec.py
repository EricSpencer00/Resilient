"""Regression checks for the generated-module verification boundary."""

import contextlib
import importlib.util
import io
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "verify_lean_spec", Path(__file__).resolve().parents[1] / "verify_lean_spec.py"
)
verifier = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verifier)


class VerifyLeanSpecTests(unittest.TestCase):
    def invoke(self, command, cwd, timeout):
        self.commands.append((command, cwd, timeout))
        if command[0] == "rz":
            function = command[1].split("=", 1)[1]
            return f"theorem {function}_correct : True := by trivial\n"
        if command[1:3] == ["env", "lean"]:
            path = Path(command[3])
            self.assertTrue(path.is_file())
            self.assertIn("theorem", path.read_text())
            self.outputs.append(path)
        return ""

    def setUp(self):
        self.commands = []
        self.outputs = []

    def check(self, runner):
        with patch.object(verifier, "run", side_effect=runner):
            with contextlib.redirect_stdout(io.StringIO()):
                verifier.verify("rz", "lake", Path("/repo"), timeout=17)

    def test_explicitly_checks_both_generated_modules_and_cleans_up(self):
        self.check(self.invoke)
        self.assertEqual(len(self.commands), 5)
        self.assertEqual(self.commands[0], (["lake", "build"], Path("/repo/resilient/lean-spec"), 17))
        self.assertEqual(len(self.outputs), 2)
        self.assertTrue(all(not path.exists() for path in self.outputs))
        self.assertTrue(all(timeout == 17 for _, _, timeout in self.commands))

    def test_each_failed_stage_prevents_success(self):
        for stage in range(5):
            with self.subTest(stage=stage):
                self.commands = []
                def fail(command, cwd, timeout):
                    if len(self.commands) == stage:
                        raise subprocess.CalledProcessError(1, command)
                    return self.invoke(command, cwd, timeout)
                with self.assertRaises(subprocess.CalledProcessError):
                    self.check(fail)
                self.assertEqual(len(self.commands), stage)
        self.assertTrue(all(not path.exists() for path in self.outputs))

    def test_empty_or_wrong_theorem_cannot_pass(self):
        for output in ("", "import Resilient", "theorem other_correct : True := by trivial"):
            with self.subTest(output=output):
                with self.assertRaisesRegex(ValueError, "expected theorem"):
                    self.check(lambda command, cwd, timeout: output)

    def test_admissions_cannot_pass(self):
        for admission in ("sorry", "admit", "axiom"):
            with self.subTest(admission=admission):
                with self.assertRaisesRegex(ValueError, "proof admission"):
                    self.check(lambda command, cwd, timeout: f"theorem double_correct := {admission}")

    def test_timeout_cannot_pass(self):
        with self.assertRaises(subprocess.TimeoutExpired):
            self.check(lambda command, cwd, timeout: (_ for _ in ()).throw(
                subprocess.TimeoutExpired(command, timeout)
            ))

    def test_missing_executable_cannot_pass(self):
        with patch.object(verifier.shutil, "which", return_value=None):
            with self.assertRaisesRegex(ValueError, "executable not found"):
                verifier.executable("missing-rz")

    def test_relative_executable_is_resolved(self):
        with patch.object(verifier.shutil, "which", return_value="bin/rz"):
            self.assertEqual(verifier.executable("bin/rz"), str(Path("bin/rz").resolve()))


if __name__ == "__main__":
    unittest.main()
