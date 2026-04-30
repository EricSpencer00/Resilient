# Generic Monomorphization Design

**Date:** 2026-04-30
**Status:** Design lock-in for [#368 RES-405](https://github.com/EricSpencer00/Resilient/issues/368) (`needs-design before implementation`)
**Tracking:** RES-405
**Unblocks:** [#366 RES-403](https://github.com/EricSpencer00/Resilient/issues/366) first-class fn values, [#365 RES-402](https://github.com/EricSpencer00/Resilient/issues/365) polymorphic Array/Option/Result, RES-290 trait bound-checking

---

## Why this document exists

[#368](https://github.com/EricSpencer00/Resilient/issues/368)
is tagged `needs-design` because there are several plausible
implementation strategies for `fn<T>(x: T) -> T` and picking
the wrong one forces re-work later. The design space:

- Erasure (boxed values, single compiled body) vs.
  monomorphization (one compiled body per instantiation) vs.
  hybrid (erased in the interpreter, monomorphized in the JIT).
- Inference algorithm — Hindley-Milner full type variable
  unification vs. local-only inference vs. no inference.
- Where in the pipeline does substitution happen — pre-typecheck,
  during typecheck, post-typecheck.
- How calls into generic fns surface in the bytecode VM and
  Cranelift JIT.

This doc picks an answer for each, gives the tradeoff, and
folds the answer back into [#368](https://github.com/EricSpencer00/Resilient/issues/368)'s
acceptance criteria.

---

## Q1. Erasure vs. monomorphization vs. hybrid

### Recommendation: **monomorphization in the bytecode VM and JIT; erasure (parameter-driven dispatch) in the tree-walking interpreter**

This is the hybrid: each backend gets the strategy that fits
its execution model.

- **Tree-walking interpreter** — values already carry type
  tags (`Value::Int`, `Value::Float`, etc.), so a generic body
  can dispatch by inspecting the runtime tag. No
  monomorphization needed; one compiled `Node::Function` body
  serves every instantiation. The substitution is recorded at
  the call site so the typechecker can validate the
  instantiation.
- **Bytecode VM** — generates one specialized `Chunk` per
  instantiation observed at compile time. Calls to
  `id::<Int>(5)` resolve to a different `OpCall` target than
  `id::<Float>(5.0)`. Caching prevents re-codegen for the
  same instantiation.
- **Cranelift JIT** — same as the VM, except the specialized
  Chunk is also lowered to native machine code.

### Tradeoffs

| Strategy | Walker | VM | JIT |
|---|---|---|---|
| Pure erasure | Free (already tag-dispatched) | Loses VM perf — every call goes through tag-dispatch | Defeats the JIT — no monomorphization means no constant folding |
| Pure monomorphization | Wastes walker memory — every body cloned per instantiation | Right answer | Right answer |
| Hybrid (recommended) | Walker stays cheap; VM and JIT keep perf | | |

### Why hybrid wins

The three backends have different perf goals: the walker is
the "let me try a thing fast" path, the VM and JIT are the
"this is the actual deploy target" paths. Forcing one strategy
across all three loses on either end. Hybrid is more code, but
each backend's contribution is small (the walker change is
~zero, the VM change is one cache + one specializer pass, the
JIT change is a wrapper over the VM's specializer).

### V1 acceptance criteria absorbed

- Walker: extends `Interpreter::eval` to record the `T = …`
  substitution at every generic-call site; uses it in
  diagnostic messages but not for runtime dispatch (which
  stays tag-driven).
- VM: new `monomorph_cache` field on the compiler keyed by
  `(fn_id, type_args)`. First call with a given key codegens
  a specialized chunk; subsequent calls with the same key
  reuse it.
- JIT: same cache, lowering through the existing `compile_chunk`
  path.
- Each backend's tests verify the relevant invariant (walker:
  type-tag dispatch produces correct results; VM: each
  instantiation gets its own chunk; JIT: monomorphized native
  code is generated).

---

## Q2. Inference algorithm

### Recommendation: **bidirectional local inference with explicit instantiation as a fallback**

When the user writes `id(5)`, the typechecker:

1. Looks at the formal parameter type `T`.
2. Looks at the actual argument's inferred type `Int`.
3. Unifies `T` with `Int`; records `T = Int` in the
   substitution map for this call site.
4. Substitutes `T` with `Int` throughout the function body for
   typechecking purposes.

When the user writes `id::<Int>(5)`, step 3 is skipped — `T =
Int` is taken from the explicit annotation. Step 4 still runs.

If unification fails (e.g., `id(5)` where the body forces `T
= String`), or if the argument types are too underdetermined
to infer (`id(None)` — what's the inner type?), the
typechecker emits an error pointing at the call site and
suggests explicit instantiation.

### Tradeoffs

| Approach | Pro | Con |
|---|---|---|
| No inference (always explicit) | Simplest | Every generic call needs `<T = …>`; user-hostile |
| Local bidirectional (recommended) | Matches Rust's "expected type → inferred type" UX; requires no global type-variable graph | Won't infer through deep call chains the way Hindley-Milner does |
| Full Hindley-Milner | Maximally inferential | Expensive — a global unification graph; HM walker is its own ticket (RES-120, blocking #366); blocks ahead of the existing schedule |

### Why local bidirectional wins

Resilient already has bidirectional inference for non-generic
type-checking (a function-call expression's expected type
flows from the surrounding context; an argument's inferred
type flows up). Extending it to generic substitutions is a
local change — no new "global type variable graph" data
structure. RES-120's HM walker is for a different purpose
(closure-capture types) and is currently blocking #366; we
do *not* want #368 to inherit that block.

If a future use case needs HM-strength inference for generics
(e.g. `let xs = []; let y = first(xs)` where `first` returns
the first element of an array), we revisit then. The local
algorithm is forward-compatible; you can layer HM on top
without re-doing the substitution machinery.

### V1 acceptance criteria absorbed

- New `monomorph::infer_subst(call_site, fn_decl) -> Result<HashMap<TypeVar, Type>>`
  that runs after argument-typechecking and produces the
  substitution map.
- Failure surfaces as `error: cannot infer type parameter T;
  add explicit instantiation: id::<???>(5)` with the call site
  highlighted.
- Body-consistency check (per the ticket's acceptance
  criterion): if the substitution implies `T = Int` but the
  body uses `T` in a String context, the error points at the
  body inconsistency, not the call site.

---

## Q3. Where in the pipeline does substitution happen

### Recommendation: **post-typecheck, in a dedicated `monomorph::lower` pass**

The pipeline becomes:

```
parse → named_args lower → default_params lower → newtypes lower
      → typecheck (records substitutions on every generic call)
      → monomorph::lower (emits one specialized body per
                           (fn_id, type_args) seen)
      → backend (walker / VM / JIT)
```

The typechecker records substitutions but does NOT alter the
AST — it just adds annotations on call-site nodes. The
`monomorph::lower` pass walks the AST once, finds every
generic-call site, and emits cloned-and-substituted function
bodies for the backends to use. The walker can skip this pass
(it dispatches by tag); the VM and JIT both consume it.

### Tradeoffs

| Where | Pro | Con |
|---|---|---|
| Pre-typecheck | Avoids re-checking substituted bodies | Substitution can't depend on inference results — every call needs explicit `<T = …>` |
| During typecheck | Single pass | Mixes two concerns; bug surface inflates |
| Post-typecheck (recommended) | Substitution sees full inference results; lowering pass is independently testable | One more pass over the AST; substituted bodies need re-typechecking against their concrete types (mostly free — the existing typechecker just runs again on the lowered body) |

### Why post-typecheck wins

It separates two concerns cleanly: typechecking is "do the
types work as written" (with type variables); lowering is
"replace the variables with concrete types and emit
specialized bodies". Each is testable in isolation. It also
matches the pattern Rust's compiler uses (`rustc -Z dump-mir`
shows the substituted version after monomorphization).

The "one more pass" cost is real but bounded. Empirically,
post-typecheck monomorph passes in production compilers run
in linear time over the AST; we have no reason to expect
worse.

### V1 acceptance criteria absorbed

- New module `resilient/src/monomorph.rs` hosting the lowering
  pass. Per the feature-isolation pattern in CLAUDE.md.
- New `<EXTENSION_PASSES>` arm in `typechecker.rs` (or a
  dedicated post-typecheck driver call) invoking
  `monomorph::lower(program, type_subst_map)`.
- Lowering produces new `Node::Function` entries with
  generated names like `id$Int` (the `$` is a separator that
  user identifiers can't contain, so collisions are
  impossible).
- Original generic `Node::Function` is preserved — the walker
  uses it directly.

---

## Q4. Trait bounds — interaction with #290

### Recommendation: **bounds parsed in V1; bound-checking deferred to RES-290's PR**

The grammar `fn<T: Trait>(x: T)` parses today (RES-289 already
landed it); V1's typechecker accepts the bound but doesn't
validate it. RES-290 (trait system) adds the bound-checking
logic when it lands.

V1 of #368 simply forwards bounds through monomorphization —
each instantiation carries the bound list, and RES-290's
checker reads it.

### Tradeoffs

| Approach | Pro | Con |
|---|---|---|
| Block #368 on RES-290 | Bounds are validated from day 1 | RES-290 is a complex ticket of its own; blocking #368 pushes generics ship-date out indefinitely |
| Forward bounds through, validate later (recommended) | Generics ship; RES-290 completes the picture later | Programs that *should* fail bound-checks compile silently in V1 |
| Strip bounds in V1 | Simplest | `<T: Trait>` becomes `<T>` — programs that depend on bound-equipped methods stop working |

### Why forwarding wins

Resilient's V1 ship sequence already has trait bound checking
in flight (RES-290). The window between #368 V1 and RES-290
landing is the only time programs can compile despite a
broken bound — and it's a backstop window, not a permanent
state. The error surface "I wrote `<T: Trait>` and the body
called `t.trait_method()` but the call site provided a `T`
that doesn't impl `Trait`" is exactly what RES-290 will flag.

### V1 acceptance criteria absorbed

- The substitution map's value type is `Type` plus an optional
  `Vec<TraitBound>` carried verbatim from the declaration.
- Monomorphization preserves the bound list on each
  generated specialized body; RES-290's checker reads it.
- Until RES-290 lands, accidentally-broken bounds compile
  with a runtime error if the trait method is invoked.

---

## Sign-off summary

| # | Question | Recommendation | Risk if wrong |
|---|---|---|---|
| Q1 | Erasure vs monomorph | Hybrid — walker erases, VM and JIT monomorphize | Low — flipping later is per-backend, no AST/Type changes |
| Q2 | Inference algorithm | Local bidirectional with explicit fallback | Medium — strengthening to HM later is layerable; weakening would break programs |
| Q3 | Where to substitute | Post-typecheck, dedicated `monomorph::lower` | Low — moving the pass earlier later is mechanical |
| Q4 | Trait bound interaction | Forward; validated by RES-290 | Low — RES-290 is on the same ship-train |

---

## What this spec does NOT decide

- HM walker integration. RES-120 is its own ticket; it's about
  closure-capture inference, not generics.
- The mangling scheme for instantiated names beyond the
  `id$Int` pattern. The pattern works for primitives and
  user-defined types alike; complex generic instantiations
  (`Pair$Int$String$Vec`) flatten to the same scheme.
- Whether monomorphization copy-clones or shares interior
  immutable nodes. Implementation detail; ship clones first,
  benchmark, sharer-share later if needed.
- The interaction with RES-402 polymorphic Array/Option/Result.
  Q1's hybrid strategy applies the same way; RES-402 just
  provides more types that benefit from monomorphization.

---

## V1 implementation order (informational)

The implementation work for [#368](https://github.com/EricSpencer00/Resilient/issues/368)
naturally splits into 4 PRs:

1. **PR 1**: substitution machinery — `TypeVar`, `Subst`,
   typechecker records `<T = Int>` on every generic call.
   No backend changes yet — walker / VM / JIT still ignore
   the records, so existing behaviour is preserved.
2. **PR 2**: walker plumbing — uses the substitution for
   diagnostics ("expected `T` (= Int), got String") but
   continues to dispatch by tag.
3. **PR 3**: VM monomorphization — `monomorph::lower` pass
   producing specialized chunks; cache; one chunk per
   `(fn_id, type_args)`.
4. **PR 4**: JIT monomorphization — same cache, native lowering.

After PR 4, `<T: Trait>` parses and forwards bounds for
RES-290 to validate when it lands.
