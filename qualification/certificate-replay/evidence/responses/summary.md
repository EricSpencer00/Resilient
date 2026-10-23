# End-to-end evidence

- Run ID: bc5d3a1de99244630b7af6710b01439ad1e41a480085f00970e1c6ee4c670516
- Result: passed (exit code 0)
- Source revision: 821b5513382d5d471fc787c56308cf4a90388d7f
- Command: cargo test --locked --manifest-path resilient/Cargo.toml --features z3 --test solver_replay_conformance

## Toolchain

- cargo: cargo 1.98.1 (797e8a9bc 2026-08-05)
- python: Python 3.12.3
- rustc: rustc 1.98.1 (48a229cea 2026-09-01)

## Inputs

- Cargo.lock: b4f6e62a8d081da34b6d45174a8ea964f85eb9e128a4d8ac2bf2d8752e6de58f
- resilient/src/solver_replay.rs: 82f0b2a2d8a3385ceb71214354197ab3a0bfb0d306fcc3ef117e3a5252c1d873
- resilient/tests/solver_replay_conformance.rs: ce5e3a25e43cd51455dbd0abdfb20e53f0d6649235a7e02ffc9d7af27f61999f

## Outcome

- command: passed
- replay_accepts_one_clean_unsat_response: passed
- replay_rejects_conflicting_or_extra_responses: passed
- replay_rejects_invalid_utf8_after_unsat: passed
- replay_rejects_nonproof_responses: passed
- replay_rejects_stderr_even_after_unsat: passed
- replay_rejects_unsat_from_failed_process: passed
