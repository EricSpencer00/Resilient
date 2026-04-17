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
| # | Goalpost | Status |
|---|---|---|
| **G1** | Green build | ✅ RES-001, clippy clean RES-007 |
| **G2** | Test harness | ✅ RES-002 (19 tests), RES-003 (`println`), RES-006 (golden framework), RES-008 (`string+`), RES-011 (bare return), RES-009/010 (no parser/lexer panics) |
| **G3** | Drop dummy parameters | ✅ RES-004 |
| **G4** | Diagnostics — phase 1 done, phase 2 open | 🟡 graceful errors landed (RES-009, RES-010); source spans still RES-005 |
| **G5** | Proper lexer via `logos` | ⏳ pending |

### Language sanity
| # | Goalpost | Status |
|---|---|---|
| **G6** | AST hardening (one canonical AST, resolve parser.rs fate) | ⏳ |
| **G7** | Real type checker (inference, unification, exhaustiveness) | ⏳ |
| **G8** | Function contracts (`requires` / `ensures`) at runtime | ⏳ |

### Verifiability
| # | Goalpost | Status |
|---|---|---|
| **G9** | Symbolic assert (Z3 or custom bounded verifier for int domain) | ⏳ |
| **G10** | Live-block invariants re-checked on every retry | ⏳ |

### Post-G10 — extended ladder
(Added 2026-04-16 once G1–G3 landed. These move the ultimate goalpost
closer to "real production language for embedded safety-critical work.")

| # | Goalpost | Success criterion |
|---|---|---|
| **G11** | Stdlib primitives | `print`, `read_line`, math (`abs`, `min`, `max`, `sqrt`), collections stubs — all registered as builtins, all tested. |
| **G12** | Structs / records | User-defined product types with named fields; field access `.`; lowered in typechecker; round-tripped in tests. |
| **G13** | Pattern matching | `match value { pattern => expr, ... }` with literal, identifier, wildcard patterns. Exhaustiveness checked at compile time. |
| **G14** | Static type errors are language-level | Typechecker is *the* source of truth — type mismatches rejected at compile time, not at interpreter runtime. |
| **G15** | Cranelift backend | Emit machine code for a restricted core; `cargo run -- --compile hello.rs` writes `hello.o`. |
| **G16** | `no_std` / Cortex-M embedded target | Runtime strippable so a minimal Resilient program can link and boot on a QEMU-hosted Cortex-M. |
| **G17** | Language Server Protocol | Basic LSP over stdin/stdout; provides diagnostics and hover types. |
| **G18** | Effect tracking | Function signatures declare I/O, non-determinism, and "can panic" effects; typechecker enforces them transitively. |
| **G19** | Proof-carrying assertions | Export the SMT queries generated in G9 as a verification certificate; re-check them with an independent solver. |
| **G20** | Self-hosting | Write a Resilient program that exercises enough of the language to be plausibly checked by its own verifier. |

## Moving the post

When the Executor lands a ticket that closes a goalpost, the Manager
updates the status cell and adds a changelog entry. When all visible
goalposts show ✅ or 🟡, the Manager drafts more goalposts at the
bottom — the ladder grows indefinitely.

## Changelog

- 2026-04-16 — ladder seeded.
- 2026-04-16 — G1 landed (RES-001, RES-007 clippy followup).
- 2026-04-16 — G2 landed (RES-002 harness, RES-003 println, RES-006
  golden tests, RES-008 string+primitive coercion — first two examples
  run end-to-end and are pinned by golden files).
- 2026-04-16 — G3 landed (RES-004 dropped dummy-param requirement; 5
  examples rewritten, docs updated).
- 2026-04-16 — ladder extended with G11–G20 post-initial-foundation.
- 2026-04-16 — ralph-loop launcher parked (see `.board/LOOP_STATUS.md`);
  this session is acting as both Manager and Executor.
- 2026-04-16 — RES-008 (string + primitive coercion) landed — `minimal.rs`
  now runs end-to-end; golden tests for hello/minimal pinned.
- 2026-04-16 — RES-006 (golden-test framework) landed. `tests/examples_golden.rs`
  walks examples/ and diffs against .expected.txt sidecars.
- 2026-04-16 — RES-007 (clippy clean) landed. `cargo clippy -- -D warnings` exits 0.
- 2026-04-16 — RES-011 (bare return) landed.
- 2026-04-16 — RES-009 + RES-010 landed: no parser/lexer panic can crash
  the binary. Unknown characters and unrecognized syntax now produce red
  "Parser error" diagnostics and let the driver exit cleanly.
