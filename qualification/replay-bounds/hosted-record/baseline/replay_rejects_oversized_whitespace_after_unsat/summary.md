# End-to-end evidence

- Run ID: addb21005eecfad2504d5cbdae77dd2510e2045547f5e04c878f1d3f0f088569
- Result: failed (exit code 101)
- Source revision: b272c109ef1ca61104eeff7f025362a08e053c4b
- Command: cargo test --manifest-path resilient/Cargo.toml --locked --features z3 --test solver_replay_bounds replay_rejects_oversized_whitespace_after_unsat -- --exact --nocapture

## Toolchain

- cargo: cargo 1.99.1 (5f94df478 2026-08-27)
- python: Python 3.14.4
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)

## Inputs

- resilient/tests/solver_replay_bounds.rs: 76cf986da4d530b91024440907d9d94fdd88aa05afcbb7f7dac6dbc7578d06a6

## Outcome

- command: failed
- replay_rejects_oversized_whitespace_after_unsat: failed
