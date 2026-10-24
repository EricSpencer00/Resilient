# End-to-end evidence

- Run ID: fdb199ff00d1da56ba4ce1c4b848d5e184c769633fa4354db46b1f43d387238a
- Result: passed (exit code 0)
- Source revision: dd68821162438478c65bbbdb4f1cff52f9e09afd
- Command: cargo test --manifest-path resilient/Cargo.toml --locked --features z3 --test contract_replay_roundtrip false_postcondition_is_rejected -- --exact --nocapture

## Toolchain

- cargo: cargo 1.99.1 (5f94df478 2026-08-27)
- python: Python 3.14.4
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)

## Inputs

- resilient/tests/contract_replay_roundtrip.rs: 1a8c0fd584b41abc243ebde1b5a839beee754b2f0501bac9f5d0fc396f219601

## Outcome

- command: passed
- false_postcondition_is_rejected: passed
