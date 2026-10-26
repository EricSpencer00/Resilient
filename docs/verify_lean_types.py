#!/usr/bin/env python3
"""Elaborate typed int/bool emitter examples against the pinned Lean model."""

import argparse
from pathlib import Path
import re
import subprocess
import tempfile

from verify_lean_spec import ROOT, executable, run

CASES = {
    "true": "fn sample() -> bool { return true; }",
    "false": "fn sample() -> bool { return false; }",
    "boolean_parameter": "fn sample(bool x) -> bool { return x; }",
    "negation": "fn sample(bool x) -> bool { return !x; }",
    "double_negation": "fn sample(bool x) -> bool { return !!x; }",
    "boolean_equality": "fn sample(bool a, bool b) -> bool { return a == b; }",
    "negated_equality": "fn sample(bool a, bool b) -> bool { return !(a == b); }",
    "integer_equality": "fn sample(int a, int b) -> bool { return a == b; }",
    "less": "fn sample(int a, int b) -> bool { return a < b; }",
    "less_equal": "fn sample(int a, int b) -> bool { return a <= b; }",
    "greater": "fn sample(int a, int b) -> bool { return a > b; }",
    "greater_equal": "fn sample(int a, int b) -> bool { return a >= b; }",
    "negated_comparison": "fn sample(int a, int b) -> bool { return !(a < b); }",
    "arithmetic": "fn sample(int a, int b) -> int { return -(a + b) * (a - b); }",
    "negative_constant": "fn sample() -> int { return -7; }",
    "division": "fn sample() -> int { return 6 / 2; }",
    "modulo": "fn sample() -> int { return 7 % 2; }",
    "shadowing": "fn sample(int sample, bool eval) -> int { return sample; }",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", required=True)
    parser.add_argument("--lake", default="lake")
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("timeout must be positive")
    try:
        compiler, lake = executable(args.compiler), executable(args.lake)
        project = ROOT / "resilient/lean-spec"
        run([lake, "build"], project, args.timeout)
        with tempfile.TemporaryDirectory(prefix="lean-types-") as directory:
            for name, source in CASES.items():
                path = Path(directory) / "sample.rz"
                path.write_text(source)
                theorem = run([compiler, "--no-cache", "--emit-lean-spec=sample", str(path)], ROOT, args.timeout)
                if not re.search(r"\btheorem\s+sample_correct\b", theorem) or re.search(r"\b(sorry|admit|axiom)\b", theorem):
                    raise ValueError(f"{name}: theorem missing or contains an admission")
                target = Path(directory) / "Sample.lean"
                target.write_text(theorem)
                run([lake, "env", "lean", str(target)], project, args.timeout)
                print(f"{name}: Lean checked", flush=True)
        print(f"Checked {len(CASES)} typed expression-model theorems; no backend correctness claim.")
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Typed Lean verification failed: {error}")
        if isinstance(error, subprocess.CalledProcessError):
            print(error.stdout or "")
            print(error.stderr or "")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
