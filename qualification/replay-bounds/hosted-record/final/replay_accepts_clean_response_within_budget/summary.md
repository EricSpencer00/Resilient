# End-to-end evidence

- Run ID: 57cbe7c2a8836dfd59ef88a24539e09a68d29ce5f4ec9ae1c0917057df3192ee
- Result: passed (exit code 0)
- Source revision: ded79260b1960de1249fd2231f9f9cd8c1e25e0b
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
