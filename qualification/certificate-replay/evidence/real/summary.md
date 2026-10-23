# End-to-end evidence

- Run ID: e346362a96a4504da7e1df8058996e63f7ebccbf3c0556e24cf24b506101a864
- Result: passed (exit code 0)
- Source revision: 821b5513382d5d471fc787c56308cf4a90388d7f
- Command: python3 qualification/certificate-replay/run-real-solver.py --rz resilient/target/debug/rz --solver /usr/bin/z3 --output ../real-results.json

## Toolchain

- cargo: cargo 1.98.1 (797e8a9bc 2026-08-05)
- python: Python 3.12.3
- rustc: rustc 1.98.1 (48a229cea 2026-09-01)

## Inputs

- qualification/certificate-replay/queries/invalid.smt2: bc54c7dab754dc8d4af5169f96068b22736d20d35fd72a8cba9d28eb5f162fa1
- qualification/certificate-replay/queries/multiple.smt2: caa7a6a1b28e7434b3bfbd819c2c8dab6c3c56445a04e893971d2a35f43cac1e
- qualification/certificate-replay/queries/sat.smt2: 62d584b96631365b27039a05c9985bbebdcadaf875e2a04c8f6af3b79e3619ac
- qualification/certificate-replay/queries/unsat.smt2: 9c4eecf83f4b88fbd5a7822b6b9a9ab7c1f0eca154757f5d0f5214757d86668e
- qualification/certificate-replay/run-real-solver.py: 2fa7939afd77f690a7d8decc1eae47f2c79300a5b8c22031a2779a20526698b8
- resilient/target/debug/rz: 19a2b251d2d6d8c347c8d4a471009df444a587bcb74bbeaabd255f0e73488aaf

## Outcome

- command: passed
- real_solver::invalid: passed
- real_solver::multiple: passed
- real_solver::sat: passed
- real_solver::unsat: passed
