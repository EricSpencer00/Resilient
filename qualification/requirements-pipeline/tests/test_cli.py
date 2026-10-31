"""Public CLI requirements, with an independently frozen transition oracle."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

TOOL = Path(__file__).resolve().parents[1] / "pipeline.py"
SOURCE = """States: off, ready, tripped.
Events: arm, fault, reset.
Initial: off.
R01: When state is off and event is arm, next state shall be ready.
R02: When state is off and event is fault, next state shall be tripped.
R03: When state is off and event is reset, next state shall be off.
R04: When state is ready and event is arm, next state shall be ready.
R05: When state is ready and event is fault, next state shall be tripped.
R06: When state is ready and event is reset, next state shall be off.
R07: When state is tripped and event is arm, next state shall be tripped.
R08: When state is tripped and event is fault, next state shall be tripped.
R09: When state is tripped and event is reset, next state shall be off.
S01: After event fault, state shall be tripped.
S02: From state tripped, event arm shall lead to state tripped.
L01: Assuming fair event fault, state tripped shall eventually be reached.
"""
EXPECTED = [
    ["off", "arm", "ready"], ["off", "fault", "tripped"], ["off", "reset", "off"],
    ["ready", "arm", "ready"], ["ready", "fault", "tripped"], ["ready", "reset", "off"],
    ["tripped", "arm", "tripped"], ["tripped", "fault", "tripped"], ["tripped", "reset", "off"],
]


class PipelineCLI(unittest.TestCase):
    def setUp(self):
        if os.environ.get("REQUIREMENTS_RECORDS"):
            self.root = Path(os.environ["REQUIREMENTS_RECORDS"]).resolve() / self._testMethodName
            self.root.mkdir(parents=True, exist_ok=False)
        else:
            self.temp = tempfile.TemporaryDirectory(prefix="requirements-e2e-")
            self.addCleanup(self.temp.cleanup)
            self.root = Path(self.temp.name)
        self.invocations = 0
        self.source = self.root / "requirements.txt"
        self.source.write_text(SOURCE)

    def run_tool(self, *args):
        command = [sys.executable, str(TOOL), *map(str, args)]
        result = subprocess.run(command, capture_output=True, text=True, timeout=90)
        self.invocations += 1
        (self.root / f"invocation-{self.invocations}.json").write_text(json.dumps({
            "command": command, "status": result.returncode, "stdout": result.stdout, "stderr": result.stderr,
        }, indent=2) + "\n")
        return result

    def generate(self, name="bundle"):
        bundle = self.root / name
        result = self.run_tool("generate", self.source, "--output", bundle)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return bundle

    def test_deterministic_outputs_and_independent_transition_oracle(self):
        first, second = self.generate(), self.generate("elsewhere")
        self.assertEqual({p.name: p.read_bytes() for p in first.iterdir()},
                         {p.name: p.read_bytes() for p in second.iterdir()})
        ir = json.loads((first / "model.json").read_text())
        self.assertEqual([[r["from"], r["event"], r["to"]] for r in ir["transitions"]], EXPECTED)
        self.assertEqual(len(ir["properties"]), 3)
        self.assertIn("WF_vars", (first / "Controller.tla").read_text())

    def test_rejects_ambiguous_partial_unknown_and_unreachable_requirements(self):
        bad = [
            SOURCE + "The system should usually be safe.\n",
            SOURCE + "L02: Assuming fair event reset, state off shall eventually be reached.\n",
            SOURCE.replace("R09: When state is tripped and event is reset, next state shall be off.\n", ""),
            SOURCE + "R10: When state is off and event is arm, next state shall be off.\n",
            SOURCE + "R01: When state is off and event is arm, next state shall be ready.\n",
            SOURCE.replace("next state shall be ready", "next state shall be absent"),
            SOURCE.replace("next state shall be ready", "next state shall be off"),
            SOURCE.replace("States: off, ready, tripped.", "States: off, off, tripped."),
            SOURCE.replace("Initial: off.", "Initial: absent."),
        ]
        for index, source in enumerate(bad):
            with self.subTest(index=index):
                self.source.write_text(source)
                output = self.root / f"bad-{index}"
                result = self.run_tool("generate", self.source, "--output", output)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("requirements:", result.stderr)
                self.assertFalse(output.exists())

    def test_bundle_tampering_is_not_accepted(self):
        bundle = self.generate()
        (bundle / "implementation.rz").write_text("fn step(int state, int event) -> int { return 0; }")
        result = self.run_tool("check-runtime", bundle, "--rz", "missing-tool", "--output", self.root / "evidence")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("artifact mismatch", result.stderr)

    def test_missing_compiler_cannot_pass(self):
        bundle = self.generate()
        result = self.run_tool("check-runtime", bundle, "--rz", self.root / "missing", "--output", self.root / "evidence")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("executable", result.stderr)

    def test_rehashed_tampering_and_stale_evidence_are_rejected(self):
        bundle = self.generate()
        manifest = json.loads((bundle / "manifest.json").read_text())
        (bundle / "Controller.cfg").write_text("SPECIFICATION Spec\n")
        import hashlib
        manifest["artifacts"]["Controller.cfg"] = hashlib.sha256((bundle / "Controller.cfg").read_bytes()).hexdigest()
        (bundle / "manifest.json").write_text(json.dumps(manifest))
        result = self.run_tool("check-runtime", bundle, "--rz", "missing", "--output", self.root / "evidence")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("artifact mismatch", result.stderr)
        clean = self.generate("clean")
        output = self.root / "existing"
        output.mkdir()
        (output / "report.json").write_text("old evidence")
        result = self.run_tool("check-runtime", clean, "--rz", "missing", "--output", output)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((output / "report.json").read_text(), "old evidence")

    @unittest.skipUnless(os.name == "posix", "executable fixture uses POSIX")
    def test_timeout_is_failure_with_retained_output(self):
        bundle = self.generate()
        stub = self.root / "hung-compiler"
        stub.write_text("#!/usr/bin/env python3\nimport time\nprint('started', flush=True)\ntime.sleep(5)\n")
        stub.chmod(0o700)
        output = self.root / "timeout"
        result = self.run_tool("check-runtime", bundle, "--rz", stub, "--timeout-seconds", "1", "--output", output)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("timed out", result.stderr)
        self.assertEqual(json.loads((output / "report.json").read_text())["status"], "failed")
        self.assertIn("started", (output / "interpreter.log").read_text())

    @unittest.skipUnless(os.environ.get("RZ_BIN"), "requires actual built compiler")
    def test_each_transition_mutation_is_detected(self):
        bundle = self.generate()
        source = (bundle / "implementation.rz").read_text()
        for index, (_, _, target) in enumerate(EXPECTED):
            with self.subTest(transition=index):
                lines = source.splitlines()
                correct = ["off", "ready", "tripped"].index(target)
                lines[index + 1] = lines[index + 1].replace(f"return {correct};", f"return {(correct + 1) % 3};")
                candidate = self.root / f"mutant-{index}.rz"
                candidate.write_text("\n".join(lines) + "\n")
                result = self.run_tool("check-runtime", bundle, "--rz", os.environ["RZ_BIN"],
                                       "--implementation", candidate, "--output", self.root / f"mutant-{index}")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("transition mismatch", result.stderr)

    @unittest.skipUnless(os.environ.get("RZ_BIN"), "requires actual built compiler")
    def test_every_transition_executes_under_interpreter_and_both_vm_dispatches(self):
        bundle = self.generate()
        evidence = self.root / "evidence"
        result = self.run_tool("check-runtime", bundle, "--rz", os.environ["RZ_BIN"], "--output", evidence)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        report = json.loads((evidence / "report.json").read_text())
        self.assertEqual(report["runtime_observations"], 27)
        self.assertEqual(report["model_check"], "not-run")
        self.assertFalse(report["certified"])
        self.assertEqual(set(report["observations"]), {"interpreter", "match", "direct"})
        self.assertEqual([r["requirement"] for r in report["observations"]["direct"]], [f"R{i:02}" for i in range(1, 10)])

    @unittest.skipUnless(os.environ.get("RZ_BIN"), "requires actual built compiler")
    def test_wrong_implementation_fails_actual_output_comparison(self):
        bundle = self.generate()
        mutant = self.root / "mutant.rz"
        mutant.write_text("fn step(int state, int event) -> int { return 0; }\n")
        evidence = self.root / "evidence"
        result = self.run_tool("check-runtime", bundle, "--rz", os.environ["RZ_BIN"],
                               "--implementation", mutant, "--output", evidence)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("transition mismatch", result.stderr)
        self.assertEqual(json.loads((evidence / "report.json").read_text())["status"], "failed")

    @unittest.skipUnless(os.environ.get("RZ_BIN") and os.environ.get("TLC_JAR"), "requires actual compiler and TLC")
    def test_real_model_and_runtime_agree_and_contradictory_property_fails(self):
        bundle = self.generate()
        result = self.run_tool("verify", bundle, "--rz", os.environ["RZ_BIN"], "--jar", os.environ["TLC_JAR"], "--output", self.root / "full")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        report = json.loads((self.root / "full/report.json").read_text())
        self.assertEqual(report["model_check"], "passed")
        self.assertEqual(report["model_transitions"], 9)
        self.source.write_text(SOURCE.replace("S01: After event fault, state shall be tripped.", "S01: After event fault, state shall be off."))
        bad = self.generate("contradictory")
        result = self.run_tool("verify", bad, "--rz", os.environ["RZ_BIN"], "--jar", os.environ["TLC_JAR"], "--output", self.root / "bad-full")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Invariant Property0 is violated", (self.root / "bad-full/tlc.log").read_text())
        self.source.write_text(SOURCE.replace("L01: Assuming fair event fault, state tripped", "L01: Assuming fair event fault, state ready"))
        bad = self.generate("bad-liveness")
        result = self.run_tool("verify", bad, "--rz", os.environ["RZ_BIN"], "--jar", os.environ["TLC_JAR"], "--output", self.root / "bad-live-evidence")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Temporal properties were violated", (self.root / "bad-live-evidence/tlc.log").read_text())


if __name__ == "__main__":
    unittest.main()
