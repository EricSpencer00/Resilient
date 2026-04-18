---
id: RES-114
title: Identifier character policy: ASCII-only, explicit spec + test
state: DONE
priority: P3
goalpost: G5
created: 2026-04-17
owner: executor
---

## Summary
The current lexer silently accepts whatever `is_alphanumeric()`
returns true for, which in Rust includes the full Unicode XID
class. For a safety-critical language we don't want `kafa`
(Cyrillic) to collide visually with `kafa` (Latin). Lock the policy
to ASCII identifiers only, document it, and test a handful of
homoglyph attacks to prove we reject them.

## Acceptance criteria
- Identifiers match `[A-Za-z_][A-Za-z0-9_]*` exactly. String
  literals retain full UTF-8 (unchanged).
- Non-ASCII in identifier position produces a clean parser error
  with span: `identifier contains non-ASCII character 'ф' at line:col`.
- `SYNTAX.md` gains a section "Lexical: identifiers" stating the
  rule and the rationale (safety-critical homoglyph avoidance).
- Unit tests: `lexer_rejects_cyrillic_identifier`,
  `lexer_rejects_mixed_latin_greek`, `lexer_accepts_underscored_names`.
- Commit message: `RES-114: ASCII-only identifier policy + tests`.

## Notes
- String / comment bodies keep UTF-8; only identifier scanning
  tightens.
- Future waiver path: if a real user asks for non-ASCII, revisit
  under a new ticket with an explicit opt-in flag. Don't build
  the opt-in now (YAGNI).

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files changed:
- `resilient/src/main.rs`
  - `Lexer::is_letter` tightened from `ch.is_alphabetic()` to
    `ch.is_ascii_alphabetic()`, so only `[A-Za-z_]` starts / is
    accepted inside an identifier. Non-ASCII at an identifier
    position now falls through to `Token::Unknown(ch)`.
  - `parse_statement`'s `Token::Unknown(ch)` arm gains a non-ASCII
    branch that emits the dedicated diagnostic:
    `identifier contains non-ASCII character '<ch>' — Resilient
    identifiers are ASCII-only (see SYNTAX.md)` when `ch` is
    alphabetic in the Unicode sense.
  - Four new unit tests: Cyrillic `кафа` rejected,
    `Αlpha` (Greek start) at statement head produces the
    non-ASCII diagnostic, underscored ASCII identifiers still
    accepted (regression sanity check), and UTF-8 inside string
    literals still parses (confirms the policy narrowed *only*
    identifier scanning).
- `SYNTAX.md` — new `## Lexical: identifiers` section stating
  the ASCII-only rule, showing the error shape, and recording
  the rationale (homoglyph avoidance for safety-critical use).

Notes:

- The logos lexer's identifier regex was already ASCII-only
  (`[a-zA-Z][a-zA-Z0-9_]*|_[a-zA-Z0-9_]+`), so it was already
  aligned with the new policy. The parity test continues to
  enforce that the two lexers produce identical token streams.
- No opt-in flag for non-ASCII identifiers (YAGNI per the
  ticket's note) — a future ticket can add one if there's a
  real user ask.

Verification:
- `cargo build --locked` — clean.
- `cargo test --locked` — 279 unit (+4 new) + all integration
  pass.
- `cargo test --locked --features logos-lexer` — 280 unit (incl.
  parity) pass.
- `cargo clippy --locked --features logos-lexer --tests -- -D warnings`
  — clean.
- Manual: `let кафа = 1;` prints four `identifier contains
  non-ASCII character '<ch>'` parser diagnostics (one per
  Cyrillic char) and exits non-zero.
