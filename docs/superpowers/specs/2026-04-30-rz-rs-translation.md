# `.rz` ↔ `.rs` translation — feasibility assessment

**Date:** 2026-04-30
**Status:** Design assessment / feasibility note (no ticket yet)
**Tracking:** companion to the `.res` → `.rz` extension migration

---

## TL;DR

- **`.rz` → `.rs` (Resilient → Rust)**: feasible as a *one-way semantic
  port* for a defined subset of Resilient features. Roughly 60–70% of
  the language can be machine-translated; the remaining 30%+ uses
  Resilient-specific constructs (`live { }`, `requires` / `ensures`
  contracts, `recovers_to`, `actor`/`receive`, the runtime's
  cooperative scheduler) that have no direct Rust equivalent and
  would either be (a) lowered to runtime calls into a hypothetical
  `resilient_rt` crate, (b) elided with a generated comment, or
  (c) left as a translation error pointing at the offending construct.
  This is a useful capability — "give me a Rust translation of this
  Resilient program" is exactly what users ask for when prototyping.
- **`.rs` → `.rz` (Rust → Resilient)**: feasible only for a *very small
  subset* of Rust, and the feature is much less useful in practice.
  Resilient is missing whole subsystems Rust has (lifetimes, traits
  with associated types, async, macros, modules-as-files at the
  Rust scale, the `unsafe` story beyond MMIO). A round-trip
  Rust → Resilient translator would either reject most real Rust
  files or produce Resilient code that doesn't typecheck. Not worth
  building until the language reaches a much later goalpost.

The recommendation: **treat `.rz` → `.rs` as a real follow-up
project** (a separate command, e.g. `rz emit-rust prog.rz`), and
**defer `.rs` → `.rz` indefinitely**. The two directions have very
different cost/value profiles and they shouldn't be bundled.

---

## Why `.rz` → `.rs` is feasible

Resilient is a strict subset of "memory-safe imperative language
with contracts" — the surface that `.rs` already covers. The
mapping for the core surface is mostly mechanical:

| Resilient | Rust |
|---|---|
| `fn name(int x, string s) -> int { ... }` | `fn name(x: i64, s: &str) -> i64 { ... }` |
| `let x = 1;` | `let x: i64 = 1;` |
| `if c { a; } else { b; }` | `if c { a; } else { b; }` |
| `while c { body; }` | `while c { body; }` |
| `for x in arr { body; }` | `for x in &arr { body; }` |
| `[1, 2, 3]` | `vec![1, 2, 3]` |
| `Array<int>` | `Vec<i64>` |
| `Option<int>` | `Option<i64>` |
| `Result<int, string>` | `Result<i64, String>` |
| `struct Foo { int x; }` | `struct Foo { x: i64 }` |
| `enum Color { Red, Green }` | `enum Color { Red, Green }` |
| Arithmetic / comparison ops | identical |
| `fn(int) -> int` | `fn(i64) -> i64` |

The translator is a tree-walker that consumes the existing
Resilient AST (already unified, already tested) and emits
syntactically-valid Rust. Reusing the existing parser means we
get the migration "for free" the moment the translator is
written.

### What needs care, but is tractable

- **String types**: Resilient `string` is owned-by-default;
  emitting `&str` vs `String` requires a small ownership analysis.
  Conservative default: emit `String` for parameters that are
  later mutated, `&str` everywhere else.
- **Integer width**: Resilient `int` is `i64`; emit as `i64`.
- **Pattern matching**: Resilient `match` extends naturally to
  Rust's; payload-carrying enum patterns (RES-400 PR 4+) need
  matching destructuring.
- **`live { }` blocks**: lower to a function call into
  `resilient_rt::live(|| { body }, ...)` — the runtime crate
  ships the loop. Requires the user to depend on `resilient_rt`
  in their generated `Cargo.toml`.

### What's a translation error (with a clear pointer)

- **`requires` / `ensures` clauses**: no direct Rust equivalent.
  Options: (a) emit as a `debug_assert!` at the function entry/
  exit; (b) emit as a `// requires: ...` doc comment; (c) reject
  with a "this contract has no Rust translation — try
  `--ensures-as-asserts`" diagnostic. Pick (a) as the default
  with a flag to switch.
