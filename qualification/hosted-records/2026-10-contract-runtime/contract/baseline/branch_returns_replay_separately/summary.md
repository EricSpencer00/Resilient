# End-to-end evidence

- Run ID: fda76be1ad4b4a930b8a581f822da3f732b0fe77e91cc4fbb345f71099ca6dac
- Result: failed (exit code 101)
- Source revision: c9bf76695638eeccd0dd6cc3b0f8aac42e1d3694
- Command: cargo test --manifest-path resilient/Cargo.toml --locked --features z3 --test contract_replay_roundtrip branch_returns_replay_separately -- --exact --nocapture

## Toolchain

- cargo: cargo 1.99.1 (5f94df478 2026-08-27)
- python: Python 3.14.4
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)

## Inputs

- resilient/tests/contract_replay_roundtrip.rs: 1a8c0fd584b41abc243ebde1b5a839beee754b2f0501bac9f5d0fc396f219601

## Outcome

- branch_returns_replay_separately: failed
- command: failed
