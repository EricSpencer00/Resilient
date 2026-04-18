---
id: RES-184
title: LSP: rename symbol (prepareRename + rename)
state: OPEN
priority: P3
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
Refactor basics: given a symbol under the cursor and a new name,
emit a workspace edit that renames every reference. Build on the
resolution table from RES-182/183.

## Acceptance criteria
- `Backend::prepareRename` returns the range of the symbol under
  the cursor if it's renamable (local, param, top-level fn).
  Returns null otherwise.
- `Backend::rename` returns a `WorkspaceEdit` grouping per-file
  `TextEdit` lists.
- New-name validation: must match the identifier pattern
  (`[A-Za-z_][A-Za-z0-9_]*`); else return an LSP error.
- Collision detection: if the new name shadows a still-visible
  binding, return an LSP error `rename would shadow <name>` rather
  than produce broken code.
- Integration test renaming a top-level fn + its callers, asserts
  every edit is present.
- Commit message: `RES-184: LSP rename symbol`.

## Notes
- Don't rename struct fields yet — separate ticket since it
  touches struct literal shorthand (RES-154) semantics.
- `prepareRename` is the UX guard — users get the "cannot rename
  here" feedback before they type.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed and bailed by executor (blocked on
  RES-182 + RES-183 which are both open / bailed)

## Attempt 1 failed

The ticket's Summary is explicit: "Build on the resolution
table from RES-182/183." Both prerequisites are currently
open with their own bails on missing LSP infrastructure:

- **RES-182** (go-to-definition) bailed citing four gaps:
  no `Backend` document storage, no capabilities advertised
  (`hover_provider` / `definition_provider` / and now
  `rename_provider`), no cursor → AST-node walker, and spans
  that don't carry source URIs.
- **RES-183** (find-references) bailed citing the same four
  gaps — it literally said "reuse the name-resolution table
  from RES-182; no new pre-pass."

Rename sits on top of BOTH. It additionally needs:

1. **prepareRename** — given a cursor, determine whether the
   symbol there is renamable and return its text range. This
   needs the cursor → AST walker (gap #3 above).
2. **rename** — rewrite every binding + usage site. This
   needs RES-183's references scan PLUS the declaration-
   site lookup from RES-182.
3. **WorkspaceEdit grouping** — group `TextEdit`s per
   source URI. This needs RES-182's span-source-URI work
   (gap #4) so imported-file references are routed to the
   right file.
4. **Collision detection** — scope-aware check that the new
   name doesn't shadow a still-visible binding. This needs
   the scope-aware resolver RES-182 described as oversized.
5. **Identifier pattern regex** — validate new name matches
   `[A-Za-z_][A-Za-z0-9_]*`. Trivial; not a blocker.
6. **Integration test** in `tests/lsp_rename.rs` exercising
   a top-level fn + callers.

Item 5 is an iteration-sized slice on its own. Items 1-4
reimplement the entire LSP infra RES-182 / RES-183 already
flagged. Delivering RES-184 before those land means either
(a) duplicating the work both predecessor tickets deferred
for a shared-infra ticket, or (b) building a narrower
rename-only resolver that will collide with RES-182's
scheme when it eventually lands.

## Clarification needed

Manager, please gate on RES-182 + RES-183 landing (or the
proposed RES-XXX-a / RES-XXX-b shared-infra split from
RES-182's clarification). Once the resolver + document
storage + span-URI work are in, RES-184 reduces to:

- Add `rename_provider: Some(OneOf::Right(RenameOptions {
  prepare_provider: Some(true), work_done_progress_options:
  Default::default() }))` to `ServerCapabilities`.
- `Backend::prepare_rename`: find the identifier at the
  cursor, return its span's `Range` (or `None` if not a
  renamable binding).
- `Backend::rename`: invoke RES-183's references query plus
  the declaration site, group the resulting Locations by
  URI into `WorkspaceEdit.changes`.
- Collision check: before emitting edits, query the
  scope-aware resolver at each target site; if any site
  has a visible binding with the new name, return the
  LSP error.
- Identifier regex validation up-front.
- Integration test as specified.

That's the iteration-sized slice the ticket intends.
Landing rename before references + goto is ordering the
dependency chain backwards; the ticket text acknowledges
this explicitly by referencing RES-182/183 as prerequisites.

No code changes landed — only the ticket state toggle and
this clarification note. Committing the bail as a
ticket-only move so `main` stays unchanged except for the
metadata.
