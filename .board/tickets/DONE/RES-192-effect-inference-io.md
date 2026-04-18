---
id: RES-192
title: IO-effect inference: flag functions that reach `println` or file_*
state: DONE
priority: P3
goalpost: G18
created: 2026-04-17
owner: executor
---

## Summary
`@pure` (RES-191) is opt-in and checked. Going further: infer an
effect set for every fn. The MVP tracks one effect — `IO` — and
colors every transitively-IO fn. User-facing surface is an
LSP inlay hint and `--audit` column.

## Acceptance criteria
- Effect lattice for now: `{}` (pure) or `{IO}`. Operators are
  union.
- Pass: fixpoint over the call graph. Builtins pre-populated (from
  RES-191's table). User fns aggregate effects of their body.
- Reported via:
  - `--audit` gains an "effects" column.
  - LSP hover (extension of RES-181) appends
    `[effects: IO]` when non-empty.
- Unit tests: a chain `caller -> helper -> println` tagged IO at
  every step; a leaf fn that only does arithmetic tagged pure.
- Commit message: `RES-192: IO effect inference`.

## Notes
- Keep the lattice small (binary). Adding more effects (MEM,
  ALLOC, PANIC) is follow-up work and requires careful user-level
  documentation.
- Don't error on IO — just report. `@pure` is the error path; this
  is informational.

## Resolution

### Approach
Binary effect lattice (`pure`/`IO`) inferred by fixpoint over the
call graph. Reuses the `IMPURE_BUILTINS` + `is_known_pure_builtin`
tables from RES-191; a user fn is tagged IO iff any call site
reaches an impure builtin, another IO user fn, or an unresolvable
callee (method / indirect — conservatively IO).

### Files changed
- `resilient/src/typechecker.rs`
  - New public `infer_fn_effects(statements) -> HashMap<String, bool>`
    — fixpoint over the call graph. Bounded at `|fns| + 1` passes
    since each pass can only flip pure→IO once per fn.
  - New private `body_reaches_io(node, effects)` — recursive AST
    walker used inside the fixpoint. Covers every expression /
    statement shape: calls (impure builtin / IO user-fn /
    unknown callee), literals, control flow, LiveBlock (inherits
    its body's effects, not intrinsically IO), field/index
    mutation (not IO by RES-192's definition), Match arms, etc.
  - `VerificationStats` gained `pub fn_effects: HashMap<String,
    bool>`.
  - `check_program_with_source` calls `infer_fn_effects` after
    the regular walk + purity pass, populating the stats field.
- `resilient/src/main.rs` — `print_verification_audit` now prints
  an "effects (inferred)" block: counter of IO-reaching fns over
  total fns, then one line per user fn with `[effects: IO]`
  (yellow) or `[effects: {}]` (green). Table is sorted for
  stable output across runs and omitted entirely when the
  program has no user fns.

### Policy notes
- **LiveBlock is NOT intrinsically IO.** A `live { return x; }`
  with a pure body stays pure. The ticket's definition is "reach
  println or file_*"; retries alone don't qualify. A `live` block
  whose body contains IO still propagates — verified by
  `live_block_alone_is_not_io` + `live_block_with_io_body_is_io`
  tests.
- **Unknown user fn = IO.** If a call site resolves to a bare
  identifier not in the top-level fn table (rare — typechecker
  would have rejected earlier), we conservatively flag IO.
- **Indirect / method callees = IO.** Same conservative default.
- **Pure builtins that don't flow through `is_known_pure_builtin`**
  — none today, but if a new builtin is added to `BUILTINS`
  without updating either list, it'll count as IO. Matches the
  "conservative by default" stance.

### Scope deviation from AC
The ticket's "LSP hover (extension of RES-181) appends
`[effects: IO]` when non-empty" is **deferred**: RES-181 (LSP
hover) is bailed, so there's no existing hover to extend. Landing
a hover surface is not in scope for RES-192 — the ticket's dep
was "extension", i.e. additive to something that should already
exist.

The `stats.fn_effects` HashMap is public, so once RES-181 (or its
rewrite) lands, the hover handler needs ~5 lines to append the
inferred-effect tag. Logged as a follow-up below.

### Tests
11 new in `typechecker::purity_tests` (module name predates
RES-192 but covers both purity + effects now — they share the
`IMPURE_BUILTINS` table):
- `effect_chain_propagates_io_transitively` — AC canary: 3-level
  chain `top → caller → helper → println` → all 3 IO.
- `arithmetic_only_leaf_is_pure` — AC canary: pure leaf.
- `fixpoint_handles_mutual_recursion` — mutual `a ↔ b` with no
  IO stays pure (fixpoint terminates).
- `io_reaches_through_mutual_recursion` — same shape but `b`
  calls `println` → both `a` and `b` IO.
- `file_io_builtins_flag_io`, `clock_and_random_flag_io` —
  coverage for the rest of IMPURE_BUILTINS.
- `pure_builtin_calls_stay_pure` — `abs` / `min` don't flip.
- `live_block_alone_is_not_io` — policy pin.
- `live_block_with_io_body_is_io` — policy pin.
- `empty_program_produces_empty_effects` — edge case.
- `stats_field_populated_by_full_check` — end-to-end via
  `check_program_with_source`.

### Manual audit output
```
$ cat /tmp/p.rs
@pure
fn square(int x) { return x * x; }
fn helper(int x) { println("h"); return x; }
fn caller(int x) { return helper(x); }
fn main(int _d) { return caller(3); } main(0);
$ resilient -t --audit --seed 0 /tmp/p.rs
Type check passed

--- Verification Audit ---
  ...
  effects (inferred): 3 / 4 fns reach IO
    caller                       [effects: IO]
    helper                       [effects: IO]
    main                         [effects: IO]
    square                       [effects: {}]
h
Program executed successfully
```

### Verification
- `cargo build` → clean
- `cargo test --locked` → 524 + 16 + 4 + 3 + 1 + 12 + 4 + 5 tests
  pass (was 513 core; +11 new effect tests)
- `cargo test --locked --features lsp` → 551 + 16 + 4 + 3 + 1 +
  12 + 8 + 4 + 5 pass
- `cargo clippy --locked --features lsp,z3,logos-lexer --tests
  -- -D warnings` → clean

### Follow-ups (not in this ticket)
- **LSP hover integration** — add `[effects: IO]` to hover text
  once RES-181 lands. `stats.fn_effects` is already exposed; the
  hover handler just needs to look up the name under the cursor.
- **More effects** (`MEM`, `ALLOC`, `PANIC`). Ticket Notes call
  this out explicitly as follow-up work "requires careful user-
  level documentation".
- **Annotation-backed effect set.** RES-193 extends the lattice
  with effect polymorphism for higher-order fns. Currently
  blocked on RES-124 (generic `fn<T>` syntax) + RES-120.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 resolved by executor (fixpoint effect inference +
  --audit column; 11 unit tests; LSP hover deferred on RES-181's
  bail)
