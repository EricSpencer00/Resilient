#!/usr/bin/env python3
"""Check the Lean model and explicitly elaborate emitted demo theorems."""

import argparse
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def run(command, cwd, timeout):
    result = subprocess.run(
        command, cwd=cwd, text=True, capture_output=True, timeout=timeout, check=True
    )
    return result.stdout


def verify(compiler, lake, root=ROOT, timeout=120):
    project = root / "resilient" / "lean-spec"
    source = root / "resilient" / "examples" / "lean_spec_demo.rz"
    run([lake, "build"], project, timeout)
    # A library build follows imports; an unimported Generated module needs
    # its own elaboration command, even after the library build succeeds.
    with tempfile.TemporaryDirectory(prefix="resilient-lean-") as directory:
        for function in ("double", "triple"):
            theorem = run(
                [compiler, f"--emit-lean-spec={function}", str(source)], root, timeout
            )
            if not re.search(rf"\btheorem\s+{function}_correct\b", theorem):
                raise ValueError(f"{function}: compiler did not emit the expected theorem")
            if re.search(r"\b(sorry|admit|axiom)\b", theorem):
                raise ValueError(f"{function}: emitted source contains a proof admission")
            path = Path(directory) / f"{function.capitalize()}.lean"
            path.write_text(theorem, encoding="utf-8")
            run([lake, "env", "lean", str(path)], project, timeout)
            print(f"{function}: Lean checked")
    print("Checked 2 emitted expression-model theorems; no backend correctness claim.")


def executable(value):
    found = shutil.which(value)
    if found is None:
        raise ValueError(f"executable not found: {value}")
    # Commands run in two directories, so relative executable paths must be
    # made absolute before entering either working directory. Preserve the
    # invocation name: toolchain proxies dispatch from names such as `lake`.
    return str(Path(found).absolute())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", required=True, help="path to a built rz binary")
    parser.add_argument("--lake", default="lake", help="Lake executable (default: PATH)")
    parser.add_argument("--timeout", type=int, default=120, help="seconds per command")
    args = parser.parse_args()
    try:
        if args.timeout <= 0:
            raise ValueError("timeout must be positive")
        verify(executable(args.compiler), executable(args.lake), timeout=args.timeout)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Lean verification failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError):
            print(error.stdout or "", file=sys.stderr)
            print(error.stderr or "", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
