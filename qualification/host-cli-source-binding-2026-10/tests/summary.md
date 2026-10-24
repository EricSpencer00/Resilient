# End-to-end evidence

- Run ID: 9acb20e24d23d96524aa5666bd5bd4b07a689d6e5d5dc34c1b1725117649a228
- Result: passed (exit code 0)
- Source revision: be7d1302d66bb9061e497ffc6df6d9b57660d298
- Command: cargo test --manifest-path resilient/Cargo.toml --locked --features z3 --test contract_replay_roundtrip --test manifest_authentication --test loop_certificate_roundtrip --test finite_loop_certificate_guards --test solver_replay_bounds --test solver_replay_required --test solver_replay_conformance --test source_binding --test source_binding_graph

## Toolchain

- cargo: cargo 1.99.1 (5f94df478 2026-08-27)
- python: Python 3.14.4
- rustc: rustc 1.99.1 (58cf8f9c3 2026-10-13)

## Inputs

- Cargo.lock: c0fd47234d3e20dcfab556170e0f9ece69e41f5c67de537d25b3aee938ec0626
- resilient/tests/contract_replay_roundtrip.rs: 1a8c0fd584b41abc243ebde1b5a839beee754b2f0501bac9f5d0fc396f219601
- resilient/tests/finite_loop_certificate_guards.rs: 6f22c08adfe3e60e7108ded5dbb43d61ef5021e0ee0328d251030bd9d403b355
- resilient/tests/loop_certificate_roundtrip.rs: 5db750dc91cb1ff5af579061379373368422a0412c6c5c7bc3971113c789334b
- resilient/tests/manifest_authentication.rs: 665d1e8f157aaea83b085a50ca4b92b3b23ceadd4e519aac46ed25922b3f851b
- resilient/tests/solver_replay_bounds.rs: 76cf986da4d530b91024440907d9d94fdd88aa05afcbb7f7dac6dbc7578d06a6
- resilient/tests/solver_replay_conformance.rs: ce5e3a25e43cd51455dbd0abdfb20e53f0d6649235a7e02ffc9d7af27f61999f
- resilient/tests/solver_replay_required.rs: a8db20647cdd0e8fbf68c8323944770997ddf3992d945e878d45d880c2f942c0
- resilient/tests/source_binding.rs: c1b9b00e863f60c530fe902a04fb1ae4a946ce701663432c82d39ec76ca3d066
- resilient/tests/source_binding_graph.rs: e6d72cac446b7c11d0c0e5a74b7d90355283f00b8aa4213b1458972bdb70d772

## Outcome

- absolute_target_is_rejected: passed
- branch_returns_replay_separately: passed
- cache_and_no_cache_emit_identical_binding: passed
- changed_entry_is_rejected: passed
- changed_import_is_rejected: passed
- changing_iteration_binding_is_not_certified: passed
- coherently_omitted_import_is_rejected: passed
- command: passed
- compiler_signs_the_manifest_for_strict_replay: passed
- constant_return_replays: passed
- cyclic_graph_and_excessive_inventory_are_bounded: passed
- duplicate_edge_is_rejected: passed
- duplicate_inventory_file_is_rejected: passed
- empty_iteration_does_not_claim_an_executed_invariant_proof: passed
- extra_edge_is_rejected: passed
- false_loop_invariant_does_not_emit_a_proof: passed
- false_postcondition_is_rejected: passed
- finite_loop_certificates_replay_with_real_solver: passed
- hash_only_verification_does_not_require_solver: passed
- identical_relocated_sources_verify: passed
- legacy_replay_does_not_require_a_manifest_signature: passed
- legacy_replay_remains_compatible_without_binding: passed
- malformed_inventory_is_rejected: passed
- missing_import_edge_is_rejected: passed
- missing_inventory_is_rejected: passed
- missing_source_and_invalid_root_are_rejected: passed
- nested_diamond_graph_relocates_with_its_common_root: passed
- outside_root_fifo_is_rejected_without_opening_it: passed
- package_resolution_graph_does_not_invoke_git: passed
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
- signed_inventory_binds_all_parsed_inputs: passed
- straight_return_replays: passed
- strict_replay_accepts_the_complete_signed_manifest: passed
- strict_replay_rejects_changed_program_identity: passed
- strict_replay_rejects_missing_manifest_signature: passed
- strict_replay_rejects_removed_obligations: passed
- strict_replay_rejects_reordered_obligations: passed
- strict_replay_rejects_wrong_manifest_key: passed
- symlink_import_outside_root_is_rejected_before_read: passed
- traversal_inventory_path_is_rejected: passed
- unreachable_inventory_file_is_rejected: passed
- unsigned_inventory_is_rejected: passed
- unsupported_version_and_empty_inventory_are_rejected: passed
- valid_iteration_binding_retains_runtime_check: passed
- while_invariant_certificates_replay_with_real_solver: passed
