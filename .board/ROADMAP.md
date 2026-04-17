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
| **G1** | Green build | ✅ RES-001, RES-007 |
| **G2** | Test harness | ✅ 46 unit + 1 golden + 4 smoke. RES-002 / 003 / 006 / 008 / 011 / 009 / 010 / 016 all landed. |
| **G3** | Drop dummy parameters | ✅ RES-004 |
| **G4** | Diagnostics | ✅ RES-005 (line:col), RES-009 + RES-010 + RES-016 (no parser/lexer panics), RES-028 (assert shows operands) |
| **G5** | Proper lexer via `logos` | ⏳ pending — current lexer is hand-rolled but solid |

### Language sanity
| # | Goalpost | Status |
|---|---|---|
| **G6** | AST hardening (one canonical AST, resolve parser.rs fate) | ⏳ |
| **G7** | Real type checker (inference, unification, exhaustiveness) | ⏳ |
| **G8** | Function contracts (`requires` / `ensures`) at runtime | ✅ RES-035 |

### Verifiability
| # | Goalpost | Status |
|---|---|---|
| **G9** | Symbolic assert (Z3 or custom bounded verifier for int domain) | ⏳ — **NEXT strategic milestone** |
| **G10** | Live-block invariants re-checked on every retry | ✅ RES-036 |

### Stdlib / ergonomics / ecosystem
| # | Goalpost | Status |
|---|---|---|
| **G11** | Stdlib primitives | 🟡 13 builtins: println, print, abs, min, max, sqrt, pow, floor, ceil, len, push, pop, slice. Next: file I/O, input, string utilities. |
| **G12** | Arrays + structs / records | 🟡 Arrays landed (RES-032, RES-033, RES-037). Structs pending. |
| **G13** | Pattern matching (`match`) | ⏳ |
| **G14** | Static type errors at compile time | ⏳ |
| **G15** | Cranelift backend | ⏳ |
| **G16** | `no_std` / Cortex-M embedded target | ⏳ |
| **G17** | Language Server Protocol | ⏳ |
| **G18** | Effect tracking | ⏳ |
| **G19** | Proof-carrying assertions | ⏳ |
| **G20** | Self-hosting | ⏳ |

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
- 2026-04-16 — session 2: 15 more tickets landed (RES-012 through RES-030).
  G4 fully closed, G11 kicked off. Every parser/lexer panic eliminated
  (RES-016). Full operator suite — prefix, arithmetic, comparison, logical,
  bitwise, shifts. `while` loops. `static let`. Assignment. Forward
  references. Block comments. Hex/binary literals. Non-zero exit on error.
  46 unit + 1 golden + 4 smoke tests, clippy clean. Docs synced.
- 2026-04-16 — session 3: arrays (RES-032), push/pop/slice (RES-033),
  for..in (RES-037), function contracts (RES-035 closes G8), live-block
  invariants (RES-036 closes G10). The language now has: composite data,
  ergonomic iteration, and — critically — both function-level and
  block-level correctness conditions. That's the foundation the G9
  SMT layer will run on. Also: GitHub Actions CI is wired up.
  69 unit + 1 golden + 4 smoke, clippy clean.
