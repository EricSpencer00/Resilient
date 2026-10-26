#!/usr/bin/env python3
"""Check lifecycle safety and require counterexamples for four cleanup mutants."""

import argparse
from pathlib import Path
import shutil
import subprocess
import tempfile

MODEL_DIR = Path(__file__).resolve().parents[1]
MUTANTS = {
    "runnable": (
        "runnable' = SelectSeq(runnable, LAMBDA q : q # p)",
        "runnable' = runnable",
    ),
    "blocked": ("blocked' = blocked \\ {p}", "blocked' = blocked"),
    "mailbox": ("mailbox' = [mailbox EXCEPT ![p] = <<>>]", "mailbox' = mailbox"),
    "accounting": (
        "droppedCount' = droppedCount + Len(mailbox[p])",
        "droppedCount' = droppedCount",
    ),
}


def check(java, jar, timeout):
    original = (MODEL_DIR / "runtime_lifecycle.tla").read_text()
    for name in ("original", *MUTANTS):
        with tempfile.TemporaryDirectory(prefix="resilient-lifecycle-") as directory:
            root = Path(directory)
            for filename in ("runtime.tla", "runtime_lifecycle.cfg"):
                shutil.copyfile(MODEL_DIR / filename, root / filename)
            source = original
            if name != "original":
                before, after = MUTANTS[name]
                if source.count(before) != 1:
                    raise RuntimeError(f"{name}: mutation target missing or ambiguous")
                source = source.replace(before, after)
            (root / "runtime_lifecycle.tla").write_text(source)
            result = subprocess.run(
                [java, "-XX:+UseParallelGC", "-Xmx1g", "-cp", str(jar), "tlc2.TLC",
                 "-workers", "1", "-seed", "1", "runtime_lifecycle.tla"],
                cwd=root, text=True, capture_output=True, timeout=timeout,
            )
            output = result.stdout + result.stderr
            if name == "original":
                passed = result.returncode == 0 and "Model checking completed. No error has been found." in output
            else:
                # A parse/tool failure is not a detected safety violation.
                passed = result.returncode != 0 and "is violated" in output and "The behavior up to this point is:" in output
            if not passed:
                raise RuntimeError(f"{name}: unexpected TLC outcome ({result.returncode})\n{output}")
            print(f"{name}: {'safety checked' if name == 'original' else 'counterexample found'}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jar", required=True, type=Path)
    parser.add_argument("--java", default="java")
    parser.add_argument("--timeout", type=int, default=120, help="seconds per model")
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("timeout must be positive")
    jar = args.jar.resolve(strict=True)
    java = shutil.which(args.java)
    if java is None:
        parser.error(f"Java executable not found: {args.java}")
    check(str(Path(java).resolve()), jar, args.timeout)


if __name__ == "__main__":
    main()
