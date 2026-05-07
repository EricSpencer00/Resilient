# Major Language Features Status

A roadmap of what's implemented, partially implemented, and genuinely missing in Resilient. This was originally a "10 missing features" list; the audit found most are already in the compiler. The remaining genuinely missing pieces are listed below.

## Already Implemented

| Feature | Ticket | Status |
|--------|--------|--------|
| `?` operator (Result/Option propagation) | RES-086, RES-375 | Full — in `Node::TryExpression` eval |
| String interpolation `"x = {x}"` | RES-221 | Full — `string_interp.rs` module |
| Anonymous fn literals `fn(int x) -> int { ... }` | RES-403 | Full — `Node::FunctionLiteral` |
| Function-type annotations `fn(int) -> int` | RES-403 | Full — parser accepts in param position |
| Higher-order functions (`map`, `filter`, `reduce`) | RES-927 | Full — method-call form on arrays |
| Generic type parameters `fn id<T>(x: T) -> T` | RES-405 PR1-4 | Full with monomorphization |
| Variadic FFI (`extern fn printf(string, ...)`) | RES-316 | Full for FFI bindings |
| Pipe operator `\|>` | RES-926 | Full |
| `as` cast operator | RES-934 | Full |
| Closure captures (immutable) | RES-164 | Full |
| Tuple structs and tuple destructuring | RES-928, RES-933 | Full |
| `if let` / `while let` / `else if let` | RES-908, RES-914, RES-930 | Full |
| Range patterns / array slicing | RES-915, RES-911 | Full |
| Compound assignment `+=` etc. | RES-912, RES-917 | Full |
| Operator overloading via traits | (this PR) | Full at runtime |

## Partially Implemented

| Feature | Ticket | Status |
|--------|--------|--------|
| Sum type / enum payloads | RES-400 PR1 | Parser scaffold for payload-less variants only; PR2-5 (payloads, matching, exhaustiveness, eval) remain |
| Mutable closure capture | RES-328 | In progress — cell-based shared mutation works; auto-capture sugar deferred |
| Module system | RES-116 | Textual splicing + namespacing primitives; full `mod`/`use` graph deferred |
| Default function parameters | RES-118 | MVP for top-level fns; not for anonymous fn literals |

## Genuinely Missing

These have no in-flight scaffold and would each be substantial multi-PR work:

### 1. Macros (Compile-Time Code Generation)

No metaprogramming facilities exist. `assert_eq!`, `println!`-style format-string compile-time checking, and DSL macros all require an AST-at-compile-time representation, an expansion phase, and hygiene rules.

**Approximate scope**: 4-6 PRs (lexer for macro_rules!, parser for invocations, expansion engine, hygiene/scoping, error reporting, docs).

### 2. Async / Await

Embedded I/O and actor-style cooperative scheduling currently use the live-block / actor primitives, but there's no `async fn` or `await` syntax. Adding it would unlock more conventional non-blocking I/O patterns.

**Approximate scope**: 5-8 PRs (parser, type representation, state-machine transform, scheduler integration, examples).

### 3. Const Functions / Compile-Time Evaluation

Currently constants are limited to literals and a few folded expressions. A `const fn` keyword that lets the typechecker fully evaluate user code at compile time would enable better array sizes, lookup tables, and compile-time invariants.

**Approximate scope**: 3-4 PRs (parser, evaluator separation, typechecker integration, docs).

### 4. Trait Default Methods

Trait declarations can only contain method *signatures*. Adding a default body (so impls can omit methods that have a default) is a real ergonomic gap for trait-heavy code.

**Approximate scope**: 1-2 PRs (parser change + trait registry update).

### 5. Pattern-Matching Exhaustiveness for Structs

Match patterns over enums get exhaustiveness checks; match over plain structs and primitives don't. Closing this gap surfaces a class of bugs that typechecking misses today.

**Approximate scope**: 2-3 PRs (extend the existing exhaustiveness pass, golden-file diagnostics).

### 6. Type Classes / Implicit Parameters

For ergonomic generic numeric code (`fn sum<T: Num>(arr: Array<T>) -> T`), some kind of bounded-instance lookup mechanism beyond the current trait-bound substitution would help. Today users must wrap operations in trait method calls.

**Approximate scope**: 4-5 PRs (design + parser + typechecker + monomorphizer + docs).

### 7. Recursive Type Definitions

Self-referential structs like `struct Tree { Tree left, Tree right, }` would need a Box / indirection abstraction. Currently the type system treats this as an unbounded size error.

**Approximate scope**: 2-3 PRs (Box type, parser sugar, typechecker, runtime indirection).

### 8. Destructuring in Function Parameters

`fn rotate((int x, int y)) -> (int, int)` is a small ergonomic win that would parallel the existing `let` destructuring.

**Approximate scope**: 1 PR (parser change, typechecker, eval).

### 9. Custom Derive Attributes

`#[derive(Debug, Eq)]` on structs would auto-generate trait impls. Manual `impl` for boilerplate traits is currently the only option.

**Approximate scope**: 2-3 PRs (attribute parser already exists; generator + per-derive logic).

### 10. Associated Constants on Traits

`trait Bounded { const MIN: int; const MAX: int; }` — useful for numeric trait bounds and embedded constants.

**Approximate scope**: 2-3 PRs (parser, type representation, monomorphization).

---

## Footprint Reality Check

Resilient at ~1,800 KB of `lib.rs` is a much more complete language than the original "10 missing features" pass suggested. The genuinely-missing list above is what's left after auditing the actual implementation. Each item is decomposable into a small chain of PRs per the project's "ship-to-merge" workflow.
