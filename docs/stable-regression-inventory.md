# Stable Regression Inventory

This file tracks the current stable Resilient language and CLI surface
and the direct regression or smoke coverage that pins each area.

Scope rules:

- Track shipped surfaces users are expected to rely on today.
- Prefer direct integration or smoke coverage over incidental unit-test
  coverage.
- When a surface is not yet stable, call it out explicitly instead of
  treating it as uncovered stable behavior.

## Stable Language Surface

| Surface | Direct coverage | Status | Notes |
|---|---|---|---|
| Default interpreter execution path | `resilient/tests/examples_smoke.rs` | Covered | End-to-end run path over shipped example programs. |
| Older stable core constructs from the roadmap reset backlog: assignment, forward references, modulo, prefix `!`/`-`, logical ops, bitwise ops, `while`, block comments, hex/binary literals with `_` separators | `resilient/tests/stable_language_surface_backfill.rs` | Covered | Added for RES-3128 to backfill older stable syntax/runtime surfaces that previously relied on incidental coverage. |
| Older stable core constructs: `static let`, bare `return;`, string comparisons, `len()` | `resilient/tests/stable_language_surface_backfill.rs` | Covered | Added for RES-3128. |
| Live blocks and retry semantics | `resilient/tests/live_block_spec.rs`, `resilient/tests/live_retry_log_cli.rs`, `resilient/tests/panic_on_fault_smoke.rs` | Covered | Runtime healing path plus emitted retry log / panic-on-fault modes. |
| Contracts, bounds proof audit, and safety-critical checking | `resilient/tests/bounds_elision_smoke.rs`, `resilient/tests/safety_critical_smoke.rs` | Covered | Pins strict verification/audit behavior. |
| Effects and effect explanation | `resilient/tests/effect_system_smoke.rs`, `resilient/tests/explain_effects_cli.rs` | Covered | Direct effect-system and CLI explanation coverage. |
| `#[cfg(...)]` conditional compilation | `resilient/tests/cfg_smoke.rs` | Covered | Direct stable cfg path coverage. |
| Linear types | `resilient/tests/linear_types.rs` | Covered | Direct end-to-end typechecker coverage. |
| `try` / `catch` runtime semantics | `resilient/tests/try_catch_runtime.rs` | Covered | Direct runtime coverage. |
| Recovery contracts / `recovers_to` | `resilient/tests/recovers_to_smoke.rs`, `resilient/tests/recovers_to_z3_obligation.rs` | Covered | Runtime and verifier paths. |
| Information flow / noninterference | `resilient/tests/info_flow_smoke.rs`, `resilient/tests/noninterference_smoke.rs` | Covered | Direct end-to-end policy checks. |

## Stable CLI Surface

| Surface | Direct coverage | Status | Notes |
|---|---|---|---|
| Global help and REPL discoverability | `resilient/tests/repl_smoke.rs`, `resilient/tests/safety_critical_smoke.rs` | Covered | Pins `rz --help`, `rz repl --help`, and explicit REPL alias text. |
| REPL startup path | `resilient/tests/safety_critical_smoke.rs` | Covered | Direct `rz repl` launch smoke. |
| `rz check <file>` | `resilient/tests/check_smoke.rs` | Covered | Success, type-error, quiet, and usage paths. |
| `--typecheck-strict` | `resilient/tests/typecheck_strict_smoke.rs` | Covered | Pins the fatal typecheck path that turns soft diagnostics into a hard error. |
| `rz fmt <file>` stdout path | `resilient/tests/roundtrip.rs` | Covered | Canonical round-trip formatting. |
| `rz fmt <file> --in-place` | `resilient/tests/stable_cli_surface_smoke.rs` | Covered | Added for RES-3128. |
| `rz lint <file>` | `resilient/tests/lint_smoke.rs` | Covered | Dedicated CLI wiring coverage. |
| `--dump-tokens` | `resilient/tests/dump_tokens_smoke.rs` | Covered | Direct lexer-stream smoke. |
| `--dump-ast-json` | `resilient/tests/stable_cli_surface_smoke.rs`, `resilient/tests/self_host_parity.rs` | Covered | Added dedicated CLI smoke in RES-3128; parity suite already consumes the JSON shape. |
| `--dump-chunks` | `resilient/tests/dump_chunks_smoke.rs` | Covered | Direct VM disassembly smoke. |
| `--audit` | `resilient/tests/bounds_elision_smoke.rs` | Covered | Direct verification audit coverage. |
| `--explain-effects` | `resilient/tests/explain_effects_cli.rs` | Covered | Dedicated CLI coverage. |
| `--version` / `--version --verbose` | `resilient/tests/stable_cli_surface_smoke.rs` | Covered | Added for RES-3128. |
| `stack-usage <file>` | `resilient/tests/stable_cli_surface_smoke.rs` | Covered | Added for RES-3128. |
| `pkg init` workflow | `resilient/tests/pkg_init_smoke.rs` | Covered | Dedicated project-scaffolding smoke. |
| `bench <file>` | `resilient/tests/bench_cli.rs`, `resilient/tests/bench_cli_summary_json.rs` | Covered | Human and machine-readable outputs. |
| Example corpus smoke | `resilient/tests/examples_smoke.rs`, `resilient/tests/examples_golden.rs` | Covered | Direct shipped-example coverage. |
| Self-host parity report | `resilient/tests/self_host_parity_report_cli.rs` | Covered | Dedicated report artifact smoke. |
| Certificate verification (`verify-cert`, `verify-all`) | `resilient/tests/verify_cert_smoke.rs`, `resilient/tests/verify_all_smoke.rs` | Covered | Stable when built with `--features z3`. |
| Backend-limited execution (`--vm`, `--jit`) | `resilient/tests/examples_smoke.rs` | Covered | Stable subset / feature-gated, but still pinned by direct smoke. |
| LSP server | `resilient/tests/lsp_smoke.rs` and focused LSP smoke files | Covered | Feature-gated shipped surface. |

## Intentionally Deferred / Not Yet Stable

| Surface | Reason | Follow-up shape |
|---|---|---|
| `rz test` | `docs/tooling.md` still classifies the first-class test runner as future work, so it is not counted as part of the stable surface in this inventory. | Reclassify in docs and add dedicated CLI smoke when the runner is promoted to stable. |
| `rz tool ...` external tool bridge | Present in code but not documented as stable in the public tooling reference or top-level help. | Promote/document the subcommand first, then add direct smoke coverage as part of that stabilization slice. |
| Debugger / profiler paths | Public docs classify them as future work. | Stabilize the user-facing workflows before adding them to this inventory. |

## Maintenance Rule

When a change ships new stable language behavior or a new stable CLI
workflow, add or update a direct regression/smoke test in the same PR
and update this inventory so the coverage source of truth stays current.
