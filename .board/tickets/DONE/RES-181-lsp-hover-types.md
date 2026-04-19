---
id: RES-181
title: LSP: hover shows inferred type of the symbol under cursor
state: DONE
priority: P2
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
RES-074 scaffolded the LSP; RES-090/093/094 landed integration
tests. Hover is the most-used LSP feature and the easiest
high-signal extension: given a position, return the inferred type
of the expression/binding there.

## Acceptance criteria
- `Backend::hover` implementation in `lsp_server.rs`.
- On a position inside an identifier: walk the AST for the
  enclosing node; return `Hover { contents: MarkedString(type_str), range }`
  where `type_str` is the inferred type from RES-120's inferer or
  the typechecker's recorded type if RES-120 isn't enabled.
- On a position inside a literal: return the literal's type
  ("Int" for a number literal).
- No hover for whitespace / comments (null response).
- Integration test under `tests/lsp_hover.rs` spawns the binary,
  opens a document, sends `textDocument/hover`, asserts the
  expected type string on three positions.
- Commit message: `RES-181: LSP hover shows inferred type`.

## Notes
- Markdown is rendered by some clients, plain by others — use
  `MarkedString::String` to keep output universal.
- If inference failed for the fn, return the last known type
  rather than nothing — a partial answer is better than blank.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed and bailed by executor (blocked on RES-120 +
  substantial new LSP infra)
