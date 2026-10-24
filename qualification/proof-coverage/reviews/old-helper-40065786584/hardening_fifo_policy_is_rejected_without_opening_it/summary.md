# End-to-end evidence

- Run ID: 8a087e4d0eaa1433ad7c5719399f100c9dd665fdce9f9a0fbd16d231f856179c
- Result: failed (exit code 101)
- Source revision: 3efcbaf7ba1c61959dbd07e396d8599bbf6b4116
- Command: cargo test --manifest-path resilient/Cargo.toml --locked --features z3 --test proof_coverage_audit hardening_fifo_policy_is_rejected_without_opening_it -- --exact --nocapture

## Toolchain

- cargo: cargo 1.99.1 (5f94df478 2026-08-27)
- python: Python 3.14.4
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)

## Inputs

- resilient/tests/proof_coverage_audit.rs: fb6784849f97b759565deec3ad726d3fb550dc180066891b205e0383c8a35a03

## Outcome

- command: failed
- hardening_fifo_policy_is_rejected_without_opening_it: failed
