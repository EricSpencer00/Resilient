# Red-green E2E attestation

- Test: replay_rejects_invalid_timeout_configuration
- Baseline: replay_rejects_invalid_timeout_configuration failed as a test failure
- Final: replay_rejects_invalid_timeout_configuration passed
- Command: cargo test --manifest-path resilient/Cargo.toml --locked --features z3 --test solver_replay_bounds replay_rejects_invalid_timeout_configuration -- --exact --nocapture

## Runs

### Baseline

- Run ID: 57b05d3adc9e5fb8605569eb7e7b6463ca0746f58339390500f6170c767a7e90
- Manifest SHA-256: d935186c39ccf21bcfede0b3dd7148e53e001a6cd2b17d14ebb38262dd099312
- Source revision: b272c109ef1ca61104eeff7f025362a08e053c4b
- Result: failed (exit code 101)

### Final

- Run ID: f7476633fc6cd5fc070c206d6f6a9680ea89f53b25f575d7b53484751728e9d9
- Manifest SHA-256: b7174551bf21d24cf041c730197411348e122a2ac215178fca1276772086d90a
- Source revision: ded79260b1960de1249fd2231f9f9cd8c1e25e0b
- Result: passed (exit code 0)

## Toolchain

- cargo: cargo 1.99.1 (5f94df478 2026-08-27)
- python: Python 3.14.4
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)

## Inputs

- resilient/tests/solver_replay_bounds.rs: 76cf986da4d530b91024440907d9d94fdd88aa05afcbb7f7dac6dbc7578d06a6

## Outcome

- baseline replay_rejects_invalid_timeout_configuration: failed
- final replay_rejects_invalid_timeout_configuration: passed
