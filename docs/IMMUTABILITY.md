# Binding Mutability (RES-4197)

## Current semantics (as of v1.0.x)

Resilient performs **no mutability enforcement on `let` bindings**. Every
`let` binding is reassignable:

```resilient
let x = 5;
x = 6; // accepted today — no diagnostic, no typecheck error
```

This is true regardless of whether the binding is a local, a loop variable,
or a function parameter. The typechecker has no notion of a binding's
mutability at all — it tracks types, not mutation permissions.

Two keywords already exist in the lexer/parser that might look related but
are **not** connected to this behavior:

- `mut` — used exclusively in reference types: `&mut T`, `&mut[R] T`. It
  marks a reference as exclusive/writable through that reference; it has no
  meaning attached to a plain `let` binding. `let mut x = 5;` is not special
  — `mut` is not a valid token in that position today.
- `const` — used exclusively for top-level `const` *statements*
  (module-level constant declarations), parsed by `parse_const_statement`.
  It does not exist as a per-binding qualifier on `let`.

### The orphaned E0012 code

`resilient/src/diag.rs` reserves diagnostic code `E0012` for "reassignment
of an immutable (`let`) binding," but no pass in the typechecker ever emits
it — confirmed independently while investigating PR #4162 and PR #4195.
The registry entry describes intent that was never wired up; this document
is the design record for what to do about that gap.

## Is this part of the stable surface?

`STABILITY.md` documents `let`, `fn`, `if`/`else`, `while`, `match`, and
`return` as part of the **Stable** core syntax tier, but it does not make
any claim about reassignment semantics specifically — mutability was never
called out as a designed guarantee in either direction. That said,
reassignment of `let` bindings is **de facto stable**: it has always
worked, and code in the wild (including our own example corpus) already
relies on it.

### Corpus evidence

Sampling the example corpus at `resilient/examples/*.rz` (638 `.rz` files)
for the pattern "a name bound via `let` is later reassigned with `=`,
`+=`, `-=`, `*=`, or `/=`" (see the heuristic script referenced below):

- **44 / 638 files (6.9%)** contain at least one reassignment of a
  `let`-bound name.
- **92 total reassignment occurrences** across those files.

Reassignment is a minority pattern but not a rare one — common in loop
accumulators, running totals, and iterative algorithms. Any change that
makes reassignment an error by default would break roughly 1 in 14
existing example programs, and by extension a comparable slice of
real-world programs using the same idioms.

## Design decision

