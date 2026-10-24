# Red-green E2E attestation

- Test: straight_return_replays
- Baseline: straight_return_replays failed as a test failure
- Final: straight_return_replays passed
- Command: cargo test --manifest-path resilient/Cargo.toml --locked --features z3 --test contract_replay_roundtrip straight_return_replays -- --exact --nocapture

## Runs

### Baseline

- Run ID: 7a8e744c2240572e00f645230a979889d6b0ebf976a8180af168546d87366884
- Manifest SHA-256: 2d16654d1c2061f0cb637d0d04e39064430ff2cedbc6cba70886ac974bfc5566
- Source revision: c9bf76695638eeccd0dd6cc3b0f8aac42e1d3694
- Result: failed (exit code 101)

### Final

- Run ID: 2a4bcc348c0b7bc6ee05604730fd5b1f4d428d07267abf0e98f6ac71082e4266
- Manifest SHA-256: 773efc7fd8b2ba3f9fc072e6305b89bc883d6a6a28bd9d096024269a0f2308ab
- Source revision: dd68821162438478c65bbbdb4f1cff52f9e09afd
- Result: passed (exit code 0)

## Toolchain

- cargo: cargo 1.99.1 (5f94df478 2026-08-27)
- python: Python 3.14.4
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)

## Inputs

- resilient/tests/contract_replay_roundtrip.rs: 1a8c0fd584b41abc243ebde1b5a839beee754b2f0501bac9f5d0fc396f219601

## Outcome

- baseline straight_return_replays: failed
- final straight_return_replays: passed
