"""Toolchain launchers must retain their invocation name across cwd changes."""

import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "lean_verifier_dispatch", Path(__file__).parents[1] / "verify_lean_spec.py"
)
VERIFIER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERIFIER)


class LauncherDispatchTests(unittest.TestCase):
    def test_symlink_launcher_keeps_argv_zero_in_another_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            proxy = root / "toolchain-proxy"
            proxy.write_text('#!/bin/sh\ncase "${0##*/}" in lake) printf lake;; *) exit 23;; esac\n')
            proxy.chmod(0o755)
            launcher = root / "lake"
            launcher.symlink_to(proxy)
            elsewhere = root / "project"
            elsewhere.mkdir()
            command = VERIFIER.executable(str(launcher))
            result = subprocess.run([command], cwd=elsewhere, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, "lake")
            self.assertEqual(command, str(launcher))

    def test_relative_path_lookup_is_absolute_without_following_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            proxy = root / "proxy"
            proxy.write_text("#!/bin/sh\nexit 0\n")
            proxy.chmod(0o755)
            (root / "lake").symlink_to(proxy)
            relative = os.path.relpath(root, Path.cwd())
            with patch.dict(os.environ, {"PATH": relative}):
                command = VERIFIER.executable("lake")
            self.assertTrue(Path(command).is_absolute())
            self.assertEqual(Path(command).name, "lake")
            self.assertTrue(Path(command).samefile(proxy))


if __name__ == "__main__":
    unittest.main()
