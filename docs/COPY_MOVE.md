# Copy vs. Move Semantics (RES-4079 design lock-in)

Linked from [MEMORY_MODEL.md's Enforcement Reality Check](MEMORY_MODEL.md#enforcement-reality-check-what-is-actually-checked-today).
Resolves the design question opened by [#4079](https://github.com/EricSpencer00/Resilient/issues/4079),
deferred from the A-E5 use-after-move follow-up [#4070](https://github.com/EricSpencer00/Resilient/issues/4070).

## The question

Outside `linear T` (`resilient/src/linear.rs`), does the language have
a Copy/Move type distinction for plain (unannotated) bindings? If a
plain binding is passed to a function and the caller reads it again
afterward, is that:

- always fine (Copy semantics — the callee got an independent value), or
- sometimes a bug (Move semantics — the callee took ownership and the
  caller's binding is now stale)?

This has to be pinned down before any use-after-move checker for plain
bindings can be sound, because the two answers produce opposite
verdicts on the same program.

## Survey: what the interpreter/VM actually do today

`resilient/src/linear.rs` is explicit that `linear T` is a **type-annotation
marker**, not a runtime representation change — `is_linear`/`strip_linear`
just read a `"linear "` string prefix off the existing type-annotation slot.
There is no separate "moved-from" runtime state for non-linear values: a
plain local is a value in the interpreter's environment map, and passing it
to a call evaluates the argument expression to a value and binds that value
(or an equal clone, depending on backend — see `docs/differential.rs`/VM
parity notes) into the callee's frame. Nothing in the calling convention
removes or poisons the caller's binding. Concretely:

```rust
fn take(x: int) -> int { return x + 1; }

fn main() {
    let a = 5;
    let b = take(a);
    let c = a; // reads `a` again after passing it — legal today, always has been
}
```

This is true not just for primitives but for every non-`linear` type in the
language today, including arrays and structs: there is no ownership-transfer
calling convention, no "moved-from" poison state, and no expression syntax
that takes a reference to a local for a value type to alias into (per the
Enforcement Reality Check, `&`/`&mut` only appear in parameter/`let` *type*
annotations, never as an expression). So **the runtime reality is
Copy-everywhere** for anything that isn't `linear`.

## Options considered

### A. Everything-Copy (no move checking on unannotated bindings)

Formalize the status quo: every non-`linear`, non-reference type is Copy.
Re-reading a plain binding after passing it anywhere is always legal,
because it already always evaluates to an independent value under the
current calling convention. `linear T` remains the sole move-semantics
surface, enforced by the existing `check_linear_usage` pass.

- **Zero false positives** — literally changes nothing about which
  programs compile; this is what already happens.
- Matches reality exactly: no new runtime behavior, no new syntax, no
  new diagnostics to design.
- Does not by itself provide move-based resource safety for
  heap-backed/handle-like types — but those already have an opt-in
  mechanism (`linear T`) precisely for this purpose.

### B. Infer Move for resource types only (files/handles/heap-backed)

Structurally infer Move for certain type shapes (heap-backed structs,
handles) without an explicit annotation, and reject use-after-move for
those specifically.

- Requires the compiler to walk field types transitively to classify
  Copy vs. Move, and requires deciding this **today, in this PR**,
  which fields/types count as "resource-like" — i.e. re-litigating a
  Rust-style `derive(Copy)`-by-absence rule with no annotation surface
  to opt out.
- Highest risk of false positives: any existing program built assuming
  today's Copy-everywhere behavior for a struct that gets reclassified
  as Move under this rule would start failing to compile — a direct
  violation of the project's zero-false-positive doctrine for new
  static checks.
- Silently changes the calling convention's meaning for existing code
  without any change in syntax, which is confusing: two calls that look
  identical (`f(x)`) would have different semantics depending on `x`'s
  inferred type-classification, with no marker in the source to see it.

### C. Explicit opt-in `move` keyword/expression

Add concrete move syntax (e.g. a `move` keyword or expression) and
enforce use-after-move only for bindings passed through it, leaving all
implicit calling-convention passes as Copy.

- Zero false positives on existing code, since old programs contain no
  `move` expressions and are entirely unaffected.
- But this requires designing and shipping a new expression-level
  syntax feature (lexer, parser, AST node, typechecker plumbing) — a
  substantially larger scope than "decide Copy vs. Move," and the kind
  of ownership-transfer expression surface `docs/MEMORY_MODEL.md`
  explicitly flags as not existing yet for `&`/`&mut` either.

## Decision

**Adopt Option A: everything-Copy for all non-`linear`, non-reference
types.** No new use-after-move checking is added for plain bindings.
`linear T` remains the only move-semantics surface in the language, and
`check_linear_usage` (`resilient/src/linear.rs`) remains the only
use-after-move enforcement pass.

### Rationale

- **Zero-false-positive doctrine.** This project's CI-as-merge-gate model
  (see root `CLAUDE.md`) requires new static checks to have zero false
  positives. Option A is the only one of the three that provably
  satisfies this by construction — it changes no runtime semantics and
  adds no new rejection path, so no previously-compiling program can
  start failing.
- **Matches implemented reality.** The survey above shows the
  interpreter already treats every non-`linear` value as Copy. Codifying
  that as the design decision documents truth rather than aspiration —
  consistent with the "Enforcement Reality Check" methodology already
  used elsewhere in `MEMORY_MODEL.md`.
- **Safety-critical positioning is served by the existing opt-in, not a
  silent structural inference.** Resilient already has a purpose-built
  mechanism for "this must be consumed exactly once, and re-reading it
  after consumption is a bug": `linear T`. A program author who needs
  move semantics for a resource (file handle, memory-mapped device,
  allocation) opts in explicitly and gets full use-after-move
  enforcement today, with a diagnostic pointing at the double-use site.
  There is no unserved use case that Option B or C would additionally
  cover for safety-critical code that `linear T` doesn't already cover
  in a more auditable, explicit way.
- **Options B and C both require a second design-and-implementation
  project of their own** (structural Copy/Move field-walk rules, or a
  new ownership-transfer expression syntax) before any use-after-move
  enforcement could ship soundly. Given the zero-false-positive
  constraint, that work is properly its own future ticket if a concrete
  use case for it emerges — it should not be smuggled in as an
  extension of this design decision.

### Consequence for #4070's remaining scope

#4070 item 1 ("use-after-move for unannotated bindings") is resolved:
**no unannotated-binding use-after-move checker will be built.** There is
nothing to implement here — the decision itself is the full resolution,
matching the scope #4079 was opened for. #4070's other deferred items
(conditional-path aliasing beyond straight-line `let`-copies,
interprocedural aliasing, local-to-local reference aliasing) are
unaffected by this decision and remain open follow-up work tracked on
that issue, since they concern `&`/`&mut` reference aliasing, not
Copy/Move on plain values.

If a future ticket wants Move semantics beyond `linear T` (e.g. a `move`
expression per Option C), it should treat this document as the baseline
survey and open a fresh design ticket — do not retrofit move-checking
onto today's silent call-by-value convention.
