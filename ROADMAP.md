# Resilient Roadmap

North star: turn Resilient from an MVP interpreter into a serious
formally-verifiable language for safety-critical embedded systems.

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
| **G1** | Green build | ✅ RES-001, RES-007 |
| **G2** | Test harness | ✅ 46 unit + 1 golden + 4 smoke. RES-002 / 003 / 006 / 008 / 011 / 009 / 010 / 016 all landed. |
| **G3** | Drop dummy parameters | ✅ RES-004 |
| **G4** | Diagnostics | ✅ RES-005 (line:col), RES-009 + RES-010 + RES-016 (no parser/lexer panics), RES-028 (assert shows operands) |
| **G5** | Proper lexer via `logos` | ⏳ pending — current lexer is hand-rolled but solid |

### Language sanity
| # | Goalpost | Status |
|---|---|---|
| **G6** | AST hardening (one canonical AST, resolve parser.rs fate) | ✅ RES-070 deleted dead `parser.rs`; RES-069/077/078/079/084/085/086/087/088 landed spans on **every** `Node` variant; RES-080 surfaces them in typechecker diagnostics. |
| **G7** | Real type checker (inference, unification, exhaustiveness) | ⏳ |
| **G8** | Function contracts (`requires` / `ensures`) at runtime | ✅ RES-035 |

### Verifiability
| # | Goalpost | Status |
|---|---|---|
| **G9** | Symbolic assert (Z3 or custom bounded verifier for int domain) | ✅ RES-067 wired Z3; RES-068 elides runtime checks for fully-proven fns. |
| **G10** | Live-block invariants re-checked on every retry | ✅ RES-036 |

### Stdlib / ergonomics / ecosystem
| # | Goalpost | Status |
|---|---|---|
| **G11** | Stdlib primitives | 🟡 13 builtins. RES-055 made `floor`/`ceil`/`pow` type-preserving (Int→Int when lossless). Next: file I/O, input, string utilities. |
| **G12** | Arrays + structs / records | 🟡 Arrays + RES-034 nested index assignment (`a[i][j] = v`) at any depth. Structs pending. |
| **G13** | Pattern matching (`match`) | ✅ RES-039 (closed earlier; carry-over) |
| **G14** | Static type errors at compile time | 🟡 RES-053/054 partial; full inference pending |
| **G15** | Cranelift backend / modules / VM | 🟡 RES-073 landed `use "path";` modules. Cranelift (RES-072) and bytecode VM (RES-076) still open. |
| **G16** | `no_std` / Cortex-M embedded target | ⏳ — RES-075 ticket open |
| **G17** | Language Server Protocol | ⏳ — RES-074 ticket open, blocked on RES-069 |
| **G18** | Effect tracking | ✅ RES-191 (`@pure`), RES-192 (`@io` inference), RES-389 (declared effects), RES-385c (linear×effects). Actor concurrency design landed (RES-208, RES-332/333). |
| **G19** | Proof-carrying assertions | ✅ RES-071 (`--emit-certificate`), RES-194 (Ed25519 signatures), RES-195 (`verify-all` + manifest), RES-331 (schema v1 doc). Bundle is round-trippable end-to-end. |
| **G20** | Self-hosting | ⏳ Blocked on RES-323 (lexer in Resilient) → RES-379 (parser in Resilient). |
| **G21** | FFI v1 (tree-walker + static registry) | ✅ Shipped 2026-04-19. RES-383 security audit landed 2026-04-29. |

### New between G4 and G5 (core-language improvements landed in session 2)

Not assigned their own goalpost but worth listing, since each is a
language-level feature a user would see:

- **Assignment**: `x = expr` (RES-017)
- **Forward references**: caller/callee order doesn't matter (RES-018)
- **Modulo operator** `%` (RES-015)
- **Prefix operators** `!` and `-` (RES-012)
- **Logical operators** `&&` / `||` (RES-021)
- **Bitwise operators** `& | ^ << >>` (RES-029)
- **String comparisons** `< > <= >=` + `len()` builtin (RES-022)
- **While loops** with runaway guard (RES-023)
- **Block comments** `/* */` (RES-024)
- **Hex/binary integer literals** with `_` separators (RES-025)
- **`static let`** persistent bindings across calls (RES-013)
- **Bare `return;`** (RES-011)
- **Pratt-parser invariant fix** (RES-014)
- **Non-zero exit on error** (RES-027)

## Moving the post

When a ticket closes a goalpost, update the status cell above and add a
changelog entry below.

## Changelog

- 2026-04-16 — ladder seeded.
- 2026-04-16 — G1 landed (RES-001, RES-007 clippy followup).
- 2026-04-16 — G2 landed (RES-002 harness, RES-003 println, RES-006
  golden tests, RES-008 string+primitive coercion).
- 2026-04-16 — G3 landed (RES-004 dropped dummy-param requirement).
- 2026-04-16 — ladder extended with G11–G20 post-initial-foundation.
- 2026-04-16 — RES-008 (string + primitive coercion) landed.
- 2026-04-16 — RES-006 (golden-test framework) landed.
- 2026-04-16 — RES-007 (clippy clean) landed.
- 2026-04-16 — RES-011 (bare return) landed.
- 2026-04-16 — RES-009 + RES-010 landed: no parser/lexer panic can crash the binary.
- 2026-04-16 — session 2: 15 more tickets landed (RES-012 through RES-030).
  G4 fully closed, G11 kicked off.
- 2026-04-16 — session 3: arrays (RES-032), push/pop/slice (RES-033),
  for..in (RES-037), function contracts (RES-035 closes G8), live-block
  invariants (RES-036 closes G10).
- 2026-04-17 — session 4 (28 tickets): G6 fully closed ✅, G15 bytecode VM
  end-to-end 🟡, G17 LSP scaffolding + 3 integration tests 🟡.
- 2026-04-17 — G15 JIT real expression + control-flow (Phases B–E via RES-096/099/100/102).
  G18 no_std embedded toolchain proven end-to-end ✅ (RES-075/097/098).
- 2026-04-19 — G21 FFI v1 shipped (tree-walker + static registry, 748 tests pass).
- 2026-04-20 — Migrated ticket tracking from `.board/` to GitHub Issues.
- 2026-04-29 — G18 closed: linear × effect interaction (RES-385c) and concurrency design (RES-208) landed.
  G19 closed: certificate manifest schema v1 (RES-331) plus end-to-end signed `verify-all`.
  RES-383 FFI v1 security audit signed off (no CVEs).
  RES-392b per-prefix `recovers_to` BMC scaffolding.
