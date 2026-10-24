# Red-green E2E attestation

- Test: replay_rejects_oversized_whitespace_after_unsat
- Baseline: replay_rejects_oversized_whitespace_after_unsat failed as a test failure
- Final: replay_rejects_oversized_whitespace_after_unsat passed
- Command: cargo test --manifest-path resilient/Cargo.toml --locked --features z3 --test solver_replay_bounds replay_rejects_oversized_whitespace_after_unsat -- --exact --nocapture

## Runs

### Baseline

- Run ID: addb21005eecfad2504d5cbdae77dd2510e2045547f5e04c878f1d3f0f088569
- Manifest SHA-256: 2ca6b3cd69a05e0d8d9ee6c70bfb6c2fd3827bdee3fc639c230723388d1ee3f7
- Source revision: b272c109ef1ca61104eeff7f025362a08e053c4b
- Result: failed (exit code 101)

### Final

- Run ID: daa52b29c5dd1f99cfc88e6bfb9c4a13518fbaa46231272ccabde0c1a3b41aba
- Manifest SHA-256: 074a71771c7c93fc9fd65c03544f93aa2a6f3ba8240b36a9eebbc9a4c5b794d3
- Source revision: ded79260b1960de1249fd2231f9f9cd8c1e25e0b
- Result: passed (exit code 0)

## Toolchain

- cargo: cargo 1.99.1 (5f94df478 2026-08-27)
- python: Python 3.14.4
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)

## Inputs

- resilient/tests/solver_replay_bounds.rs: 76cf986da4d530b91024440907d9d94fdd88aa05afcbb7f7dac6dbc7578d06a6

## Outcome

- baseline replay_rejects_oversized_whitespace_after_unsat: failed
- final replay_rejects_oversized_whitespace_after_unsat: passed
