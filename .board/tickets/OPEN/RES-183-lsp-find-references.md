---
id: RES-183
title: LSP: find-references for top-level functions
state: OPEN
priority: P3
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
Counterpart to RES-182: given a cursor on a fn name, list every
location that calls it. Scope to top-level fns in the open file
(+ spliced imports). Local / param refs are less useful and out
of scope.

## Acceptance criteria
- `Backend::references` returns an array of `Location` covering
  every call site.
- Match is AST-driven, not textual — `Node::Call` with callee
  name equal to the target.
- `includeDeclaration: true` in the request adds the defining
  site; false omits it.
- Integration test with a 3-caller setup + a struct literal that
  uses the same name but is distinct (should not appear).
- Commit message: `RES-183: LSP find-references`.

## Notes
- Same name-resolution table from RES-182 is reused — no new
  pre-pass.
- Performance: linear scan of the AST is fine. Don't premature-
  optimize; typical files are small.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed and bailed by executor (blocked on the
  same LSP infra gap that bailed RES-181 / RES-182)

## Attempt 1 failed

The ticket's text is explicit: "Same name-resolution table from
RES-182 is reused — no new pre-pass." RES-182 is currently
OPEN with its own bail identifying four missing pieces of LSP
infrastructure:

1. **Document storage** on `Backend` — today
   `resilient/src/lsp_server.rs` has no
   `HashMap<Uri, (text, program)>` or equivalent;
   `publish_analysis` re-parses on every did_open /
   did_change but doesn't cache anything cursor-aware can
   consume. `Backend` has `client` only (confirmed by
   reading lines 27–31 of `lsp_server.rs`).
2. **Capability advertisement** — `ServerCapabilities` in
   `initialize` (line 142) sets only `text_document_sync`.
   `references_provider` / `hover_provider` /
   `definition_provider` are all unset, so a compliant
   client won't route the request here in the first place.
3. **Position → AST-node lookup** — given a cursor at
   line L col C, find the Node it refers to. Today's
   `Span` carries a line/col pair but no walker exists that
   maps "cursor in text" back to an identifier node.
4. **Spans don't carry source URIs** — with RES-073's
   import splicing, imported nodes' spans are in the
   imported file's coordinate system but unstamped with
   a source path. `Location` over a cross-file reference
   can't fill `Uri` correctly today.

RES-183 layered on top of RES-182's name-resolution table
would be a ~100-line ticket (AST scan for `CallExpression`
with matching callee name, plus the `includeDeclaration`
toggle). Without that foundation, delivering it means
reimplementing pieces 1-4 AND the resolver — the "reuses
RES-182's table" clause is a dead reference.

## Clarification needed

Manager, please gate on RES-182 landing (or its proposed
RES-XXX-a / b split — see RES-182's own clarification
section). Once RES-182 is in, RES-183 reduces to:

- Add `references_provider: Some(OneOf::Left(true))` to
  `ServerCapabilities`.
- Implement `Backend::references`: consume the cursor's
  target name via the name-resolution table (RES-182),
  AST-walk for `Node::CallExpression { function:
  Identifier { name, .. }, .. }` where name matches,
  collect spans into `Location`s, optionally include the
  fn decl's own span based on `includeDeclaration`.
- Integration test `tests/lsp_references.rs` with the
  ticket's three-caller + struct-literal-false-positive
  shape.

That's the iteration-sized slice the ticket intends.
Blocking on RES-182 keeps the LSP infra consistent — one
resolver, not two — and avoids a "RES-183 invented its own
scheme that conflicts with RES-182" merge mess when 182
eventually lands.

No code changes landed — only the ticket state toggle and
this clarification note. Committing the bail as a ticket-only
move so `main` stays unchanged except for the metadata.
