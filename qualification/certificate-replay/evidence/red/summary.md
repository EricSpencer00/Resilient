# End-to-end evidence

- Run ID: 544de02bd9e619dcaaf4149e947c0209289aa54e9dc50f285a57f05d9fbc38fa
- Result: failed (exit code 101)
- Source revision: ef28c2220270cf57f4b6ac2bda5cc6626c318944
- Command: cargo test --locked --manifest-path resilient/Cargo.toml --features z3 --test solver_replay_required

## Toolchain

- cargo: cargo 1.98.1 (797e8a9bc 2026-08-05)
- python: Python 3.12.3
- rustc: rustc 1.98.1 (48a229cea 2026-09-01)

## Inputs

- Cargo.lock: b4f6e62a8d081da34b6d45174a8ea964f85eb9e128a4d8ac2bf2d8752e6de58f
- resilient/tests/solver_replay_required.rs: a8db20647cdd0e8fbf68c8323944770997ddf3992d945e878d45d880c2f942c0

## Outcome

- command: failed
- hash_only_verification_does_not_require_solver: passed
- requested_replay_rejects_empty_obligation_set: failed
- requested_replay_rejects_missing_absolute_solver: failed
- requested_replay_rejects_solver_absent_from_path: failed
