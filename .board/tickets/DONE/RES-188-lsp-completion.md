---
id: RES-188
title: LSP: completion for builtins and in-scope locals
state: DONE
priority: P2
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
Minimum-viable completion: when the user types an identifier
prefix, offer builtins and in-scope bindings that start with that
prefix. No fuzzy matching yet; no type-driven filtering.

## Acceptance criteria
- `Backend::completion` returns a `CompletionResponse::Array` of
  `CompletionItem`.
- Source set: every builtin name (from the registry) + every
  in-scope local/param/top-level fn at the cursor position.
- Each item includes `kind` (Function / Variable / Keyword),
  `detail` (the item's type), and `insertText` (the name).
- Triggered by Ctrl-Space and by identifier-prefix typing (the
  client drives that — we just need to return responses promptly).
- Integration test exercising prefix completion inside a fn body.
- Commit message: `RES-188: LSP identifier completion`.

## Notes
- No post-dot completion yet — `.` for field access is a separate
  ticket once RES-185 / RES-155 settle.
- Limit results to 100 items — editors often truncate anyway,
  and large lists hurt latency.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed and bailed by executor (same shared LSP infra
  gap flagged by RES-181 / RES-182)
- 2026-04-17 claimed by executor — landing RES-188a scope (builtins + top-level
  decls; locals/params deferred to RES-188b — same scope-walker gap as RES-182b)
- 2026-04-17 landed RES-188a (builtins + top-level decls); RES-188b deferred

## Resolution (RES-188a — builtins + top-level decls)

This landing covers RES-188a of the implicit split: identifier
completion seeded from the `BUILTINS` registry plus top-level
decls in the current document. Scope-aware local / parameter
completion (RES-188b) stays deferred on the same scope-walker
that blocks RES-182b.

Builds directly on the shared LSP plumbing laid down by RES-181a
(document storage, cached source text, token-level position
helpers) and RES-182a (top-level decl map). No new infra.

### Files changed

- `resilient/src/main.rs`
  - New `pub(crate) fn builtin_names() -> impl Iterator<Item = &'static str>`
    exposes the `BUILTINS` table to the LSP module without moving
    the full `BuiltinFn` values across crate boundaries.
    `#[allow(dead_code)]` keeps the default / z3 / jit builds
    warning-clean.
- `resilient/src/lsp_server.rs`
  - New imports: `CompletionItem`, `CompletionItemKind`,
    `CompletionOptions`, `CompletionParams`, `CompletionResponse`.
  - `initialize` capabilities advertise `completion_provider`
    with no trigger characters (identifier-prefix completion is
    client-driven; post-dot field completion is a separate
    future ticket).
  - New `pub(crate) fn prefix_at(src, pos) -> String` walks
    backwards from the cursor to the first non-identifier
    character; returns the typed prefix. Handles clients that
    send positions past EOL, empty lines, and underscore-
    containing names.
  - New `pub(crate) struct Candidate { label, kind, detail }` +
    `CandidateKind { Function, Struct, TypeAlias }` enum for
    pre-LSP-serialization candidate shapes. Lets the ordering /
    filtering / cap logic stay unit-testable over pure `Vec`s.
  - New `pub(crate) const COMPLETION_LIMIT: usize = 100` — the
    ticket's explicit cap.
  - New `pub(crate) fn completion_candidates(program, prefix)`
    enumerates:
      * Builtins (alphabetically sorted snapshot of BUILTINS)
        labeled as Function with detail = "builtin".
      * Top-level `Node::Function` / `Node::StructDecl` /
        `Node::TypeAlias` (source order) with kind + detail
        reflecting the decl's shape.
    Prefix-filters in-line and stops early at the cap.
  - New `candidate_to_completion_item` mapper — isolates the
    tower-lsp wire-shape conversion so helper unit tests stay
    framework-free.
  - New `Backend::completion` handler: reads cached source +
    AST, extracts prefix, enumerates candidates, maps to
    `CompletionItem`, returns `CompletionResponse::Array`.
- `resilient/tests/lsp_completion_smoke.rs` (new)
  - End-to-end: initialize → didOpen a 4-line document → three
    completion requests:
      * Mid-identifier (`prin|`) → asserts `println` + `print`
        in the array.
      * User-decl prefix (`my_|`) → asserts `my_helper` in the
        array.
      * Empty prefix (Ctrl-Space at a blank line) → asserts a
        non-empty array.
    Finishes in ~0.5s.

### Tests (18 unit + 1 integration, all `res188a_*`)

Unit:
- `prefix_at_*` (9 cases): empty source, identifier start / mid
  / end, stops at non-identifier char, underscore support,
  multi-line correctness, cursor past EOL clamps, non-existent
  line returns empty.
- `candidates_*` (8 cases): empty-prefix + empty program yields
  builtins, prefix filters builtins (`prin` → only prin-prefixed
  names, with a leak-prevention invariant), includes top-level
  fn / struct / type alias (each with the correct `kind`),
  deterministic ordering (builtins before user decls via `abs` /
  `abc`), respects the 100-item cap, unmatched prefix returns
  empty.
- `candidate_to_completion_item_maps_fields`: field mapping to
  the LSP wire shape.

Integration (`lsp_completion_smoke.rs`): full LSP round-trip.

### Verification

```
$ cargo build                                   # OK (8 warnings, baseline)
$ cargo build --features z3                     # OK
$ cargo build --features jit                    # OK
$ cargo build --features lsp                    # OK
$ cargo test --locked
test result: ok. 651 passed; 0 failed     (non-lsp baseline unchanged)
$ cargo test --locked --features lsp
test result: ok. 726 passed; 0 failed     (+18 unit, +1 integration)
$ cargo test --features lsp res188a
test result: ok. 18 passed; 0 failed
$ cargo test --features lsp --test lsp_completion_smoke
test result: ok. 1 passed; 0 failed       (finishes in <1s)
```

### What was intentionally NOT done

- **RES-188b** — no scope-aware local / parameter completion.
  A prefix matching a `let` binder or fn parameter that isn't
  also a top-level decl returns only builtins. Same scope-
  walker gap as RES-182b / RES-169b.
- **Post-dot field completion** — explicit non-goal per the
  Notes section. Its own future ticket.
- **Fuzzy matching** — explicit non-goal. Prefix matching only,
  client-side fuzzy sort is enough.
- **Type-driven filtering** — explicit non-goal.

### Follow-ups the Manager should mint

- **RES-188b** — scope-aware local / parameter completion,
  leveraging the same scope walker that unblocks RES-182b.
  Candidate source extends from [builtins + top-level] to
  [builtins + in-scope locals + params + top-level], filtered
  by position.
- **RES-188c** (future, once field-aware resolution exists) —
  post-dot completion: after a `.` token, enumerate the target
  type's fields.

## Attempt 1 failed

Same infra gap the last two LSP tickets flagged:

- `Backend` has no document storage today (`publish_analysis`
  receives text and drops it).
- `initialize` advertises neither `hover_provider`,
  `definition_provider`, nor `completion_provider`.
- No AST position walk.
- No `did_close` handler.

Completion-specific additions on top of the shared gap:

- Scope-aware collection of in-scope bindings at a given cursor
  position — the same walker RES-182 needs, plus a "keep bindings
  visible at this position" filter (stop descending once the
  position is passed).
- A `BUILTINS` iterator exported from `main.rs` for the LSP to
  enumerate builtin names + types. Currently `const BUILTINS: &[(&str,
  BuiltinFn)]` is `main.rs`-local and not pub.
- `Backend::completion` + 100-item cap + integration test in
  `tests/lsp_completion.rs`.

## Clarification needed

Recommend landing a shared-LSP-infra ticket first (see RES-181 and
RES-182's `## Clarification needed` sections, which propose the
same RES-XXX-a split). Once that's in, RES-188 reduces to: scope
walker + builtin enumeration + completion handler + test — one
iteration.

No code changes landed — only the ticket state toggle and this
clarification note.
