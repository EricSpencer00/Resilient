---
id: RES-189
title: LSP: inlay hints for inferred `let` types
state: DONE
priority: P3
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
Once RES-123 lands (inferred return types) and RES-120
(inference), users will omit annotations. Inlay hints show the
inferred type inline as editor chrome — `let x = 3 + 2  :: Int`.

## Acceptance criteria
- `Backend::inlay_hint` returns a list of `InlayHint` values over
  a requested range.
- Emit one hint per `let` binding that lacks an explicit type
  annotation, with the inferred type as the label and position at
  the end of the pattern.
- Parameter hints: at a call site, label each positional arg with
  the corresponding param name (`add(a: 1, b: 2)`-style chrome).
  Off by default, behind a workspace config setting
  `resilient.inlayHints.parameters: bool`.
- Integration test: 5-let snippet, 3 hints expected.
- Commit message: `RES-189: LSP inlay hints for inferred types`.

## Notes
- Hints must not interfere with diagnostics — they're purely
  visual. Don't introduce any new AST passes; reuse the inference
  cache from RES-120.
- Client behavior varies on when to refresh — we respect
  `inlayHint/refresh` notifications.

## Resolution

### Approach
The ticket references a non-existent "inference cache from RES-120".
RES-120 is bailed (blocked on RES-119 + NodeId). Rather than wait,
I landed this with the **existing typechecker**: the per-statement
walk already computes every `Node::LetStatement`'s value type —
just didn't surface it. Adding a side-channel is a ~10-line change
and matches what a real inference cache would expose anyway.

### Files changed
- `resilient/src/typechecker.rs`
  - New public struct `LetTypeHint { span, name_len_chars, ty }`.
  - New field on `TypeChecker`: `pub let_type_hints: Vec<LetTypeHint>`.
  - `Node::LetStatement` case: when `type_annot.is_none()` AND the
    inferred value type is not `Any` / `Void` / `Var`, push a hint.
  - Skip criteria protect editor UX: `Any`-typed hints are noisy,
    `Void` shouldn't happen for a let, `Var` would leak inference
    artifacts.

- `resilient/src/lsp_server.rs`
  - New `Backend.inlay_hint_parameters: Mutex<bool>` — opt-in flag
    for call-site parameter hints (defaults off per ticket). Read
    from `InitializeParams.initialization_options` in both flat
    (`{"resilient.inlayHints.parameters": true}`) and nested
    (`{"resilient": {"inlayHints": {"parameters": true}}}`) forms.
  - Capability advertisement: `inlay_hint_provider: Some(OneOf::Right(
    InlayHintServerCapabilities::Options(InlayHintOptions { ... })))`
    with `resolve_provider: Some(false)` — we emit full labels up
    front, no resolve dance needed.
  - `Backend::inlay_hint` handler: runs the typechecker on the
    cached AST, converts `let_type_hints` to `InlayHint`s, adds
    parameter hints if opted in, filters by the request's range.
  - New helpers:
    - `inlay_hint_from_let(&LetTypeHint) -> InlayHint` — positions
      at end-of-pattern (`let_kw_col + 4 + name_len_chars`) with
      label `": <type>"`.
    - `collect_param_hints(program) -> Vec<InlayHint>` — walks
      calls and top-level fns, emitting one hint per positional
      arg when arity matches. Unknown callees and arity mismatches
      are skipped (any pairing would be misleading).
    - `collect_top_level_fns`, `walk_call_hints`, `expression_span`
      — helpers for the AST walk.
    - `read_init_param_hints_flag(opts) -> bool` — parses the two
      init-option forms.
    - `position_in_range(p, r) -> bool` — viewport filter.

- `resilient/Cargo.toml` — added `serde_json = "1"` to
  `[dev-dependencies]` so the new init-option unit tests can build
  JSON values via `json!` macro. (tower-lsp pulls serde_json in
  transitively but the macro needs a direct edge for name
  resolution in the test module.)

- `resilient/tests/lsp_smoke.rs` — two new end-to-end tests:
  - `lsp_inlay_hint_types_for_unannotated_lets` — AC canary. 5-let
    snippet (3 unannotated + 2 annotated); asserts exactly 3 TYPE
    hints (`kind":1`) with labels `": int"`, `": bool"`, `": string"`
    and NO parameter hints without opt-in.
  - `lsp_inlay_hint_parameter_hints_opt_in` — opts in via
    `initializationOptions`, defines `fn add(int a, int b)`, calls
    `add(1, 2)`, asserts PARAMETER hints (`kind":2`) with labels
    `"a: "` and `"b: "`.

### Tests
- **Unit (10 new)**:
  - `typechecker_collects_hints_for_unannotated_lets` (5-let AC at
    the TC level)
  - `typechecker_skips_any_typed_let_hints`
  - `inlay_hint_from_let_positions_after_identifier`
  - `param_hints_tag_each_arg_with_param_name`
  - `param_hints_skip_arity_mismatches`
  - `param_hints_skip_unknown_callees`
  - `read_init_param_hints_flag_flat_form`
  - `read_init_param_hints_flag_nested_form`
  - `read_init_param_hints_flag_defaults_false`
  - `position_in_range_inclusive_endpoints`
- **Integration (2 new)**: the two `lsp_smoke` tests above.

### Verification
- `cargo test --locked` → 488 passed (unchanged — the new work is
  all under `lsp`)
- `cargo test --locked --features lsp` → 515 passed (was 505,
  +10 new unit tests) + 8 lsp_smoke (was 6, +2 new)
- `cargo clippy --locked --features lsp,z3,logos-lexer --tests
  -- -D warnings` → clean
- The `random_int_is_deterministic_under_same_seed` test is known-
  flaky under parallel test execution (sibling random-RNG tests
  don't acquire the `RNG_TEST_LOCK` — orthogonal to this ticket);
  it passes on isolated runs. Not introduced by this change.

### Follow-ups (not in this ticket)
- Parameter hints for imported fns and method calls — will need
  RES-182's unified name-resolution table.
- Struct-destructure let hints (`let Point { x, y } = p;`) — the
  current walker only handles the simple `let <name> = ...` form.
- Pattern-end precision for `let mut` or future `let pat: …` —
  current approximation assumes `"let " + name` and would drift
  for richer patterns.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 resolved by executor (let-type hints via typechecker
  side-channel; opt-in parameter hints; 10 unit + 2 integration
  tests)
