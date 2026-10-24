# End-to-end evidence

- Run ID: fa47dfaf7943f25a8584b21f3aae7947530958489b52d51bcbaca51cfe4a9ec5
- Result: passed (exit code 0)
- Source revision: b272c109ef1ca61104eeff7f025362a08e053c4b
- Command: cargo test --manifest-path resilient/Cargo.toml --locked --features z3 --test solver_replay_bounds replay_accepts_clean_response_within_budget -- --exact --nocapture

## Toolchain

- cargo: cargo 1.99.1 (5f94df478 2026-08-27)
- python: Python 3.14.4
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)

## Inputs

- resilient/tests/solver_replay_bounds.rs: 76cf986da4d530b91024440907d9d94fdd88aa05afcbb7f7dac6dbc7578d06a6

## Outcome

- command: passed
- replay_accepts_clean_response_within_budget: passed
