# End-to-end evidence

- Run ID: e5abecac0d7b1d1b33f2a920627e18367dab0f4dc4e1efec3e5025f9b433b527
- Result: failed (exit code 101)
- Source revision: 3efcbaf7ba1c61959dbd07e396d8599bbf6b4116
- Command: cargo test --manifest-path resilient/Cargo.toml --locked --features z3 --test proof_coverage_audit integration_bv_wrong_guard_rejects_valid_unsat_subset -- --exact --nocapture

## Toolchain

- cargo: cargo 1.99.1 (5f94df478 2026-08-27)
- python: Python 3.14.4
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)

## Inputs

- resilient/tests/proof_coverage_audit.rs: fb6784849f97b759565deec3ad726d3fb550dc180066891b205e0383c8a35a03

## Outcome

- command: failed
- integration_bv_wrong_guard_rejects_valid_unsat_subset: failed
