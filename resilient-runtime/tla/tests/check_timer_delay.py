#!/usr/bin/env python3
"""Check bounded timer delay progress under fair ticks/polls and policy mutants."""

import argparse
from pathlib import Path
import subprocess
import tempfile

MODEL = Path(__file__).resolve().parents[1]
MUTANTS = ("AdvanceAfterExpiry", "AcknowledgeCancellation", "PreserveReservation")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jar", required=True)
    parser.add_argument("--java", default="java")
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--log-dir", type=Path)
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error("timeout must be positive")
    try:
        if args.log_dir is not None:
            args.log_dir.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="timer-delay-tlc-") as directory:
            root = Path(directory)
            (root / "timer_delay.tla").write_text((MODEL / "timer_delay.tla").read_text())
            original = (MODEL / "timer_delay.cfg").read_text()
            for mutant in (None, *MUTANTS):
                config = original if mutant is None else original.replace(f"{mutant} = TRUE", f"{mutant} = FALSE")
                (root / "timer_delay.cfg").write_text(config)
                result = subprocess.run([args.java, "-cp", str(Path(args.jar).absolute()), "tlc2.TLC", "-workers", "1", "-cleanup", "-config", "timer_delay.cfg", "timer_delay.tla"],
                                        cwd=root, text=True, capture_output=True, timeout=args.timeout)
                output = result.stdout + result.stderr
                if args.log_dir is not None:
                    (args.log_dir / f"{mutant or 'baseline'}.log").write_text(output)
                if mutant is None:
                    if result.returncode or "Model checking completed. No error has been found." not in output:
                        raise ValueError(f"baseline model failed:\n{output}")
                    print("baseline: invariants and fair-progress property passed", flush=True)
                else:
                    expected = "Invariant NoLostReservation is violated" if mutant == "PreserveReservation" else "Temporal properties were violated"
                    if result.returncode == 0 or expected not in output:
                        raise ValueError(f"{mutant}: expected counterexample missing:\n{output}")
                    print(f"{mutant}=FALSE: expected counterexample found", flush=True)
        print("Period 1, delay 3; fair positive ticks and waiter polls assumed. No Rust refinement or hardware latency proof.")
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"Timer delay model check failed: {error}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
