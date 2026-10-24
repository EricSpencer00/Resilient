# End-to-end evidence

- Run ID: c74ae18b6c533b5714d4b544e355b9066d441058031a465e346b1a72f81eb183
- Result: passed (exit code 0)
- Source revision: 6ad36dfb4b3a9c5d8c2ff396c3d35335e649e3a3
- Command: cargo test --manifest-path resilient/Cargo.toml --locked --features z3 --test contract_replay_roundtrip --test manifest_authentication --test loop_certificate_roundtrip --test finite_loop_certificate_guards --test solver_replay_bounds --test solver_replay_required --test solver_replay_conformance

## Toolchain

- cargo: cargo 1.99.1 (5f94df478 2026-08-27)
- python: Python 3.14.4
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)

## Inputs

- Cargo.lock: b4f6e62a8d081da34b6d45174a8ea964f85eb9e128a4d8ac2bf2d8752e6de58f
- resilient/tests/contract_replay_roundtrip.rs: 1a8c0fd584b41abc243ebde1b5a839beee754b2f0501bac9f5d0fc396f219601
- resilient/tests/finite_loop_certificate_guards.rs: 6f22c08adfe3e60e7108ded5dbb43d61ef5021e0ee0328d251030bd9d403b355
- resilient/tests/loop_certificate_roundtrip.rs: 5db750dc91cb1ff5af579061379373368422a0412c6c5c7bc3971113c789334b
- resilient/tests/manifest_authentication.rs: 665d1e8f157aaea83b085a50ca4b92b3b23ceadd4e519aac46ed25922b3f851b
- resilient/tests/solver_replay_bounds.rs: 76cf986da4d530b91024440907d9d94fdd88aa05afcbb7f7dac6dbc7578d06a6
- resilient/tests/solver_replay_conformance.rs: ce5e3a25e43cd51455dbd0abdfb20e53f0d6649235a7e02ffc9d7af27f61999f
- resilient/tests/solver_replay_required.rs: a8db20647cdd0e8fbf68c8323944770997ddf3992d945e878d45d880c2f942c0

## Outcome

- branch_returns_replay_separately: passed
- changing_iteration_binding_is_not_certified: passed
- command: passed
- compiler_signs_the_manifest_for_strict_replay: passed
- constant_return_replays: passed
- empty_iteration_does_not_claim_an_executed_invariant_proof: passed
- false_loop_invariant_does_not_emit_a_proof: passed
- false_postcondition_is_rejected: passed
- finite_loop_certificates_replay_with_real_solver: passed
- hash_only_verification_does_not_require_solver: passed
- legacy_replay_does_not_require_a_manifest_signature: passed
- replay_accepts_clean_response_within_budget: passed
- replay_accepts_one_clean_unsat_response: passed
- replay_rejects_conflicting_or_extra_responses: passed
- replay_rejects_invalid_timeout_configuration: passed
- replay_rejects_invalid_utf8_after_unsat: passed
- replay_rejects_nonproof_responses: passed
- replay_rejects_oversized_whitespace_after_unsat: passed
- replay_rejects_response_after_deadline: passed
- replay_rejects_stderr_even_after_unsat: passed
- replay_rejects_unsat_from_failed_process: passed
- requested_replay_rejects_empty_obligation_set: passed
- requested_replay_rejects_missing_absolute_solver: passed
- requested_replay_rejects_solver_absent_from_path: passed
- shadowed_iteration_binding_is_not_certified: passed
- straight_return_replays: passed
- strict_replay_accepts_the_complete_signed_manifest: passed
- strict_replay_rejects_changed_program_identity: passed
- strict_replay_rejects_missing_manifest_signature: passed
- strict_replay_rejects_removed_obligations: passed
- strict_replay_rejects_reordered_obligations: passed
- strict_replay_rejects_wrong_manifest_key: passed
- valid_iteration_binding_retains_runtime_check: passed
- while_invariant_certificates_replay_with_real_solver: passed
