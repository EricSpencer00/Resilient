---
id: RES-190
title: LSP: code action "insert `;`" for missing-semicolon diagnostics
state: OPEN
priority: P3
goalpost: G17
created: 2026-04-17
owner: executor
---

## Summary
First tangible code action — give users the "quick fix" lightbulb
experience for the single most common parser error. Lays the
pattern that other actions (delete unreachable arm, add `_`, etc.)
can follow.

## Acceptance criteria
- `Backend::code_action` returns a `CodeAction` when the diagnostic
  at the requested range has code "E-missing-semicolon"
  (introduced in RES-206's registry).
- The action's `WorkspaceEdit` inserts `;` at the end of the
  preceding token.
- Integration test opens a document with a missing `;`, requests
  code actions at the diagnostic range, asserts the action is
  present and its edit lines up with a fixed version.
- Commit message: `RES-190: LSP code action: insert missing semicolon`.

## Notes
- Depends on RES-119's Diagnostic carrying a stable code. If
  RES-206 hasn't registered "E-missing-semicolon" yet, wire a
  placeholder code and backfill when RES-206 lands.
- Don't auto-apply — editor UX presents the action and the user
  confirms.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 claimed and bailed by executor (no upstream diagnostic
  to attach to — see Attempt 1)

## Attempt 1 failed

Bailing: the code action in the AC is contingent on an upstream
diagnostic that doesn't exist, and adding it goes beyond the
ticket's stated scope.

### The AC chain

Line 18–20 says:

> `Backend::code_action` returns a `CodeAction` when the diagnostic
> at the requested range has code `"E-missing-semicolon"`
> (introduced in RES-206's registry).

For the integration test on line 23–25 to pass, the driver has to
publish a diagnostic with that code when the user writes a
missing-semi program. Today, **no such diagnostic exists** — the
parser is deliberately lenient:

- `parse_let_statement` (main.rs:2078–2080): `if peek == Semicolon
  { next(); }` — advances past `;` if present; does NOTHING if
  absent. No error emitted.
- `parse_return_statement` (main.rs:2285–2287): same shape.
- `parse_type_alias` (main.rs:1775–1779): `;` is "optional".

I reproduced the user-visible behaviour:

```
$ cat /tmp/f.rs
fn main(int _d) {
    let x = 1
    let y = 2
    return x + y;
}
main(0);
$ resilient /tmp/f.rs
Program executed successfully
```

No parser error, no type error — the parser silently accepts the
two missing semicolons. There is no `Diagnostic` with any code,
let alone `E-missing-semicolon`, for the code action to match on.

### What would be required

To land this ticket as written, one would need to:

1. **Add missing-semi detection to the parser.** Either make `;`
   mandatory (breaking change; every existing `examples/*.rs`
   would need `--fix`) or emit a *warning*-severity diagnostic
   (non-breaking). Either way: new work in `parser`-land with
   its own scope + migration story. Not a one-iteration item.

2. **Ship a diagnostic-code field.** RES-119's `Diagnostic`
   carried a `code: Option<DiagCode>`; RES-119 is bailed.
   Current `Diagnostic`s constructed in `lsp_server::publish_analysis`
   leave `code: None` via `..Default::default()`. Need to thread
   a code through from parser error site → published diagnostic.
   Blocked on RES-119.

3. **Register `"E-missing-semicolon"` in the code registry.**
   RES-206's registry — bailed.

4. **Then** implement the `code_action` handler + `WorkspaceEdit`
   insert — the parts this ticket is actually about. Without
   steps 1-3, there's nothing to trigger it on.

### Clarification needed

The Manager should either:

- **Split the prerequisite off first.** A new RES-XXX-parser-
  missing-semi-diagnostic that (a) picks strict-vs-warning,
  (b) lands the emission. Plus RES-119a (the Diagnostic scaffolding
  called out in RES-119's own clarification) so the diagnostic can
  carry a code. Re-open RES-190 once those land.

- **OR rewrite RES-190 as a "plumbing-only" ticket.** Land the
  `code_action` handler + workspace-edit builder + capability, but
  DROP the integration-test AC (no diagnostic yet to trigger on).
  Unit-test the handler in isolation with a hand-constructed
  `CodeActionParams` whose `context.diagnostics` carries a
  placeholder-code diagnostic. Follow-up ticket wires real
  diagnostic emission.

- **OR rewrite to heuristic detection.** Scan the source between
  two adjacent top-level statements' spans for a missing `;`;
  synthesize the diagnostic from the LSP side without parser
  changes. Hacky (spans don't tell us "statement ended here"
  unambiguously; would need a lexer pass on the original text)
  but self-contained.

The ticket as written presupposes infrastructure that hasn't
landed. No code changes in this attempt — only the ticket state
toggle + this note. Committing the bail as a ticket-only move so
`main` is unchanged.
