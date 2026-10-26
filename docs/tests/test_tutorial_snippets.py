"""Regression checks for executable tutorial coverage and compiler failures."""

import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

DRIVER = Path(__file__).resolve().parents[1] / "verify_tutorial_snippets.py"
SPEC = importlib.util.spec_from_file_location("tutorial_verifier", DRIVER)
verifier = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verifier)


class TutorialSnippetTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="tutorial tests ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.compiler = self.root / "fake rz"
        self.compiler.write_text(
            f"#!{sys.executable}\n"
            "import pathlib,sys,time\n"
            "source=pathlib.Path(sys.argv[-1]).read_text()\n"
            "if 'SLEEP' in source: time.sleep(5)\n"
            "if 'FAIL' in source:\n print('typed compiler diagnostic', file=sys.stderr); sys.exit(7)\n"
        )
        self.compiler.chmod(0o755)

    def run_driver(self, timeout="2"):
        return subprocess.run(
            [sys.executable, str(DRIVER), "--compiler", str(self.compiler),
             "--tutorial-dir", str(self.root), "--timeout", timeout],
            text=True, capture_output=True, timeout=10,
        )

    def write(self, source, name="lesson with spaces.md"):
        path = self.root / name
        path.write_bytes(source.encode())
        return path

    def test_no_documents_or_no_source_cannot_pass(self):
        self.assertNotEqual(self.run_driver().returncode, 0)
        self.write("# A lesson without source\n")
        result = self.run_driver()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no resilient snippets", result.stderr)

    def test_crlf_spacing_and_longer_closing_fences(self):
        path = self.write("   ````resilient title=example\r\nprintln(1);\r\n  `````  \r\n~~~resilient\r\nprintln(2);\r\n~~~~\r\n")
        extracted = list(verifier.snippets(path))
        self.assertEqual([body for _, _, body in extracted], ["println(1);\n", "println(2);\n"])
        result = self.run_driver()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Checked 2 snippets", result.stdout)

    def test_unrelated_fence_does_not_execute_embedded_source_example(self):
        path = self.write("````markdown\n```resilient\nFAIL\n```\n````\n```resilient\nprintln(1);\n```\n")
        self.assertEqual(len(list(verifier.snippets(path))), 1)
        self.assertEqual(self.run_driver().returncode, 0)

    def test_shorter_fence_does_not_close_source(self):
        path = self.write("````resilient\n```\nprintln(1);\n````\n")
        self.assertEqual(list(verifier.snippets(path))[0][2], "```\nprintln(1);\n")

    def test_unterminated_and_empty_fences_cannot_pass(self):
        for source in ("```resilient\nprintln(1);\n", "```resilient\n  \n```\n"):
            with self.subTest(source=source):
                self.write(source)
                self.assertNotEqual(self.run_driver().returncode, 0)

    def test_compiler_failure_reports_source_location_and_diagnostic(self):
        self.write("# Lesson\n```resilient\nFAIL\n```\n")
        result = self.run_driver()
        self.assertEqual(result.returncode, 1)
        self.assertIn("lesson with spaces.md:2: FAIL (exit 7)", result.stderr)
        self.assertIn("typed compiler diagnostic", result.stderr)

    def test_timeout_cannot_pass(self):
        self.write("```resilient\nSLEEP\n```\n")
        result = self.run_driver("0.05")
        self.assertEqual(result.returncode, 1)
        self.assertIn("timeout", result.stderr)

    def test_invalid_timeout_cannot_pass(self):
        self.write("```resilient\nprintln(1);\n```\n")
        for timeout in ("0", "-1", "nan", "inf"):
            with self.subTest(timeout=timeout):
                self.assertNotEqual(self.run_driver(timeout).returncode, 0)

    def test_shell_entry_point_preserves_environment_override(self):
        self.write("```resilient\nprintln(1);\n```\n")
        result = subprocess.run(
            ["bash", str(DRIVER.with_suffix(".sh")), "--tutorial-dir", str(self.root)],
            env={**os.environ, "RESILIENT_BIN": str(self.compiler)},
            text=True, capture_output=True, timeout=10,
        )
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
