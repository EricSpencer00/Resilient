# Red-green E2E attestation

- Test: constant_return_replays
- Baseline: constant_return_replays failed as a test failure
- Final: constant_return_replays passed
- Command: cargo test --manifest-path resilient/Cargo.toml --locked --features z3 --test contract_replay_roundtrip constant_return_replays -- --exact --nocapture

## Runs

### Baseline

- Run ID: 6dedff81b5bc97a4f2ba6c634b5353a79b675c5bb8a18e2a6aea4fda96243833
- Manifest SHA-256: e0fb41697dd048441f52f866802906d4d7215999b0dd157107865011ed11f084
- Source revision: c9bf76695638eeccd0dd6cc3b0f8aac42e1d3694
- Result: failed (exit code 101)

### Final

- Run ID: 1444dade07fd5bfdabfbc0fc79c9cdd4fc34aa21d342b07355a606bac5f04df3
- Manifest SHA-256: 6d5c988cf75d1e268d7e7184fb0e9db1104f0c204b44699316f3fa1d85bd29ae
- Source revision: dd68821162438478c65bbbdb4f1cff52f9e09afd
- Result: passed (exit code 0)

## Toolchain

- cargo: cargo 1.99.1 (5f94df478 2026-08-27)
- python: Python 3.14.4
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)

## Inputs

- resilient/tests/contract_replay_roundtrip.rs: 1a8c0fd584b41abc243ebde1b5a839beee754b2f0501bac9f5d0fc396f219601

## Outcome

- baseline constant_return_replays: failed
- final constant_return_replays: passed
