# Red-green E2E attestation

- Test: replay_rejects_response_after_deadline
- Baseline: replay_rejects_response_after_deadline failed as a test failure
- Final: replay_rejects_response_after_deadline passed
- Command: cargo test --manifest-path resilient/Cargo.toml --locked --features z3 --test solver_replay_bounds replay_rejects_response_after_deadline -- --exact --nocapture

## Runs

### Baseline

- Run ID: 1ef3d51978dfbd54e5a36c0b13ca9a68821e86c8df1a77ff602c231be0e7ec66
- Manifest SHA-256: 1caf77e9490ebc7cda55fb0649a0c8662ba9b3ee61463ff1862f9230651065e4
- Source revision: b272c109ef1ca61104eeff7f025362a08e053c4b
- Result: failed (exit code 101)

### Final

- Run ID: 0411c0ae133e8db260321cd1a9cb78d180db39e9ffa452f4fed6805600618c9c
- Manifest SHA-256: 4bcfe9488a58aa4e763d48c56b95d6498e794bf7a81769578f62eb243b864a23
- Source revision: ded79260b1960de1249fd2231f9f9cd8c1e25e0b
- Result: passed (exit code 0)

## Toolchain

- cargo: cargo 1.99.1 (5f94df478 2026-08-27)
- python: Python 3.14.4
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)

## Inputs

- resilient/tests/solver_replay_bounds.rs: 76cf986da4d530b91024440907d9d94fdd88aa05afcbb7f7dac6dbc7578d06a6

## Outcome

- baseline replay_rejects_response_after_deadline: failed
- final replay_rejects_response_after_deadline: passed
