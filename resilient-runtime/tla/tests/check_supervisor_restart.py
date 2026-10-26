#!/usr/bin/env python3
"""Compare a bounded timed supervisor model with compiled Rust observations."""

import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
MODEL = HERE.parent / "supervisor_restart.tla"
MUTANTS = {
    "zero-budget": ("budget > 0 /\\ ", "", 1),
    "direct-recording": ("    /\\ CanRestart\n", "    /\\ TRUE\n", 1),
    "window-boundary": ("Elapsed >= window", "Elapsed > window", 2),
    "clock-rollback": ("now - last ELSE 0", "now - last ELSE last - now", 1),
}
CONFIG = """SPECIFICATION Spec
CONSTANTS MaxBudget = 2
          MaxWindow = 2
          MaxTime = 3
INVARIANT TypeOK
INVARIANT BudgetBound
INVARIANT ZeroBudgetDisabled
INVARIANT RuntimeCorrespondence
CHECK_DEADLOCK FALSE
"""


def observations(output):
    expected = {(b, w, c, last, now) for b in range(3) for w in range(3)
                for c in range(b + 1) for last in range(4) for now in range(4)}
    seen = set()
    rows = []
    for line in output.splitlines():
        fields = line.split(",")
        if len(fields) != 9 or any(value not in ("true", "false") for value in fields[5:7]):
            raise ValueError(f"invalid runtime observation: {line}")
        key = tuple(int(value) for value in fields[:5])
        if key in seen:
            raise ValueError("duplicate runtime observation")
        seen.add(key)
        values = [str(int(value)) for value in fields[:5]]
        values += [value.upper() for value in fields[5:7]]
        values += [str(int(value)) for value in fields[7:]]
        rows.append("<<" + ", ".join(values) + ">>")
    if seen != expected:
        raise ValueError(f"runtime observation domain incomplete: {len(seen)} of {len(expected)}")
    return "{\n" + ",\n".join(rows) + "\n}"


def check(java, rustc, jar, runtime_source, timeout):
    with tempfile.TemporaryDirectory(prefix="resilient-restart-") as directory:
        root = Path(directory)
        oracle = (HERE / "supervisor_restart_oracle.rs").read_text()
        relative = '#[path = "../../../resilient/src/supervisor_runtime.rs"]'
        if oracle.count(relative) != 1:
            raise ValueError("runtime source hook missing or ambiguous")
        oracle = oracle.replace(relative, "#[path = " + json.dumps(str(runtime_source), ensure_ascii=False) + "]")
        source = root / "oracle.rs"
        source.write_text(oracle)
        executable = root / "oracle"
        subprocess.run([rustc, "--edition=2024", str(source), "-o", str(executable)],
                       check=True, timeout=timeout, capture_output=True, text=True)
        result = subprocess.run([str(executable)], check=True, timeout=timeout,
                                capture_output=True, text=True)
        rows = observations(result.stdout)
        wrapper = """---- MODULE supervisor_restart_oracle ----
EXTENDS supervisor_restart
ObservedRows == %s
RuntimeCorrespondence ==
    <<budget, window, count, last, now, CanRestart, CanRestart,
      RecordedCount, RecordedLast>> \\in ObservedRows
====
""" % rows
        original = MODEL.read_text()
        for name in ("original", *MUTANTS):
            case = root / name
            case.mkdir()
            model = original
            if name != "original":
                before, after, expected = MUTANTS[name]
                if model.count(before) != expected:
                    raise ValueError(f"{name}: mutation target missing or ambiguous")
                model = model.replace(before, after)
            (case / "supervisor_restart.tla").write_text(model)
            (case / "supervisor_restart_oracle.tla").write_text(wrapper)
            (case / "supervisor_restart_oracle.cfg").write_text(CONFIG)
            result = subprocess.run(
                [java, "-XX:+UseParallelGC", "-Xmx512m", "-cp", str(jar), "tlc2.TLC", "-workers", "1",
                 "-seed", "1", "supervisor_restart_oracle.tla"],
                cwd=case, capture_output=True, text=True, timeout=timeout,
            )
            output = result.stdout + result.stderr
            if name == "original":
                passed = result.returncode == 0 and "Model checking completed. No error has been found." in output
            else:
                passed = (result.returncode != 0 and "Error: Invariant " in output and "is violated" in output
                          and ("The behavior up to this point is:" in output
                               or "is violated by the initial state:" in output))
            if not passed:
                raise RuntimeError(f"{name}: unexpected TLC outcome ({result.returncode})\n{output}")
            print(f"{name}: {'288 runtime observations agree; safety checked' if name == 'original' else 'counterexample found'}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jar", type=Path, required=True)
    parser.add_argument("--runtime-source", type=Path, default=ROOT / "resilient/src/supervisor_runtime.rs")
    parser.add_argument("--java", default="java")
    parser.add_argument("--rustc", default="rustc")
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("timeout must be positive")
    java, rustc = shutil.which(args.java), shutil.which(args.rustc)
    if java is None or rustc is None:
        parser.error("Java and rustc executables are required")
    try:
        # Rustup-style proxies dispatch by executable name; keep symlinks intact.
        check(str(Path(java).absolute()), str(Path(rustc).absolute()), args.jar.resolve(strict=True),
              args.runtime_source.resolve(strict=True), args.timeout)
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        detail = ""
        if isinstance(error, subprocess.CalledProcessError):
            detail = (error.stdout or "") + (error.stderr or "")
        parser.exit(1, f"Supervisor model check failed: {error}\n{detail}")


if __name__ == "__main__":
    main()
