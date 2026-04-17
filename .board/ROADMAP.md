# Resilient Roadmap

North star: turn Resilient from an MVP interpreter into a serious
formally-verifiable language for safety-critical embedded systems. The
Manager keeps this roadmap current; the Executor only reads it.

## Vision

- **Resilience** — programs self-heal via `live` blocks and recover to a
  known-good state on recoverable faults.
- **Verifiability** — function contracts (`requires` / `ensures`) and
  system invariants are checked statically where possible, at runtime
  where not.
- **Simplicity** — minimal syntax, no dummy-parameter hacks, clear
  diagnostics with file:line:col.

## Goalpost ladder

Each goalpost is a checkpoint that unlocks the next. We land one at a
time, commit it, and only then move the post.

### Foundation
| # | Goalpost | Success criterion |
|---|---|---|
| **G1** | Green build | `cargo build` clean, binary runs. ✅ RES-001 |
| **G2** | Test harness | `cargo test` exists, unit tests for lexer/parser/typechecker, golden tests run every example. |
| **G3** | Drop dummy parameters | `fn main()` parses; all existing examples updated; SYNTAX.md updated. |
| **G4** | Diagnostics | Every error includes `file:line:col`, a snippet, and a caret. Lexer/parser track spans. |
| **G5** | Proper lexer | Replace hand-rolled lexer with `logos` (already a dep). Equivalent tokens, fewer LOC. |

### Language sanity
| # | Goalpost | Success criterion |
|---|---|---|
| **G6** | AST hardening | One canonical AST module. Spans on every node. Display/Debug derived. parser.rs either adopted or deleted. |
| **G7** | Real type checker | Type inference, unification, exhaustiveness on if/else returns, rejects ill-typed programs in tests. |
| **G8** | Function contracts | `requires` / `ensures` clauses parse and are checked at runtime (precursor to symbolic verification). |

### Verifiability
| # | Goalpost | Success criterion |
|---|---|---|
| **G9** | Symbolic assert | Integer-domain `assert` checked at compile time with a small SMT layer (`z3` binding or a custom bounded verifier). |
| **G10** | Live-block invariants | Invariants declared on `live { }` blocks, re-checked on every retry, violations reported with ticket-style output. |

### Future (not scheduled)
- Stdlib with print/read/math primitives
- Cranelift or LLVM backend
- LSP server
- Embedded target (`no_std`, Cortex-M)
- Effect tracking for I/O and non-determinism

## Moving the post

When the Executor lands a ticket that closes a goalpost, the Manager
updates the "Success criterion" column with a ✅ and the RES id, then
drafts new tickets for the next goalpost. The roadmap itself is a
living document — Manager may add/remove/reorder goalposts as learning
accumulates, but should leave a note in this file's changelog below.

## Changelog

- 2026-04-16 — ladder seeded; G1 landed (RES-001).
