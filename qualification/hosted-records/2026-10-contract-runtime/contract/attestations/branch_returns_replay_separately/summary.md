# Red-green E2E attestation

- Test: branch_returns_replay_separately
- Baseline: branch_returns_replay_separately failed as a test failure
- Final: branch_returns_replay_separately passed
- Command: cargo test --manifest-path resilient/Cargo.toml --locked --features z3 --test contract_replay_roundtrip branch_returns_replay_separately -- --exact --nocapture

## Runs

### Baseline

- Run ID: fda76be1ad4b4a930b8a581f822da3f732b0fe77e91cc4fbb345f71099ca6dac
- Manifest SHA-256: 9cd49d2d073710e036477212b28f2ff8db01d4a1ff37df88428ee491677f1f21
- Source revision: c9bf76695638eeccd0dd6cc3b0f8aac42e1d3694
- Result: failed (exit code 101)

### Final

- Run ID: 1acab49fbfbb3a163187f37cdd55a443d02a4cee0c6d6050c5e3a29cfce112e4
- Manifest SHA-256: 8aed79a9df20655148d2775ad88ba793314de188450ff4cefde0e40120c32df3
- Source revision: dd68821162438478c65bbbdb4f1cff52f9e09afd
- Result: passed (exit code 0)

## Toolchain

- cargo: cargo 1.99.1 (5f94df478 2026-08-27)
- python: Python 3.14.4
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)

## Inputs

- resilient/tests/contract_replay_roundtrip.rs: 1a8c0fd584b41abc243ebde1b5a839beee754b2f0501bac9f5d0fc396f219601

## Outcome

- baseline branch_returns_replay_separately: failed
- final branch_returns_replay_separately: passed
