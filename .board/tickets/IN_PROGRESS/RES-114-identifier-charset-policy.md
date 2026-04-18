---
id: RES-114
title: Identifier character policy: ASCII-only, explicit spec + test
state: OPEN
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