Given the safety-critical positioning of Resilient (where "this binding
never changes after this point" is a valuable, checkable property) and the
corpus evidence that most bindings are *not* reassigned, the recommended
shape is:

1. **v1.x — no breaking change.** Reassignment of any `let` binding
   remains legal, exactly as today. `STABILITY.md` is updated to state
   this explicitly as a Stable guarantee, closing the ambiguity that
   prompted this ticket.
2. **v1.x — opt-in enforcement.** Introduce a new binding form,
   `let const NAME = ...;` (reusing the existing `const` keyword rather
   than inventing a new one — see "Alternatives considered"), that
   typechecks as immutable: any subsequent bare assignment to `NAME` is a
   compile error reported as `E0012`. This is purely additive grammar —
   it only changes the meaning of programs that opt into the new syntax,
   so it cannot break existing code and does not require a STABILITY.md
   breaking-change process.
3. **Lint-level nudge (data-gated).** Once opt-in `let const` has shipped
   and had a release or two of real usage, add an *opt-in* lint
   (`--lint immutable-by-default` or similar, off by default) that flags
   `let` bindings which are never reassigned and suggests `let const`.
   This produces the corpus data point that matters for the 2.0 call: if
   a future corpus sample shows the overwhelming majority of `let`
   bindings are never reassigned (consistent with today's 93.1% of files
   having zero reassignment), that is the evidence base for flipping the
   default in a 2.0 major version. If usage patterns shift, the
   flip is reconsidered or dropped — this document does not pre-commit
   to it.
4. **v2.0 (tentative, not committed)** — only if the lint data supports
   it: flip the default so plain `let` becomes immutable and a new
   `let mut` form is required for reassignment. This is a breaking change
   and requires its own STABILITY.md major-version process; it is out of
   scope for any v1.x ticket and is *not* authorized by this document.

### Alternatives considered

- **New `final` or `imm` keyword.** Rejected for the opt-in increment:
  `const` already exists in the lexer/parser for a conceptually adjacent
  purpose (module-level immutable values) and reusing it as a `let`
  modifier avoids adding a fourth binding-related keyword to the surface
  syntax (`let`, `mut`, `const`, plus a new one). `let const` reads
  naturally as "a `let` binding with `const` semantics."
- **Immutable-by-default in v1.x.** Rejected: breaks ~6.9% of the example
  corpus outright and is exactly the kind of "new rejection on existing
  programs" STABILITY.md calls out as a hard stop without maintainer
  sign-off. Not attempted here.
- **Retire E0012 and do nothing.** Rejected: mutability tracking is cheap
  to add opt-in and valuable for safety-critical code (e.g. asserting a
  sensor calibration constant is never accidentally overwritten in a
  500-line function). Retiring the code forecloses that value for no
  benefit — the registry slot costs nothing sitting unused.

## Phased implementation plan / tickets to file

| Phase | Scope | Status | Ticket |
|---|---|---|---|
| 0 | This document — honest current-state doc + design decision (this PR) | ✅ done | RES-4197 |
| 1 | `STABILITY.md`: add explicit line under Stable core syntax stating `let` reassignment is a guaranteed-stable behavior | ☐ open | follow-up, small |
| 2 | Lexer/parser: accept `let const NAME = expr;` — a `bool` `is_const` flag added to the existing `Node::LetStatement` variant (see "Alternatives considered" — the flag shape was chosen over a new `Node` variant to avoid touching the ~70 files that pattern-match `Node::LetStatement`) | ✅ done | shipped this PR |
| 3 | Typechecker: track per-binding immutability, path-insensitive, same-function; emit `E0012` on any bare assignment (`=`, or a compound assign — the parser already desugars those to a plain assignment) to a `let const` binding. Lives in `resilient/src/immutability.rs`, wired via the `typechecker.rs` `<EXTENSION_PASSES>` block. Shadowing via a new `let` in the same or a nested scope is allowed and ends enforcement for that name from that point on. `RESILIENT_RICH_DIAG` gates the `[E0012]` label, matching every other registry code. | ✅ done | shipped this PR |
| 4 | Opt-in lint pass (`immutable-by-default` suggestion) gated behind a flag, off by default; instrumentation to gather real corpus/usage stats | ☐ open | follow-up, depends on Phase 3 |
| 5 (tentative) | Revisit default-immutability for a 2.0 major version based on Phase 4 data | ☐ open, not filed | explicitly deferred, requires maintainer sign-off per STABILITY.md major-version process |

Phases 1-4 are each independently shippable, additive-only PRs consistent
with the incremental-PR guidance in `CLAUDE.md`. Phases 2 and 3 shipped
together in the PR that added this note (RES-4197) — see
`resilient/src/immutability.rs` for the enforcement pass and
`resilient/src/lib.rs`'s `parse_let_statement` for the opt-in grammar.
Phases 1, 4, and 5 stay open; Phase 1 and 4 are tracked under the
original `#4197` issue (`Refs #4197`) for follow-up.

### Phase 2/3 syntax and semantics (shipped)

```resilient
fn calibrate() -> int {
    let const limit = 10;   // opt-in immutable binding
    let total = limit + 5;  // reading a `let const` is unrestricted
    // limit = 20;           // rejected: E0012 — cannot reassign `limit`
    if total > limit {
        let limit = 99;      // a fresh `let` (const or not) shadows —
        // limit = 100;       //   this reassignment targets the *new*,
                              //   non-const `limit` and is allowed.
    }
    total
}
```

- Enforcement is **same-function**: a `let const` in one function has
  no effect on any other function (each `fn` body — and each
  `ImplBlock` method body — gets a fresh scope stack).
- Enforcement is **path-insensitive**: a reassignment inside an
  `if`/`while`/`for`/`match` arm is rejected the same as one at the top
  of the function, regardless of whether the branches are jointly
  reachable.
- Enforcement is **provable-only**: the pass proves "this name was
  declared `let const` in an enclosing lexical scope, and this
  statement writes to it with no intervening shadowing `let`." It does
  not attempt aliasing or indirect-mutation analysis (e.g. mutation
  through a struct field or a reference) — those are out of scope for
  RES-4197.
- `let const (a, b) = ...;` (tuple destructure) and
  `let const Struct { .. } = ...;` (struct destructure) are **not**
  supported — the modifier is silently dropped for those forms, which
  matches the pre-existing behavior of the `let mut` sugar (RES-922).

## Reproducing the corpus measurement

The reassignment count above was produced by scanning every `.rz` file
under `resilient/examples/` for a `let NAME = ...` (or `let mut NAME = ...`)
binding followed anywhere later in the file by a bare `NAME =`, `NAME +=`,
`NAME -=`, `NAME *=`, or `NAME /=` statement. This is a textual heuristic
(no scope/shadowing awareness), so it is a reasonable lower-bound signal
of reassignment prevalence, not an exact compiler-verified count.

## See also

- `docs/LANGUAGE.md` — feature tier classification (this document is
  linked from the Stable-tier section covering `let`)
- `STABILITY.md` — stable surface and breaking-change policy
- `resilient/src/diag.rs` — `E0012` registry entry
