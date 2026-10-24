# End-to-end evidence

- Run ID: 6dedff81b5bc97a4f2ba6c634b5353a79b675c5bb8a18e2a6aea4fda96243833
- Result: failed (exit code 101)
- Source revision: c9bf76695638eeccd0dd6cc3b0f8aac42e1d3694
- Command: cargo test --manifest-path resilient/Cargo.toml --locked --features z3 --test contract_replay_roundtrip constant_return_replays -- --exact --nocapture

## Toolchain

- cargo: cargo 1.99.1 (5f94df478 2026-08-27)
- python: Python 3.14.4
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)

## Inputs

- resilient/tests/contract_replay_roundtrip.rs: 1a8c0fd584b41abc243ebde1b5a839beee754b2f0501bac9f5d0fc396f219601

## Outcome

- command: failed
- constant_return_replays: failed
