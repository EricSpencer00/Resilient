#!/usr/bin/env python3
"""Elaborate emitted declarations for Lean keywords, core names and suffix collisions."""

import argparse
from pathlib import Path
import re
import subprocess
import tempfile

from verify_lean_spec import ROOT, executable, run

NAMES = ("Int", "Bool", "theorem", "namespace", "some", "none", "Expr", "eval",
         "env_of", "sample", "sample_expr", "sample_generated", "lean_arg_0", "end")


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
        combined = ["import Resilient.Semantics"]
        with tempfile.TemporaryDirectory(prefix="lean-names-") as directory:
            for index, name in enumerate(NAMES):
                source = Path(directory) / "sample.rz"
                expression = ("sample + 1", "sample / 3", "sample % 3")[index % 3]
                source.write_text(f"fn {name}(int sample, bool eval) -> int {{ return {expression}; }}")
                theorem = run([compiler, "--no-cache", f"--emit-lean-spec={name}", str(source)], ROOT, args.timeout)
                if not re.search(rf"\btheorem\s+{re.escape(name)}_correct\b", theorem) or re.search(r"\b(sorry|admit|axiom)\b", theorem):
                    raise ValueError(f"{name}: missing theorem or proof admission")
                target = Path(directory) / "Sample.lean"
                target.write_text(theorem)
                run([lake, "env", "lean", str(target)], project, args.timeout)
                body = "\n".join(line for line in theorem.splitlines() if not line.startswith("import "))
                combined.extend([f"section emitted_{index}", body, f"end emitted_{index}"])
                print(f"{name}: Lean checked", flush=True)
            target = Path(directory) / "Combined.lean"
            target.write_text("\n".join(combined) + "\n")
            run([lake, "env", "lean", str(target)], project, args.timeout)
        print(f"Checked {len(NAMES)} emitted functions individually and together; no declaration capture.")
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Lean declaration-hygiene verification failed: {error}")
        if isinstance(error, subprocess.CalledProcessError):
            print(error.stdout or "")
            print(error.stderr or "")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
