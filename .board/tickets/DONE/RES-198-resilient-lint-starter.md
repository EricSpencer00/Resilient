---
id: RES-198
title: `resilient lint` subcommand with 5 starter lints
state: DONE
priority: P3
goalpost: tooling
created: 2026-04-17
owner: executor
---

## Summary
A lint subcommand is the vehicle for enforcing style beyond
formatter scope and catching common classes of bug that aren't
type errors. Start small — five lints, each with a stable code
and an `#[allow(...)]`-style suppress syntax.

## Acceptance criteria
- Subcommand `resilient lint <file>` runs the parser + typechecker
  + linter and prints per-lint diagnostics.
- Initial lints (each with a stable code `L0001`..`L0005`):
  - L0001: unused local binding
  - L0002: unreachable arm after `_ =>`
  - L0003: comparison `x == x` always true (typo smell)
  - L0004: mixing `&&` and `||` without parens
  - L0005: `return` at end of function body (redundant)
- Suppress syntax: `// resilient: allow L0003` on the line above
  the offending node.
- Exit 0 if no diagnostics; 1 if any lint fires at warning
  severity; 2 if any at error severity (none of the starter five
  are error-severity).
- Unit tests per lint: one triggering case + one allow-suppressed
  case.
- Commit message: `RES-198: resilient lint with 5 starter lints`.

## Notes
- Sharing infrastructure with `Diagnostic` (RES-119) lets lints
  go through the same LSP publish path "for free" — exposed in a
  follow-up.
- Lints are warnings, not errors, unless the user escalates via
  `--deny L0001` (pattern borrowed from rustc). Implement that
  flag in this ticket.

## Resolution

### Files added
- `resilient/src/lint.rs` — new module.
  - `Lint { code, severity, message, line, column }` struct.
  - `Severity` enum (`Warning` / `Error`).
  - `KNOWN_CODES` slice — validated by the CLI's `--deny` /
    `--allow` arg parsing.
  - `check(program, source) -> Vec<Lint>` — top-level entry
    that runs all five lints, applies `// resilient: allow`
    suppression, and sorts output by `(line, column)`.
  - `format_lint(&Lint, path) -> String` — single-line
    `<path>:<line>:<col>: <severity>[<code>]: <message>`
    format (matches RES-080's prefix convention).
  - 24 unit tests.
- `resilient/tests/lint_smoke.rs` — 9 integration tests
  driving the real binary:
  - Clean program → exit 0.
  - Warning → exit 1 + stdout mentions `L0001` + `warning`.
  - `--deny LCODE` escalates to `error` + exit 2.
  - `--allow LCODE` suppresses.
  - Unknown code on `--deny` → exit 2 + usage error.
  - Missing file arg → exit 2.
  - Missing file → exit 2.
  - Output format includes `<path>:<line>:<col>:`.
  - Multi-code invocation: one escalated, others still
    warning, overall exit driven by error.

### Files changed
- `resilient/src/main.rs`
  - `mod lint;` declaration.
  - New `dispatch_lint_subcommand(&args)`: `resilient lint
    <file> [--deny LCODE]* [--allow LCODE]*`. Parses args,
    reads + parses the file (exits 2 on parse errors since
    lints can't safely read a broken AST), runs `lint::check`,
    applies `--allow` drops and `--deny` severity bumps,
    prints each hit, and exits with 0/1/2 based on the
    highest severity seen.
  - Wired into `main()` alongside `dispatch_pkg_subcommand`,
    `dispatch_verify_cert_subcommand`,
    `dispatch_verify_all_subcommand`.

### The five lints

- **L0001 — unused local binding.** Collects every `let` /
  `static let` in a function body, then checks whether each
  bound name appears in any `Identifier` read-site within the
  same body. Names starting with `_` are skipped (convention).
  Limitation: shadowing isn't tracked precisely (e.g.
  `let x = 1; let x = 2;` — the first `x` is flagged as used
  if either binding's RHS references `x`). MVP; shadow-precise
  analysis is a follow-up.
- **L0002 — unreachable arm after `_ =>`.** Walks every
  `Node::Match`; once a wildcard-only arm appears, flags the
  start of every subsequent arm. Report location is the arm
  body's span, so the `// resilient: allow L0002` comment
  goes on the line above the unreachable arm (not above the
  `match` keyword). Nested `Pattern::Or` wildcards don't
  trigger the flag — only a top-level wildcard arm does.
- **L0003 — self-comparison `x == x` / `x != x`.** Walks every
  `InfixExpression` with operator `==` or `!=`; fires when
  both sides are the same `Identifier` by name. Wording
  tailored per operator (`always true` / `always false`).
- **L0004 — mixing `&&` and `||` without parens.** Walks every
  `InfixExpression` with operator `&&` / `||`; fires when any
  immediate child has the opposite operator. Known limitation:
  paren-disambiguation isn't tracked in the AST, so
  explicitly-parenthesized `(a || b) && c` also fires. Users
  suppress with `allow L0004` at the call site; this is
  documented in the lint's message and in the module header.
- **L0005 — redundant trailing bare `return;`.** Looks at the
  last statement of each function body; if it's
  `ReturnStatement { value: None }`, fires. `return VALUE;`
  is NOT flagged — in Resilient today that's load-bearing
  because the language doesn't have implicit last-expression
  returns.

### Suppress syntax

`// resilient: allow L0001` on the line immediately above the
offending node suppresses that code for that line only.
Multiple codes per line: `// resilient: allow L0001, L0003`
(whitespace- or comma-separated). Implemented by scanning the
source text for the pattern and building a
`HashSet<(line, code)>` of suppressed locations; applied as a
post-filter on the raw lint list. Only codes matching the
`L\d{4}` shape are recognized — a stray
`// resilient: allow E0008` is ignored by the parser.

### Verification
- `cargo build` → clean
- `cargo test --locked` → 557 core (was 533; +24 new unit
  tests in `lint::tests`) + 9 new integration tests in
  `tests/lint_smoke.rs`; all green
- `cargo test --locked --features lsp` → 584 + 9
- `cargo clippy --locked --features lsp,z3,logos-lexer --tests
  -- -D warnings` → clean

### Follow-ups (not in this ticket)
- **Shared `Diagnostic` surface.** RES-119 (bailed) would let
  the LSP `publish_diagnostics` path emit lint hits "for free"
  per the ticket Notes. Today lints only surface via the
  `lint` subcommand.
- **Shadow-precise L0001.** Track per-statement use sites
  instead of the "name-in-body" set so `let x = 1; let x = 2;`
  flags the first binding correctly when only the second is
  used.
- **Paren-aware L0004.** Either add paren-tracking to the AST
  or scan the source text between the parent's operator span
  and the child's leftmost-leaf span to detect an explicit
  `(` before the child.
- **More lints.** Unused parameters, always-true `if`, dead
  `else` after `return`/`break`, etc.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 resolved by executor (5 starter lints, allow-
  comment suppression, --deny/--allow CLI flags, 24 unit + 9
  integration tests)
