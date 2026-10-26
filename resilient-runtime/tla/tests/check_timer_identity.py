#!/usr/bin/env python3
"""Check timer capability lifecycle invariants and their three policy mutants."""

import argparse
from pathlib import Path
import subprocess
import tempfile

MODEL = Path(__file__).resolve().parents[1]
MUTANTS = ("CheckGeneration", "PreserveGeneration", "RetireOnExhaustion")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jar", required=True)
    parser.add_argument("--java", default="java")
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--log-dir", type=Path, help="retain raw TLC output and counterexample traces")
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("timeout must be positive")
    jar = Path(args.jar).absolute()
    try:
        if args.log_dir is not None:
            args.log_dir.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="timer-identity-tlc-") as directory:
            root = Path(directory)
            (root / "timer_identity.tla").write_text((MODEL / "timer_identity.tla").read_text())
            original = (MODEL / "timer_identity.cfg").read_text()
            for mutant in (None, *MUTANTS):
                config = original if mutant is None else original.replace(f"{mutant} = TRUE", f"{mutant} = FALSE")
                (root / "timer_identity.cfg").write_text(config)
                result = subprocess.run([args.java, "-cp", str(jar), "tlc2.TLC", "-workers", "1", "-cleanup", "-config", "timer_identity.cfg", "timer_identity.tla"],
                                        cwd=root, text=True, capture_output=True, timeout=args.timeout)
                output = result.stdout + result.stderr
                if args.log_dir is not None:
                    (args.log_dir / f"{mutant or 'baseline'}.log").write_text(output)
                if mutant is None:
                    if result.returncode or "Model checking completed. No error has been found." not in output:
                        raise ValueError(f"baseline model failed:\n{output}")
                    print("baseline: invariants passed", flush=True)
                else:
                    if result.returncode == 0 or not any(f"Invariant {name} is violated" in output for name in ("NoResurrection", "RejectRevoked")):
                        raise ValueError(f"{mutant}: expected stale-handle counterexample missing:\n{output}")
                    print(f"{mutant}=FALSE: stale-handle counterexample found", flush=True)
        print("Two slots, two generations; bounded capability safety only, no Rust refinement or timer scheduling proof.")
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Timer identity model check failed: {error}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
