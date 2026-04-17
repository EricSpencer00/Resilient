# Resilient — Phased Roadmap

A companion to `ROADMAP.md`. The ladder (G1–G20) is the ticket-level
map. This file groups goalposts into **phases** and estimates where
each phase puts us against the original pitch of "safety-critical
embedded systems language with formal verification."

Percentages are deliberately fuzzy — they capture *capability class*,
not "work remaining."

---

## Phase 0 — Infancy (0% → 15%)  ✅ COMPLETE

**Goal**: turn the half-finished MVP into a language you can actually
run programs in.

- G1 green build
- G2 test harness
- G3 drop dummy-param hack
- G4 diagnostics that don't crash the binary

Shipped: RES-001..016, RES-027, 46 tests, zero parser/lexer panics,
`cargo test` and `cargo clippy -- -D warnings` both green, GitHub
Actions CI.

---

## Phase 1 — Basic expressiveness (15% → 35%)  ✅ COMPLETE

**Goal**: a real programming language — not in the "it compiles" sense,
but in the "you can write realistic programs in it" sense.

- Full operator suite (arithmetic, comparison, logical, bitwise, shifts)
- `while`, `for..in`, `if/else`
- Arrays + indexing + push/pop/slice
- String ops + len
- 13-function stdlib
- Assignment, static let, forward refs
- **G8 function contracts** (`requires`/`ensures`)  ← first step of verifiability
- **G10 live-block invariants**  ← second step of verifiability

Shipped: RES-017..037 (everything except G6/G7/G9 bricks).

---

## Phase 2 — Structured data & error handling (35% → 50%)  ✅ COMPLETE

**Goal**: programs can express real domain logic, not just numeric
scripts. This is the single most requested class of features by
realistic example programs.

| Ticket | Adds | Status |
|---|---|---|
| RES-038 | **Structs / records** — user-defined product types, field access `.`, closes G12 | ✅ |
| RES-039 | **`match` expressions** with literal + identifier + wildcard patterns, closes G13 at MVP | ✅ |
| RES-043 | **String builtins** — split, trim, contains, to_upper, to_lower | ✅ |
| RES-040 | **`Result` type** — explicit failure handling, Ok/Err constructors, is_ok/is_err/unwrap | ✅ |
| RES-041 | **`?` propagation operator** on `Result` | ✅ |
| RES-042 | **Closures** — anonymous `fn`, by-value env capture; mutation-sharing deferred to Phase 3 refactor | ✅ |

**Phase 2 closing note**: capture-by-value matches most common
closure needs (filters, transformers, early-binding of parameters).
Shared-mutation closures require `Environment = Rc<RefCell<...>>`
which is folded into Phase 3's AST hardening — doing both refactors
at once minimizes churn.

---

## Phase 3 — Real type system (50% → 65%)  🟡 IN PROGRESS

**Goal**: the typechecker becomes load-bearing. Bad programs get
*rejected* at compile time, not at runtime through string errors.

| Ticket | Adds | Status |
|---|---|---|
| RES-052 | **Typed declarations**: `let x: int = 0;`, `fn f() -> int`, typed array literals | ✅ |
| RES-053 | **G7 typechecker rejection**: emit real errors for type mismatches; `let x: int = "hi"` fails before runtime | ✅ |
| RES-054 | **Exhaustiveness checking for `match`** — compile-time error if a `bool` arm is missing, or a scalar match lacks a default | ✅ |
| RES-050 | **G6 AST hardening** — one canonical AST module with `Span` on every node, delete the unwired `parser.rs`, Environment becomes `Rc<RefCell<...>>` | ⏳ |
| RES-051 | **G5 logos lexer** — replace the hand-rolled one once spans are in | ⏳ |
| RES-055 | **Generic builtins / simple polymorphism** — `abs<T>(x: T) -> T` | ⏳ |
| RES-056 | **Shared-mutation closures** — once Environment is `Rc<RefCell<...>>` via RES-050, rework `Value::Function` to share env | ⏳ |

**Progress: 3/7 tickets. Typecheck rejection is live today —
`cargo run -- --typecheck foo.rs` exits 1 on ill-typed programs.**

**Definition of done for Phase 3**: a type error in any program is
rejected by `resilient --typecheck file.rs` with a pointed diagnostic;
the interpreter never sees an ill-typed program.

---

## Phase 4 — The verifiability payoff (65% → 80%)  🟡 STARTED

**Goal**: the original pitch becomes true. `requires`/`ensures` are
proved correct for *all* inputs in their declared range, not just
the ones the test suite happens to exercise.

This is the phase where Resilient *earns its name*.

