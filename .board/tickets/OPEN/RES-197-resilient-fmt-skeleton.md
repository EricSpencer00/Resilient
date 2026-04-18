---
id: RES-197
title: `resilient fmt` subcommand: deterministic source formatter
state: OPEN
priority: P3
goalpost: tooling
created: 2026-04-17
owner: executor
---

## Summary
Every language past its infancy wants a canonical formatter.
Ship a minimal `resilient fmt` that parses, pretty-prints, and
writes back. The policies are few but fixed: 4-space indent,
braces on same line, trailing comma in multi-line arg lists,
single blank line between top-level items.

## Acceptance criteria
- New subcommand `resilient fmt [<file>...] [--check] [--stdin]`:
  - No args: formats every `*.rs` under the cwd (recursive).
  - `--check`: exit 1 if any file would change; print a unified
    diff. No file writes.
  - `--stdin`: read stdin, write formatted to stdout.
- Pretty-printer module `resilient/src/fmt.rs` walks the AST and
  emits canonical whitespace. Preserves comments (including block
  comments from RES-024) and attaches them to the next statement.
- Idempotent: `fmt` on `fmt(src)` == `fmt(src)`. Unit test asserts
  this on every `examples/` file.
- Commit message: `RES-197: resilient fmt subcommand`.

## Notes
- Don't invent policy as you go — write a one-page "style guide"
  doc alongside the formatter listing every rule and the rationale.
- Comment placement is the hardest part — lean on the spans
  (RES-077..088) to associate each comment with the nearest
  AST node at parse time.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 claimed and bailed by executor (oversized for one
  iteration — see Attempt 1)

## Attempt 1 failed

Bailing: the AC bundles several sub-projects that each deserve
their own iteration, and a rush-landed version would silently
damage user code.

### Concrete size of the work

1. **Pretty-printer covering every AST variant.** A `grep` of
   `Node::` in `src/main.rs` lines 917–1293 shows 39 variants
   (Program, Use, Function, LiveBlock, DurationLiteral, Assert,
   Block, LetStatement, StaticLet, Assignment, ReturnStatement,
   IfStatement, WhileStatement, ForInStatement,
   ExpressionStatement, Identifier, IntegerLiteral, FloatLiteral,
   StringLiteral, BytesLiteral, BooleanLiteral, PrefixExpression,
   InfixExpression, CallExpression, TryExpression,
   FunctionLiteral, Match, StructDecl, LetDestructureStruct,
   StructLiteral, FieldAccess, FieldAssignment, ArrayLiteral,
   IndexExpression, IndexAssignment, MapLiteral, SetLiteral,
   ImplBlock, TypeAlias). Each needs a formatting rule
   (indentation, list separators, operator-precedence parens).

2. **Comment preservation.** The current lexer DISCARDS
   comments — `//...\n` and `/* ... */` never produce tokens
   (verified: `lexer_logos.rs` marks block comments as `Skip`;
   the hand-rolled scanner jumps over `//` in `next_token`).
   Preserving them requires either:
   - Lexer mod to tokenize comments with spans, plus a parser
     that attaches them to adjacent AST nodes (invasive; risks
     breaking every downstream phase).
   - A separate pass over the original source text that
     reconstructs comment positions by line/column lookup at
     emit time. Less invasive but still ~150-200 lines and
     with its own edge cases (block comments inside strings,
     nested comments).

   All 14 `examples/*.rs` files contain comments (verified via
   `grep -l "^//\|/\*"`). Without comment preservation, the
   formatter would silently delete every `// RES-NNN:`
   annotation, every `//` module header, every block comment —
   breaking the ticket's own idempotence AC on every example
   file. With a naive drop-comments strategy, running `fmt`
   once would damage the corpus irrecoverably.

3. **`--check` + `--stdin` + recursive walk.** Straightforward
   CLI work (~80 lines), but the recursive-walk-cwd mode is a
   footgun: running `resilient fmt` at the repo root would try
   to format every Rust source file in `resilient/src/*.rs`
   (because they also end in `.rs`). A sensible default needs
   to detect that a file is actually Resilient source, which
   is a whole discussion (look for `Resilient.toml`? MIME
   sniffing? explicit allow-list?). Not addressed in the
   ticket.

4. **Idempotence test on every `examples/*.rs`.** Strong
   invariant — any precedence bug or edge-case whitespace
   mismatch immediately shows up. Would take multiple
   iterations to debug to green.

5. **Style-guide doc** — easy to write, but a load-bearing
   artifact that the formatter's output has to honour
   verbatim.

### Honest time estimate

~6-10 focused hours for a first-shot implementation, followed
by 2-3 iterations debugging idempotence failures on specific
examples. Squeezing all of that into one iteration would either
produce a broken formatter (which silently damages code) or
skip one of the AC bullets (comments, idempotence, or full
coverage).

### Clarification needed

Three resequence options for the Manager:

1. **Split into a staged rollout** (recommended):
   - RES-197a: `fmt.rs` pretty-printer scaffolding + CLI
     (`--check`, `--stdin`, explicit file args ONLY — no cwd
     walk). Covers a small shape set (Function, Block,
     LetStatement, ReturnStatement, InfixExpression, literals,
     Identifier, CallExpression). Style-guide doc. Comments
     are passed through by scanning the source text at emit
     time. Small, reviewable, testable.
   - RES-197b: broaden coverage (Match, LiveBlock, StructDecl,
     ImplBlock, TypeAlias, LetDestructureStruct, MapLiteral,
     SetLiteral, TryExpression, FunctionLiteral).
   - RES-197c: tighten the idempotence invariant — add the
     per-example harness from the original AC.
   - RES-197d: the cwd-walk mode, gated on a
     `Resilient.toml` presence so cross-project footguns are
     avoided.

2. **Rewrite as a pretty-printer-only ticket** with no CLI
   surface — `fn fmt_program(&Node) -> String` plus unit tests
   for key shapes. Follow-up tickets wire the CLI and
   comment preservation.

3. **Accept the "drop comments" deviation explicitly.** If the
   Manager is willing to lose comments in the formatter's
   output, a narrower rewrite could land in one iteration. But
   comments are user-authored metadata; losing them silently
   is a big UX regression from the current
   "just-run-the-program" status quo.

No code changes in this attempt — ticket-only move so `main`
stays clean.
