#!/usr/bin/env python3
"""Check all-input partial-arithmetic theorems with pinned Lean."""

import argparse
from pathlib import Path
import re
import subprocess
import tempfile

from verify_lean_spec import ROOT, executable, run

CASES = {
    "symbolic_division": ("a / b", "int"),
    "symbolic_remainder": ("a % b", "int"),
    "nested_divisor": ("a / (b % c)", "int"),
    "nested_division": ("a / (b / c)", "int"),
    "partial_numerator": ("(a / b) / c", "int"),
    "partial_sum": ("(a / b) + (a % c)", "int"),
    "zero_divisor": ("a / 0", "int"),
    "zero_remainder": ("a % 0", "int"),
    "undefined_child": ("(a / 0) + b", "int"),
    "undefined_right": ("a * (b % 0)", "int"),
    "negative_division": ("(-7) / 3", "int"),
    "negative_remainder": ("(-7) % 3", "int"),
    "negative_divisor": ("7 / (-3)", "int"),
    "two_negative_values": ("(-7) % (-3)", "int"),
    "partial_comparison": ("a / b < c", "bool"),
    "partial_negation": ("!(a % b == c)", "bool"),
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
        with tempfile.TemporaryDirectory(prefix="lean-domains-") as directory:
            for name, (expression, result_type) in CASES.items():
                source = Path(directory) / "sample.rz"
                source.write_text(f"fn sample(int a, int b, int c) -> {result_type} {{ return {expression}; }}")
                theorem = run([compiler, "--no-cache", "--emit-lean-spec=sample", str(source)], ROOT, args.timeout)
                if not re.search(r"\btheorem\s+sample_correct\b", theorem) or re.search(r"\b(sorry|admit|axiom)\b", theorem):
                    raise ValueError(f"{name}: theorem missing or contains an admission")
                if "else none" not in theorem:
                    raise ValueError(f"{name}: missing undefined result")
                target = Path(directory) / "Sample.lean"
                target.write_text(theorem)
                run([lake, "env", "lean", str(target)], project, args.timeout)
                print(f"{name}: Lean checked", flush=True)
        print(f"Checked {len(CASES)} all-input partial-arithmetic expression theorems; no machine-arithmetic equivalence claim.")
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Lean arithmetic-domain verification failed: {error}")
        if isinstance(error, subprocess.CalledProcessError):
            print(error.stdout or "")
            print(error.stderr or "")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
