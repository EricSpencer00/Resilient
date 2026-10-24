# End-to-end evidence

- Run ID: f68b121275ff2fbf96c430393bc3cea632c8c81834f83cd8f3bbf34d5a8c724a
- Result: failed (exit code 101)
- Source revision: 3efcbaf7ba1c61959dbd07e396d8599bbf6b4116
- Command: cargo test --manifest-path resilient/Cargo.toml --locked --features z3 --test proof_coverage_audit hardening_fifo_query_is_rejected_before_replay -- --exact --nocapture

## Toolchain

- cargo: cargo 1.99.1 (5f94df478 2026-08-27)
- python: Python 3.14.4
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)

## Inputs

- resilient/tests/proof_coverage_audit.rs: fb6784849f97b759565deec3ad726d3fb550dc180066891b205e0383c8a35a03

## Outcome

- command: failed
- hardening_fifo_query_is_rejected_before_replay: failed
