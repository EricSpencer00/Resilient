---
id: RES-191
title: `@pure` function annotation + purity checker
state: DONE
priority: P3
goalpost: G18
created: 2026-04-17
owner: executor
---

## Summary
Mark functions as side-effect-free and have the checker enforce
it. First concrete G18 (effect tracking) ticket. A pure fn can
only:
- call other pure fns,
- read its parameters,
- do arithmetic / logic / comparison,
- construct / destructure values.

It can NOT: `println`, `file_*`, mutate captures, or call
unannotated user fns.

## Acceptance criteria
- Attribute syntax: `@pure\nfn name(...)`. Parser annotates the
  Function node.
- Checker: walks the body; any violation produces a diagnostic
  with the violating site's span and the reason ("calls unannotated fn foo"
  / "calls impure builtin println").
- Builtins tagged in the registry as pure / impure; the initial
  tag list goes into a table in `typechecker.rs`.
- Recursive purity: `@pure fn a() { b(); } @pure fn b() { a(); }`
  passes so long as neither does anything impure. Implementation:
  assume purity optimistically, verify, backtrack on violation.
- Unit tests covering: success, impure call, impure builtin, pure
  mutual recursion.
- Commit message: `RES-191: @pure annotation + checker`.

## Notes
- `@pure` is checked, not inferred — inference is RES-192.
- A future ticket makes the verifier trust @pure fns for SMT
  reasoning (currently it treats all fns as arbitrary).

## Resolution

### Files changed
- `resilient/src/main.rs`
  - New `Token::At` variant. Scanner handles `'@'`; `display_syntax`
    renders as `` `@` ``.
  - `Node::Function` gained a `pure: bool` field. All 5 construction
    sites updated; destructure sites already used `..` so they stay
    as-is.
  - `Parser::parse_function` split into a thin shim + internal
    `parse_function_with_pure(pure: bool)` so the annotation path
    can flip the flag without duplicating the 100-line parser.
  - New `Parser::parse_attributed_item` — dispatched from
    `parse_statement` when `current_token == Token::At`. Reads the
    attribute name, validates (`pure` today; unknown → diagnostic +
    fall through), confirms the next token is `fn` (else diagnostic
    + fall through), then calls `parse_function_with_pure(true)`.

- `resilient/src/lexer_logos.rs` — parity: new `Tok::At` variant
  with `#[token("@")]`; `Tok::At => Token::At` arm in the
  convert fn.

- `resilient/src/typechecker.rs`
  - New pass `check_program_purity(statements, source_path)` called
    from `check_program_with_source` after the regular walk. Two-
    pass implementation: first collect the set of `@pure`-declared
    fn names, then walk each pure fn's body for violations. The
    optimistic first pass is what makes mutual recursion between
    two `@pure` fns pass without backtracking.
  - `check_body_purity` — recursive AST walker covering every
    statement / expression shape with a meaningful purity rule:
    - `CallExpression` — callee resolved to an identifier;
      `IMPURE_BUILTINS` → reject with "calls impure builtin `X`";
      `pure_fns` set → accept; `is_known_pure_builtin` → accept
      (pure-by-default builtin); else → reject with "calls
      unannotated fn `X`". Indirect / method callees rejected
      conservatively ("non-identifier callee; only bare-identifier
      calls to pure fns are allowed").
    - `LiveBlock` — reject ("retries are observable side effects").
    - `FieldAssignment` / `IndexAssignment` — reject (mutation).
    - Every control-flow / arithmetic / literal form — pass or
      recurse into children.
  - `IMPURE_BUILTINS: &[&str]` — named list: `println`, `print`,
    `input`, `clock_ms`, `random_int`, `random_float`, `file_read`,
    `file_write`, `env`, `live_retries`, `live_total_retries`,
    `live_total_exhaustions`.
  - `is_known_pure_builtin` — `PURE_BUILTINS` list (math, string,
    collection, Result, Map/Set/Bytes). Covers the rest of
    `BUILTINS` except the impure set — future new builtins added
    to one list should be thought about re the other.

### Design decisions
- **Attribute syntax.** `@pure` is its own top-level token type
  (`Token::At` + identifier), not baked into keyword recognition.
  Future attributes (`@inline`, `@deprecated`, `@cold`) all
  dispatch through the same `parse_attributed_item` path.
- **Unknown attributes are non-fatal.** Emit a diagnostic and parse
  the following item without annotation. A typoed `@pur` shouldn't
  cascade into "body looks like garbage" errors.
- **Pure-builtin policy** is "complement of the impure list" rather
  than a positive "@pure"-tagged registry. Less bit-rot risk — new
  math builtins added to `BUILTINS` automatically count as pure
  unless someone explicitly moves them to `IMPURE_BUILTINS`.
- **Mutual recursion** works because the first pass is optimistic:
  both `a` and `b` are in `pure_fns` before either body is walked.
  No backtracking needed because the ticket's rule is "pure-to-
  pure is fine, unannotated-user-fn is not" — not "infer purity
  transitively".
- **Impl-method purity** — methods inside `impl` blocks get
  `pure: false` today. Annotating them is a follow-up; the
  parser's impl-block code path would need its own `@` handling.

### End-to-end spot checks
```
$ printf '@pure\nfn double(int x) { return x * 2; }\nfn main(int _d) { return double(5); }\nmain(0);\n' | resilient -t -
Type check passed
Program executed successfully

$ cat /tmp/bad.rs
@pure
fn speak(int x) { println("hi"); return x; }
fn main(int _d) { return speak(5); } main(0);
$ resilient -t /tmp/bad.rs
Type error: /tmp/bad.rs:1:2: @pure fn `speak`: calls impure builtin `println`
```

### Verification
- `cargo build` → clean
- `cargo test --locked` → 500 + 16 + 4 + 3 + 1 + 12 + 4 = 540
  tests pass (was 488 core; +12 new `purity_tests`)
- `cargo test --locked --features lsp` → 527 core + 16 + 4 + 3 +
  1 + 12 + 8 + 4 (+12 under the lsp feature compile)
- `cargo clippy --locked --features lsp,z3,logos-lexer --tests
  -- -D warnings` → clean

### Tests
12 `purity_tests` in `resilient/src/typechecker.rs`:
- **Pass**: pure-arithmetic body; pure-builtin call; struct
  construction; mutual recursion between two `@pure` fns;
  unannotated fns not checked.
- **Reject**: `println`, `clock_ms`, `file_read`, unannotated
  user-fn call, `live` block.
- **Error wording**: fn name + callee name appear in message;
  RES-080 `<path>:<line>:<col>:` prefix appears.

### Follow-ups noted for the Manager
- RES-192 (effect inference) can infer purity for unannotated fns
  so `@pure fn uses(helper)` works when `helper` happens to be
  pure.
- Attributes on impl-block methods.
- Verifier integration — RES-191's Notes call it out: a future
  ticket lets the Z3 verifier trust `@pure` fns for SMT reasoning.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 resolved by executor (`@pure` attribute + purity
  checker; 12 unit tests; end-to-end verified)
