#!/usr/bin/env python3
"""Pair actual Lean evaluator outputs with interpreter and VM observations."""

import argparse
import os
from pathlib import Path
import re
import subprocess
import tempfile

from verify_lean_spec import ROOT, executable, run

MINIMUM = -(1 << 63)
MAXIMUM = (1 << 63) - 1


def cases():
    values = (-7, -3, -1, 0, 1, 3, 7)
    pairs = [(a, b) for a in values for b in values]
    pairs += [(MINIMUM, 1), (MINIMUM, 2), (MINIMUM, -2),
              (MINIMUM + 1, -1), (MAXIMUM, 1), (MAXIMUM, -2),
              (MAXIMUM, -1), (MINIMUM, -1)]
    return [(op, a, b) for a, b in pairs for op in ("div", "mod")
            if not (op == "div" and a == MINIMUM and b == -1)]


def source_integer(value):
    return "(-9223372036854775807 - 1)" if value == MINIMUM else f"({value})"


def verify(compiler, lake, timeout):
    project = ROOT / "resilient/lean-spec"
    run([lake, "build"], project, timeout)
    samples = cases()
    with tempfile.TemporaryDirectory(prefix="lean-signed-") as directory:
        directory = Path(directory)
        observations = ["import Resilient.SignedArithmetic"]
        for index, (op, a, b) in enumerate(samples):
            observations.append(
                f'#eval IO.println ("case{index}=" ++ (match '
                f'Resilient.eval [] (.{op} (.int ({a})) (.int ({b}))) with '
                '| some value => toString value | none => "undefined"))'
            )
        target = directory / "Observations.lean"
        target.write_text("\n".join(observations) + "\n")
        output = run([lake, "env", "lean", str(target)], project, timeout)
        expected = {}
        for line in output.splitlines():
            match = re.fullmatch(r"case(\d+)=(-?\d+|undefined)", line)
            if match:
                index = int(match[1])
                if index in expected:
                    raise ValueError("duplicate Lean observation")
                expected[index] = None if match[2] == "undefined" else int(match[2])
        if set(expected) != set(range(len(samples))):
            raise ValueError("missing or invalid Lean observations")
        environment = {**os.environ, "RESILIENT_OVERFLOW_MODE": "wrap"}
        for index, (op, a, b) in enumerate(samples):
            operator = "/" if op == "div" else "%"
            source = directory / "sample.rz"
            source.write_text(
                f"fn sample(int a, int b) -> int {{ return a {operator} b; }} "
                f"println(sample({source_integer(a)}, {source_integer(b)}));"
            )
            for backend, flags in (("interpreter", []), ("VM", ["--vm"])):
                result = subprocess.run([compiler, "--no-cache", *flags, str(source)],
                                        cwd=ROOT, env=environment, text=True,
                                        capture_output=True, timeout=timeout)
                if expected[index] is None:
                    message = result.stderr.lower()
                    if result.returncode in (0, 101) or "panicked" in message or "error:" not in message or not any(
                        marker in message for marker in ("division by zero", "modulo by zero", "divide by zero")
                    ):
                        raise ValueError(f"{backend}: missing typed zero-divisor failure for {a} {operator} {b}")
                else:
                    numbers = [int(line) for line in result.stdout.splitlines()
                               if re.fullmatch(r"-?\d+", line)]
                    if result.returncode != 0 or numbers != [expected[index]]:
                        raise ValueError(f"{backend}: signed arithmetic mismatch for {a} {operator} {b}: "
                                         f"expected {expected[index]}, got exit={result.returncode}, values={numbers}")
    print(f"Matched {len(samples)} Lean observations against both interpreter and VM ({2 * len(samples)} executions), wrap mode.")
    print("Excluded i64 MIN / -1: unbounded result exceeds i64. No universal backend refinement or overflow-policy proof.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", required=True)
    parser.add_argument("--lake", default="lake")
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("timeout must be positive")
    try:
        verify(executable(args.compiler), executable(args.lake), args.timeout)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Signed Lean correspondence check failed: {error}")
        if isinstance(error, subprocess.CalledProcessError):
            print(error.stdout or "")
            print(error.stderr or "")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
