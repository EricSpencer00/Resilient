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
| **G6** | AST hardening (one canonical AST, resolve parser.rs fate) | 🟡 RES-070 deleted dead `parser.rs`; RES-069/077/078/079/084/085 landed spans on Program + leaves + core statements + core expressions + index/field ops; RES-080 surfaces them in typechecker diagnostics. Remaining: tuple variants (`ArrayLiteral`, `TryExpression`, `Block`, `ExpressionStatement` — need tuple→struct conversion) and structural variants (`Match`, `StructLiteral`, `FunctionLiteral`, `Function`, `LiveBlock`, `Assert`, `StructDecl`) — tracked as future follow-ups. |
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
| **G18** | Effect tracking | ⏳ |
| **G19** | Proof-carrying assertions | 🟡 RES-071 landed `--emit-certificate`: SMT-LIB2 dumps re-verifiable by stock Z3. Full PCA semantics (signed certs, manifest) still ahead. |
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
- 2026-04-17 — session 4 (ralph-loop, 7+ iterations): seven tickets
  shipped under the orchestrator/executor pattern. **G6 partial** —
  RES-069 landed the `span.rs` foundation (Pos/Span/Spanned + a lexer
  helper that pairs each token with its source range), RES-070 deleted
  the 817-line dead `parser.rs` parallel parser. **G15 partial** —
  RES-073 added `use "path.rs";` module imports with recursive
  resolution, cycle detection via canonicalized-path HashSet, and
  splice-into-importer semantics. **G19 partial** — RES-071 landed
  `--emit-certificate <DIR>`: every Z3-discharged contract obligation
  now dumps a self-contained SMT-LIB2 file that stock Z3 confirms as
  `unsat` (proof confirmed via manual round-trip). **G11 polish** —
  RES-055 made `floor`/`ceil`/`pow` type-preserving with
  checked-arithmetic overflow guards. **G12 polish** — RES-034 lifted
  the single-index restriction so `a[i][j]...[k] = v` works at any
  depth, with `at dim {N}` bounds messages. **Ergonomics** — RES-026
  added `--examples-dir <DIR>` so the REPL's `examples` command can
  list real files. The remaining big-ticket items (Cranelift, LSP,
  no_std, bytecode VM) all require significant new dependencies and
  carry their own multi-iteration tickets (RES-072/074/075/076). RES-069
  itself is split into RES-077..080 for iteration-sized AST migration
  work. 165 unit + 1 golden + 6 smoke = 172 tests default, 173+1+7
  with `--features z3`. Clippy clean both ways.
