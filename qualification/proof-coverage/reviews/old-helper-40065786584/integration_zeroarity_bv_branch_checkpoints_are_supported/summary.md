# End-to-end evidence

- Run ID: ff644480cc197239242686658dce56e3f26c5d8f83110bdd8c85807ab53bd9e5
- Result: failed (exit code 101)
- Source revision: 3efcbaf7ba1c61959dbd07e396d8599bbf6b4116
- Command: cargo test --manifest-path resilient/Cargo.toml --locked --features z3 --test proof_coverage_audit integration_zeroarity_bv_branch_checkpoints_are_supported -- --exact --nocapture

## Toolchain

- cargo: cargo 1.99.1 (5f94df478 2026-08-27)
- python: Python 3.14.4
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)

## Inputs

- resilient/tests/proof_coverage_audit.rs: fb6784849f97b759565deec3ad726d3fb550dc180066891b205e0383c8a35a03

## Outcome

- command: failed
- integration_zeroarity_bv_branch_checkpoints_are_supported: failed
