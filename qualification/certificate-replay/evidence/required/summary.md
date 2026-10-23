# End-to-end evidence

- Run ID: 4fe58c05fc431aa57c9d960a3891079eac15724ee8faa6a90aa751d93034298b
- Result: passed (exit code 0)
- Source revision: 821b5513382d5d471fc787c56308cf4a90388d7f
- Command: cargo test --locked --manifest-path resilient/Cargo.toml --features z3 --test solver_replay_required

## Toolchain

- cargo: cargo 1.98.1 (797e8a9bc 2026-08-05)
- python: Python 3.12.3
- rustc: rustc 1.98.1 (48a229cea 2026-09-01)

## Inputs

- Cargo.lock: b4f6e62a8d081da34b6d45174a8ea964f85eb9e128a4d8ac2bf2d8752e6de58f
- resilient/src/lib.rs: 361de29520b1ffecf6588c7a807cd295800450e960d28f13e16e71d8b67ccabf
- resilient/tests/solver_replay_required.rs: a8db20647cdd0e8fbf68c8323944770997ddf3992d945e878d45d880c2f942c0

## Outcome

- command: passed
- hash_only_verification_does_not_require_solver: passed
- requested_replay_rejects_empty_obligation_set: passed
- requested_replay_rejects_missing_absolute_solver: passed
- requested_replay_rejects_solver_absent_from_path: passed