- **Z3-discharged contracts**: orthogonal — the Rust translation
  has the same runtime check the Resilient runtime has.
- **`recovers_to`**: depends on the runtime; lower to a call
  into `resilient_rt::recovers_to(...)`.
- **`actor` / `receive`**: depends on the runtime; lower to
  the equivalent calls. The actor's atomicity guarantee
  (RES-ACTOR-SEMANTICS Q2) requires an explicit lock around
  the body — translator emits the lock acquire/release.
- **Effect annotations** (`pure`, `io`, `-e->`): Rust has
  no first-class effect system. Translate to doc comments;
  Rust's borrow checker covers the most important effect
  (mutability) on its own.

### Where the user value lives

- **Onboarding**: a Rust developer can read the translated `.rs`
  output to understand what Resilient programs look like in
  familiar syntax.
- **Debugging**: when a Resilient program misbehaves, the Rust
  translation can be debugged with `rust-gdb` and `cargo`'s
  tooling.
- **Ecosystem reach**: the translation lets users vendor a
  Resilient module into a Rust project as a build-time codegen
  step (`build.rs` calls `rz emit-rust src/foo.rz`).

### Implementation cost

A single-pass tree-walker with proper Rust output is roughly
~3000 lines of Rust by analogy to Resilient's existing
formatter pass. The bulk of the work is the runtime-call
rewrites for `live { }` / `recovers_to` / `actor`, the
ownership analysis for `&str` vs `String`, and the test suite.
A motivated single contributor could ship a useful subset in
2–3 weeks; getting to "rejects nothing in
`resilient/examples/`" is a 2–3-month follow-on.

---

## Why `.rs` → `.rz` is much harder

Resilient is a *strict subset* of Rust's expressiveness today.
The reverse translation runs into walls everywhere:

| Rust feature | Resilient equivalent |
|---|---|
| Lifetimes (`<'a>`, `<'static>`) | none — Resilient uses regions but the model is different |
| Traits with associated types / GATs | RES-290 trait system has no associated types yet |
| Async / `Future` / `await` | none |
| Macros (`macro_rules!`, proc-macros) | none — Resilient's strong stance is "no macros" |
| `unsafe` raw pointers, `Pin`, `MaybeUninit` | only volatile MMIO is in scope |
| `dyn Trait` / vtables | none |
| Complex closures with capture | RES-164 series — closure capture not yet on the JIT path |
| `mod` / file modules at Rust scale | RES-324 has `mod` blocks but no file-tree resolution |
| `use` semantics with renames / globs | only basic `use` |
| Cargo features / conditional compilation (`#[cfg(...)]`) | partial — RES-343 covers basic shape |

Even a hypothetical translator that picks the "trivial" subset
would reject most real Rust files. The user would have to know
the subset rules upfront — which makes the tool a niche
educational toy rather than a general-purpose translator.

The exception: a *targeted* subset like "translate Rust function
signatures + bodies that touch only i64 / String / Vec / Option /
Result / control flow" might be a useful onramp for someone
porting a small Rust crate. But the feature requires a
"why would you?" justification — Rust → Resilient loses
information (Rust's type system is stronger), which is the
opposite of what users typically want from a code translator.

---

## Recommendation

1. **Land the `.res` → `.rz` extension migration first** (the
   companion PR to this doc).
2. **File a follow-up ticket for `rz emit-rust`** as a separate
   subcommand. Initial scope: function bodies + control flow +
   primitive types + `Array<T>` / `Option<T>` / `Result<T, E>`.
   Reject anything else with a structured diagnostic.
3. **Defer Rust → Resilient indefinitely.** Reopen the
   conversation when Resilient grows traits with associated
   types, file-tree modules, and a closure-capture story —
   i.e., when the language has the surface to absorb most of
   what Rust expresses.

The two directions are independent; pursuing them as separate
follow-up tickets (with the recommendation above codified)
keeps the scope manageable and avoids accidentally building a
half-finished bidirectional translator that doesn't really
work in either direction.
