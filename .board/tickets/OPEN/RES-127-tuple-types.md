---
id: RES-127
title: Tuple types `(Int, String)` with inference and destructuring let
state: OPEN
priority: P3
goalpost: G7
created: 2026-04-17
owner: executor
---

## Summary
Today "return two things" means packing a struct or an array. Tuples
are a lighter-weight alternative — especially for multi-value
returns — and HM inference handles them naturally as type
constructors of fixed arity.

## Acceptance criteria
- Parser: `(a, b)` in expression position is a tuple literal.
  `(Int, String)` in type position is a tuple type. Unit tuple `()`
  = `Type::Void` alias (same thing).
- Indexing: `t.0`, `t.1` for positional access (parser extension to
  dotted access so field-style works on tuples too).
- Destructuring let: `let (x, y) = foo();`.
- Interpreter + VM + JIT: tuples represented as a thin `Vec<Value>`
  for now; optimize layout in a follow-up.
- Unit tests covering literal construction, indexing, destructuring,
  and a type error on arity mismatch.
- Commit message: `RES-127: tuple types + destructuring let`.

## Notes
- One-element tuple syntax is `(x,)` with a trailing comma, per the
  Rust convention; `(x)` is just a parenthesized expression. Make
  sure the parser distinguishes.
- Don't introduce a `first`/`second` stdlib yet — `.0` / `.1`
  covers it.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed and bailed by executor (spans parser / AST /
  typechecker / 3 backends)

## Attempt 1 failed

Five independently-sized pieces bundled:

1. **Parser**: tuple literal `(a, b)` — disambiguated from
   parenthesized-expression `(x)` via the Rust singleton-comma
   convention `(x,)`; tuple TYPE `(Int, String)` in annotation
   position (today `type_annot` is a bare `String`, so this
   needs a structured parse or a richer grammar); `t.0` / `t.1`
   positional access (extend dotted access to accept integer
   literals after `.`).
2. **AST**: new `Node::TupleLiteral { items, span }`; a new
   pattern shape for destructuring let; a new type-annotation
   shape for `(T1, T2)`.
3. **Typechecker**: new `Type::Tuple(Vec<Type>)` variant plus
   elementwise unification. Every exhaustive `match Type` arm
   across the codebase grows a Tuple case.
4. **Three backends**: interpreter `Value::Tuple(Vec<Value>)`
   + eval is cheap; VM + JIT both need layout + opcode / IR
   work (parallel to RES-170 struct ops and RES-165 JIT
   struct-ops, both still OPEN).
5. **Tests**: literal + indexing + destructuring + arity-
   mismatch.

Plus the `(x)` vs `(x,)` disambiguation is the kind of parser
surgery that has ripple effects (fn param list, call
expression, assertion arg, match patterns all start with `(`).

## Clarification needed

Manager, please split:

- RES-127a: parser + AST for tuple literal + tuple type + `.N`
  positional access. Interpreter `Value::Tuple` + eval. Tree-
  walker smoke tests only — no VM / JIT / typechecker
  unification. Smallest self-contained slice.
- RES-127b: destructuring `let (x, y) = ...`. LetStatement's
  `name: String` grows into a `Pattern` — fans out to the
  evaluator + register-arg parser.
- RES-127c: typechecker `Type::Tuple` + arity-mismatch
  diagnostic + exhaustive-match arm updates.
- RES-127d: VM opcodes + JIT lowering for tuple literal /
  index / destructure. Share the pattern with RES-170 /
  RES-165 once those land.

No code changes landed — only the ticket state toggle and this
clarification note.