| Ticket | Adds | Status |
|---|---|---|
| RES-060 | **G9a constant folder** — discharges tautologies and rejects contradictions in contract clauses with no free variables. | ✅ |
| RES-061 | **G9b call-site fold** — substitutes literal arguments for parameters, then folds. `divide(10, 0)` is rejected at compile time when `divide` requires `b != 0`. **First real symbolic verification.** | ✅ |
| RES-062 | **Flagship example** `sensor_monitor.rs` exercising every Phase 1–3 feature plus contracts; pinned by golden test. | ✅ |
| RES-063 | **Const-let propagation** — verifier follows `let n = 5;` through to `pos(n)`, treating n as constant. | ✅ |
| RES-064 | **Flow-sensitive if-branch assumptions** — `if x == 0 { divide(10, x); }` rejected at compile time. First control-flow-aware verification. | ✅ |
| RES-065 | **Caller-requires propagation** — caller's preconditions become assumptions inside its body, so contracts chain across function boundaries. | ✅ |
| RES-066 | **`--audit` flag** — verification certificate with discharged-vs-runtime stats. Closes the user-facing loop. | ✅ |
| RES-061 | **G9b Z3 integration** (optional feature flag `--features z3`) — translate contract AST to SMT-LIB, discharge at compile time, fall back to runtime check on unknown |
| RES-062 | **Verification certificate** — on success, emit `<file>.vcert` pinning the solver version, the query, and the answer. Commit these alongside source. |
| RES-063 | Live-block invariants verified symbolically (G10 meets G9) |
| RES-064 | **G8 contracts on builtins** — `abs` gets `ensures result >= 0`, every stdlib fn is re-provable |
| RES-065 | Counterexample extraction — a failed proof emits the specific input that breaks the contract |

**Definition of done for Phase 4**: `resilient --verify program.rs`
exits 0 iff every `requires`/`ensures`/`invariant` in the program is
provably discharged. A user with no SMT background can write
contracts and get static safety.

---

## Phase 5 — Platform & ecosystem (80% → 95%)

**Goal**: move from "works on the author's laptop" to "runs in the
settings it was designed for." This is where Resilient becomes a
language you could realistically deploy.

| Ticket (future) | Adds |
|---|---|
| RES-070 | **Module system** — `import sensor; use sensor::read` |
| RES-071 | **G15 cranelift backend**: emit native code for the verified subset |
| RES-072 | **G16 `no_std` runtime**: compile a minimal Resilient program to a Cortex-M target under QEMU |
| RES-073 | **G17 LSP** over stdin/stdout — diagnostics, hover types, go-to-definition |
| RES-074 | **G18 effect tracking** — `fn read_sensor() -> int !io` ; I/O and non-determinism propagate transitively |
| RES-075 | Package manager (`resilient.toml`, `resilient-pkg add ...`) |
| RES-076 | Documentation generator — pull `requires`/`ensures` into rendered docs |

**Definition of done for Phase 5**: a published crate/package on a
package index; a deployment story for at least one embedded target.

---

## Phase 6 — The endgame (95% → 100%)

**Goal**: Resilient demonstrates the full loop. The language is
expressive enough to describe, verify, and compile itself.

| Ticket (future) | Adds |
|---|---|
| RES-080 | **G19 proof-carrying assertions** — distributed artifacts include verification certificates; consumers can re-verify with their own solver |
| RES-081 | **G20 self-hosting milestone** — a Resilient program that parses, type-checks, or verifies a non-trivial Resilient program. Goal isn't full self-hosting (that's multi-year); it's a *credible demo* that the language is powerful enough to reason about itself. |

---

## Non-goals (explicit)

Things the roadmap deliberately does NOT commit to, so that scope
doesn't creep:

- **Multi-threading or async**. Safety-critical embedded code usually
  runs in a cooperative scheduler or a stateful ISR; neither is well-
  served by language-level async. Revisit post-Phase 5 if demand.
- **GC**. Arrays are value types today; Phase 2 keeps it that way. A
  reference-counting variant may appear for closures (RES-042), but
  there's no plan to add a tracing collector.
- **Macros**. The pitch is *simplicity*. Metaprogramming adds surface
  area and opacity; if Phase 2's `match` + Phase 3's generics aren't
  enough expressivity, revisit then.
- **Arbitrary-precision arithmetic**. `int` is i64, `float` is f64.
  Embedded code doesn't need bignums, and the SMT backend is easier
  over bounded integers.

---

## Where we are right now

- Phase 0 ✅
- Phase 1 ✅
- Phase 2 🟡 **next up** — the sub-tickets above will be minted as
  the board catches up. Structs (RES-038) is the natural starting
  point.
- Phases 3–6 ⏳

~35% of the way. Phase 2 is a realistic next-session target. Phase 3
is the biggest single jump because it makes the typechecker real.
Phase 4 is the one that *delivers the pitch*.