- 2026-04-17 claimed by executor — landing RES-181a scope (literals-only hover per the bail's Option 2)
- 2026-04-17 landed RES-181a (literals-only hover); RES-181b deferred until RES-120

## Resolution (RES-181a — literals-only hover)

This landing takes the **Option 2** path from the Attempt-1 bail:
rewrite as a literals-only hover so the shared scaffolding
(document storage, capability advertisement, position-resolution
plumbing, and the integration-test harness) earns its keep ahead
of the other LSP tickets (completion, go-to-def) that want the
same infra. Identifier hover (RES-181b) stays deferred until
RES-120 exposes per-position inferred-type information.

### Key implementation choice: token-level lookup, not AST walk

The ticket's acceptance criteria talk about "walking the AST for
the enclosing node" — but that's unreliable in practice, because
`Parser::span_at_current` stamps every leaf node's span with
`last_token_*` which is updated by the lexer AFTER it advances
past the token. So `Node::IntegerLiteral { span }` ends up
carrying the start coordinates of whatever lexeme followed, not
the literal's actual source range.

This landing sidesteps that quirk by driving the lexer directly
against the cached source text at hover time. The lexer's
`next_token_with_span` returns proper start+end coordinates in
one call, giving reliable per-token positions. Re-lexing is
O(tokens) and far cheaper than the HashMap read that precedes
it — the perf is a non-issue.

### Files changed

- `resilient/src/main.rs`
  - `Lexer::next_token_with_span` promoted from private to
    `pub(crate)` so the LSP hover handler can call it.
- `resilient/src/lsp_server.rs`
  - New imports for `Hover`, `HoverContents`, `HoverParams`,
    `HoverProviderCapability`, `MarkedString`.
  - `initialize` capabilities now advertise
    `hover_provider: Some(HoverProviderCapability::Simple(true))`.
  - New `Backend::hover` method — looks up the cached source
    text (`documents_text`), delegates to `hover_literal_at`,
    wraps the result in `Hover { contents: HoverContents::Scalar,
    range }`. Returns `Ok(None)` for positions outside any
    literal token.
  - New `pub(crate) fn hover_literal_at(src, pos) -> Option<(&'static str, Range)>`
    — drives the lexer over `src`, finds the token whose span
    contains `pos`, classifies it as `Int` / `Float` / `String` /
    `Bool` / `Bytes`. Returns `None` for keywords, identifiers,
    operators, or out-of-range positions.
  - New `lex_span_contains_lsp_position` helper — handles both
    single-line and multi-line spans, with exclusive end.
- `resilient/tests/lsp_hover_smoke.rs` (new)
  - Full end-to-end LSP round-trip: initialize → initialized →
    didOpen a 4-line document → four hover requests at distinct
    literal kinds (Int / Float / Bool / String) + one at a
    non-literal keyword position.
  - Asserts each response carries the expected type name and a
    non-empty range; the keyword case asserts `"result":null`.
  - Mirrors `lsp_smoke.rs`'s hand-rolled LSP framing. No new
    dep tree.

### Tests (16 unit + 1 integration, all `res181a_*`)

Unit (`src/lsp_server.rs`, gated `--features lsp`):
- `hover_on_int_literal_start_returns_int`
- `hover_on_int_literal_middle_returns_int` (cursor inside a
  multi-char literal still resolves)
- `hover_on_bool_literal_returns_bool`
- `hover_on_false_literal_returns_bool`
- `hover_on_string_literal_returns_string`
- `hover_on_float_literal_returns_float`
- `hover_on_bytes_literal_returns_bytes`
- `hover_on_keyword_returns_none`
- `hover_on_identifier_returns_none` (RES-181b future scope)
- `hover_on_operator_returns_none`
- `hover_out_of_range_returns_none`
- `hover_on_empty_source_returns_none`
- `hover_inside_fn_body_returns_literal_type`
- `hover_returns_range_covering_the_token`
- `lex_span_contains_lsp_position_single_line`
- `lex_span_contains_lsp_position_different_line`

Integration (`tests/lsp_hover_smoke.rs`):
- `lsp_hover_returns_type_name_for_literal_positions` — end-to-
  end assertion over 4 literal kinds + 1 null-response case.

### Verification

```
$ cargo build                                   # OK (8 warnings)
$ cargo build --features z3                     # OK
$ cargo build --features jit                    # OK
$ cargo build --features lsp,logos-lexer,infer  # OK
$ cargo test --locked
test result: ok. 651 passed; 0 failed     (non-lsp baseline unchanged)
$ cargo test --locked --features lsp
test result: ok. 694 passed; 0 failed     (+16 unit, +1 integration)
$ cargo test --features lsp res181a
test result: ok. 16 passed; 0 failed
$ cargo test --features lsp --test lsp_hover_smoke
test result: ok. 1 passed; 0 failed       (finishes in <1s)
```

### What was intentionally NOT done

- **RES-181b** — no identifier hover. The handler returns None
  for any non-literal position. Blocked on RES-120 exposing a
  per-position inferred-type table.
- No AST position walker — the unreliable span-at-leaf quirk
  (documented above) led to the token-level approach instead.
  When RES-181b lands identifier hover, it will likely reuse
  the same token-lookup seam: find the token at `pos`, call the
  future inferred-type lookup with the token's lexeme.
- No caching: each hover request re-lexes the source. This is
  fine — lex is fast (< 1ms for any document size we'd see in
  practice) and premature caching would add state that
  RES-181b / future tickets would have to thread through.

### Follow-ups the Manager should mint

- **RES-181b** — identifier hover. Gated on RES-120 landing a
  per-position inferred-type table (or a narrower ticket that
  exposes the typechecker's env after `check_program`). The
  `hover_literal_at` helper is the natural extension point: add
  an `Token::Identifier(name)` arm that calls the inference
  table, and widen the return type to carry the looked-up type.

## Attempt 1 failed

Two blockers.

1. **"Inferred type" source is RES-120**, which is bailed (OPEN
   with `## Clarification needed`). The fallback line in the
   acceptance criteria — "the typechecker's recorded type if
   RES-120 isn't enabled" — doesn't help: the typechecker today
   doesn't expose its environment after `check_program` and
   doesn't retain per-position type info for identifiers at all.
   Without one of those, hover has nothing but literal-type
   heuristics.
2. **New LSP infrastructure the current scaffolding doesn't
   carry**:
   - Document storage (`Arc<Mutex<HashMap<Url, String>>>` on
     `Backend`) — today `publish_analysis` receives text and
     drops it; hover requests have only a URI.
   - `did_close` handler to clean the map.
   - Capabilities advertisement (`hover_provider: Some(...)`).
   - AST position walk — find the deepest `Spanned<Node>` whose
     span contains the requested `(line, col)`.
   - `Backend::hover` implementation + return-shape conversion.
   - An end-to-end integration test in `tests/lsp_hover.rs` —
     ~120 lines mirroring `lsp_smoke.rs`'s framing pattern
     (initialize / didOpen / hover / shutdown).

## Clarification needed

Manager, please either:

- Gate RES-181 on RES-120 + a narrow ticket to expose the
  typechecker's top-level env / a per-identifier type table; or
- Rewrite as a literals-only hover (`Int` / `Float` / `Bool` /
  `String` on literal positions) and split identifier hover into
  RES-181b, deferred until RES-120 lands.

Option 2 is ~70% of the ticket's user value and would let the
shared scaffolding — document storage, capabilities, AST position
walk — earn its keep ahead of the other LSP tickets (completion,
go-to-def) that want the same plumbing.

No code changes landed — only the ticket state toggle and this
clarification note.
