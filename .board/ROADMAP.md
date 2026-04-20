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
| **G6** | AST hardening (one canonical AST, resolve parser.rs fate) | ✅ RES-070 deleted dead `parser.rs`; RES-069/077/078/079/084/085/086/087/088 landed spans on **every** `Node` variant (Program, leaves, statements, expressions, index/field ops, tuple variants converted to struct, plus structural variants Function/Use/LiveBlock/Assert/Match/StructDecl/StructLiteral/FunctionLiteral); RES-080 surfaces them in typechecker diagnostics. Future work tracked separately: surfacing more spans in interpreter / VM runtime errors, parser-error position threading. |
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
| **G21** | FFI v1 (tree-walker + static registry) | ✅ Shipped 2026-04-19 |

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
- 2026-04-17 — session 4 continued (ralph-loop, iterations 8-27): 21
  more tickets shipped under the orchestrator/executor pattern. Major
  shifts:
  * **G6 fully closed** ✅ via the RES-077..088 series. Every `Node`
    variant now carries `span: Span`. Migration covered Program
    statements (RES-077), leaves (RES-078), core statements
    (RES-079), core expressions (RES-084), index/field ops
    (RES-085), tuple→struct conversions for ArrayLiteral +
    TryExpression (RES-086) and ExpressionStatement + Block
    (RES-087), and structural variants Function/Use/LiveBlock/
    Assert/Match/StructDecl/StructLiteral/FunctionLiteral
    (RES-088). RES-080 surfaces statement spans in typechecker
    diagnostics: errors now print `<file>:<line>:<col>:` prefix.
  * **G15 — bytecode VM end-to-end** 🟡 fully built out. RES-076
    foundation (Op enum + Chunk + compiler + stack VM for int
    arith + let), RES-081 function calls + recursion + call frames,
    RES-083 control flow (Jump/JumpIfFalse, comparison ops, &&/||
    short-circuit). RES-082 bench: bytecode VM runs fib(25) in
    30.8 ms vs 396.8 ms tree walker — **12.9× speedup**, beating
    Python 3 / Node.js / Ruby on the same workload. RES-091 +
    RES-092 thread `chunk.line_info` so VM runtime errors print
    `(line N)` at the actual offending source line.
  * **G17 — LSP scaffolding + 3 integration tests** 🟡. RES-074
    landed tower-lsp + tokio under an opt-in `lsp` feature flag,
    with a `--lsp` driver flag, `Backend::initialize` returning
    capabilities, and `did_open`/`did_change` publishing
    diagnostics with proper LSP ranges. RES-089 routed parser
    errors through the same range extractor (no more 0:0
    diagnostics). RES-090/093/094 are end-to-end integration
    tests that spawn the binary and exercise the full LSP
    protocol: handshake → didOpen → didChange edit/revert flow.
    Hand-rolled LSP framing helpers — no extra deps beyond
    tower-lsp itself.
  * **Smaller wins**: RES-026 REPL `--examples-dir`; RES-034
    nested index assignment `a[i][j]...[k] = v`; RES-055
    type-preserving `floor`/`ceil`/`pow`.
  * **Test growth across the session**: 145 unit / 4 smoke / 1
    golden → 217 unit / 11 smoke / 1 golden default; 225/12/1
    with `--features z3`; 221/14/1 with `--features lsp`
    (including 3 lsp_smoke tests). All three `cargo clippy
    -- -D warnings` clean. Net 28 tickets shipped this session
    (RES-026, 034, 055, 069..074, 076..088, 089..094 — see
    `.board/tickets/DONE/` for the ledger).
  * Remaining OPEN: RES-072 (Cranelift JIT, new deps) and
    RES-075 (no_std embedded target, new deps). Both are
    multi-iteration efforts deferred until needed.
- 2026-04-17 — session continued (iterations 28-39): both Phase 5
  pillars (JIT + no_std) opened up substantively.
  * **G15 — JIT real expression + control-flow surface** 🟡.
    Phases B–E shipped under RES-072: RES-096 (IntegerLiteral +
    Add), RES-099 (Sub/Mul/Div/Mod via isub/imul/sdiv/srem),
    RES-100 (six comparison ops via icmp + uextend, plus
    BooleanLiteral), RES-102 (if/else with brif into two
    cranelift blocks). The JIT now compiles programs like
    `if (5 + 5 == 10) { return 1; } else { return 0; }` to
    native code via parse → lower → cranelift → JITModule →
    raw fn pointer transmute. 29 jit_backend unit tests + 4
    end-to-end smoke tests (gated `--features jit`). Phase E
    enforces both arms of every if must return — Phase F lifts
    that with a merge_block + phi.
  * **G18 — no_std embedded toolchain proven end-to-end** ✅.
    RES-075 (Phase A: alloc-free `Value::Int`/`Bool` in
    `resilient-runtime/`), RES-097 (verified cross-compile to
    `thumbv7em-none-eabihf`), RES-098 (opt-in `alloc` feature
    pulls in embedded-alloc 0.5 — `Value::Float` always
    available, `Value::String` gated). Cortex-M4F builds clean
    in both feature configs; clippy passes both. 11 unit tests
    default, 14 with `--features alloc`.
  * Test growth: default 217 (unchanged), z3 225, lsp 221, jit
    245 (+29 jit unit tests + 4 jit smoke tests). resilient-runtime
    sibling crate: 11 default / 14 alloc. All clippy clean.
  * Remaining OPEN at iter 39: RES-101 (cortex-m demo crate
    that links resilient-runtime + wires LlffHeap — needs the
    manager pass to flesh out acceptance criteria first).
- 2026-04-19 — FFI Phase 1 shipped on `ffi-phase-1-tree-walker`:
  * **G21 — FFI v1 (tree-walker + static registry)** ✅.
    `extern "lib" { fn ... }` blocks parsed, type-checked, and
    resolved via `libloading` at program load. Tree-walker dispatches
    through a hand-rolled C-ABI trampoline table (arity 0–8, primitives
    only). `requires`/`ensures` contracts evaluated at FFI call sites;
    `@trusted` propagates ensures as Z3 axioms. `resilient-runtime`
    gains a zero-alloc `StaticRegistry` behind `ffi-static` for
    Cortex-M / RISC-V targets. Example `ffi_libm.rs` + SYNTAX and
    docs pages. End-to-end integration tests against a bundled C
    helper lib. All 748 tests pass (`--features ffi`).
