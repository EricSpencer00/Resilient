# End-to-end evidence

- Run ID: d7d0bb951ea73c125792ee7ae9a1266b724d75f92c3311e79547af971e011a19
- Result: passed (exit code 0)
- Source revision: 821b5513382d5d471fc787c56308cf4a90388d7f
- Command: cargo test --locked --manifest-path resilient/Cargo.toml --features z3 --test it verify_all_smoke

## Toolchain

- cargo: cargo 1.98.1 (797e8a9bc 2026-08-05)
- python: Python 3.12.3
- rustc: rustc 1.98.1 (48a229cea 2026-09-01)

## Inputs

- Cargo.lock: b4f6e62a8d081da34b6d45174a8ea964f85eb9e128a4d8ac2bf2d8752e6de58f
- resilient/src/cert_sign.rs: 3bbcd5a08981fd6077fba498f8f222cb26b8390262895e7a7efa4a9b3b4f2336
- resilient/tests/it/verify_all_smoke.rs: 6d5f7e5e5d76eabf378d1f6a0a94e0d219c65955ee1182768336f437c7be5be3

## Outcome

- command: passed
- verify_all_smoke::emit_certificates_writes_manifest_json: passed
- verify_all_smoke::verify_all_detects_sha256_mismatch: passed
- verify_all_smoke::verify_all_detects_signature_tamper: passed
- verify_all_smoke::verify_all_errors_on_malformed_manifest: passed
- verify_all_smoke::verify_all_errors_on_missing_manifest: passed
- verify_all_smoke::verify_all_happy_path_signed: passed
- verify_all_smoke::verify_all_happy_path_unsigned: passed
- verify_all_smoke::verify_all_requires_directory_argument: passed
