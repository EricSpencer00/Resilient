---
id: RES-128
title: `type Meters = Int;` type alias declarations
state: DONE
priority: P3
goalpost: G7
created: 2026-04-17
owner: executor
---

## Summary
Aliases aren't new types — `Meters` unifies with `Int` — but they
document intent in function signatures and array shapes. Tiny
feature, immediate readability payoff.

## Acceptance criteria
- Parser: `type <Name> = <Type>;` at top level. No generics
  yet (`type Pair<A,B> = (A, B)` is RES-129's follow-up).
- Resolution: aliases expand eagerly at lookup time. A cycle (alias
  referring to itself transitively) is a diagnostic, not a panic.
- Unit test: `type M = Int; fn foo(M x) -> M { return x + 1; }`
  typechecks; `let m: M = "hi";` is a type error.
- SYNTAX.md gets a "Type aliases" subsection.
- Commit message: `RES-128: type alias declarations`.

## Notes
- Aliases do NOT create a nominal type — that's RES-126's
  territory. Document this inline with a `// alias is NOT
  nominal; use struct for newtype` comment.
- No forward references across modules yet; the resolver runs
  post-import-splice anyway (RES-073), so within-file forward refs
  are fine.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files changed:
- `resilient/src/main.rs`
  - New `Token::Type` keyword + lexer arm `"type" => Token::Type`
    + `Token::Type` case in `display_syntax`.
  - New `Node::TypeAlias { name, target, span }` AST variant.
  - New `Parser::parse_type_alias` + dispatch from
    `parse_statement` on `Token::Type`. Accepts
    `type <Name> = <Target>;` (trailing `;` optional to match
    LetStatement's existing tolerance). Diagnostic shape for
    malformed forms: `Expected alias name after 'type'`,
    `Expected '=' after 'type NAME'`, `Expected target type
    name after 'type NAME ='`.
  - `Node::TypeAlias` is a runtime no-op (typechecker-only
    concern; `eval` returns `Value::Void`).
  - Four new unit tests:
    - `type_alias_accepts_structurally_compatible_value`
    - `type_alias_rejects_wrong_value_type` (asserts the
      diagnostic expands the alias: `let m: int — value has type
      string`).
    - `type_alias_cycle_is_diagnostic_not_panic` (`A -> B -> A`
      surfaces `type alias cycle: A -> B -> A` without stack-
      overflow).
    - `type_alias_forward_reference_works` (the hoisting pass
      registers aliases before per-stmt walks).
- `resilient/src/typechecker.rs`
  - New `type_aliases: HashMap<String, String>` on
    `TypeChecker`; populated in `check_program_with_source`'s
    pre-pass alongside the existing RES-061 contract-table
    registration, and also in `check_node`'s `Node::TypeAlias`
    arm for mid-walk declarations.
  - `parse_type_name` now routes through
    `parse_type_name_inner(name, &mut seen)` which expands
    aliases transitively. Cycle detection: if the walk re-enters
    an alias name already in `seen`, returns
    `"type alias cycle: <chain>"`.
- `resilient/src/lexer_logos.rs` — `#[token("type")] Type` arm
  + `Tok::Type => Token::Type` in convert. Preserves the
  feature-gated parity with the hand-rolled lexer.
- `resilient/src/compiler.rs` — `Node::TypeAlias` in the
  `node_line` structural-variants pattern so the AST→line
  exhaustive match stays exhaustive under VM compile.
- `SYNTAX.md` — new `### Type aliases` subsection showing the
  syntax, the structural-not-nominal rule (with a pointer to
  RES-126's struct form for nominal types), forward-reference
  support, and the cycle diagnostic.

Verification:
- `cargo build --locked` — clean.
- `cargo test --locked` — 292 unit (+4 new) + 3 dump-tokens +
  12 examples-smoke + 1 golden pass.
- `cargo test --locked --features logos-lexer` — 293 unit (incl.
  parity) pass.
- `cargo clippy --locked --features logos-lexer --tests -- -D warnings`
  — clean.
- Manual: `type M = int; fn inc(M x) -> M { return x + 1; }` +
  `inc(41)` typechecks and prints `42`. `let m: M = "hi";`
  (with `type M = int;`) reports `let m: int — value has type
  string`. `type A = B; type B = A; let a: A = 1;` reports
  `type alias cycle: A -> B -> A`.
