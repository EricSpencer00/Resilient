# Major Language Features Status

A roadmap of what's implemented, partially implemented, and genuinely missing in Resilient.
This was originally a "10 missing features" list; subsequent audits found most were already
in the compiler. The 50-feature pass (PR #1076) then landed the remaining items as first-slice
modules, so nearly everything below is now implemented.

## Already Implemented (pre-PR-1076)

| Feature | Ticket | Status |
|--------|--------|--------|
| `?` operator (Result/Option propagation) | RES-086, RES-375 | Full — in `Node::TryExpression` eval |
| String interpolation `"x = {x}"` | RES-221 | Full — `string_interp.rs` module |
| Anonymous fn literals `fn(int x) -> int { ... }` | RES-403 | Full — `Node::FunctionLiteral` |
| Function-type annotations `fn(int) -> int` | RES-403 | Full — parser accepts in param position |
| Higher-order functions (`map`, `filter`, `reduce`) | RES-927 | Full — method-call form on arrays |
| Generic type parameters `fn id<T>(x: T) -> T` | RES-405 PR1-4 | Full with monomorphization |
| Variadic FFI (`extern fn printf(string, ...)`) | RES-316 | Full for FFI bindings |
| Pipe operator `\|>` | RES-926 | Full |
| `as` cast operator | RES-934 | Full |
| Closure captures (immutable) | RES-164 | Full |
| Tuple structs and tuple destructuring | RES-928, RES-933 | Full |
| `if let` / `while let` / `else if let` | RES-908, RES-914, RES-930 | Full |
| Range patterns / array slicing | RES-915, RES-911 | Full |
| Compound assignment `+=` etc. | RES-912, RES-917 | Full |
| Operator overloading via traits | RES-929 | Full at runtime |

## Landed in PR #1076 — 50-Feature Pass

All 51 modules below follow the feature-isolation pattern (`src/<name>.rs` + a single
`crate::<name>::check()` call in the `<EXTENSION_PASSES>` block of `typechecker.rs`).
Each is a **first slice**: attribute parsing, registry, static analysis, tests. Follow-up
PRs can deepen each one (full lowering, Z3 integration, runtime semantics).

### Vibe-Coded Resilience — the flagship cluster

The core answer to "I vibe-coded this app, how do I know it doesn't break?"

| Module | What it does |
|--------|-------------|
| `resilience_score` | Grades every function A–F across five buckets: contracts, effects, liveness, coverage, simplicity |
| `vibe_debt` | Measures the gap between "what the programmer asserted" and "what the compiler could assert" |
| `behavioral_fingerprint` | SipHash over contract signatures; detects silent behavioral drift between commits |
| `contract_inference` | Infers `requires`/`ensures` from the function body (division-by-zero, index bounds, single-return) |
| `semantic_regression` | Diffs contract counts and `fails` variants; flags any weakening as a hard error |
| `semver_behavior` | Classifies a diff as MAJOR/MINOR/PATCH based on behavioral fingerprint + semantic regression |
| `blame_attribution` | Builds a callee→caller blame map; surfaces which caller introduced the fault |
| `autopilot` | Orchestrates vibe_debt + resilience_score + contract_inference into a single `AutopilotReport` |
| `crash_only_cert` | Certifies that `#[crash_only_cert]` functions only return `Ok`/`Err`/`result` |
| `intent_blocks` | `#[intent(property="...", enforced_by="fn1 fn2")]` — warns if the enforcer fn is absent |
| `anti_regression` | `#[stable(since="1.0", behavior="<digest>")]` — hard error if fingerprint drifts from locked value |

### Type System Innovations

| Module | What it does |
|--------|-------------|
| `refinement_types` | `#[refinement(base="int", where="self > 0")]` — predicate types with runtime guards |
| `typestate_types` | Typestate machines: `#[typestate]` structs with transition table enforcement |
| `dependent_arrays` | Array length encoded in the type; bounds violations caught at compile time |
| `row_polymorphism` | Open-record types that accept any struct with at least the specified fields |
| `info_flow` | Taint tracking: `#[tainted]` values cannot flow into `#[untainted]` sinks |
| `phantom_types` | Zero-cost unit-tag types to prevent unit confusion (metres vs. feet) |
| `recursive_types` | Self-referential struct detection and `Box`-equivalent indirection advice |

### Verification

| Module | What it does |
|--------|-------------|
| `deadlock_freedom` | Lock-acquisition order graph; cycles are hard errors |
| `session_types` | Protocol state machines: send/recv sequences verified statically |
| `probabilistic_contracts` | `requires p >= 0.0 && p <= 1.0` on probability-returning functions |
| `wcet_contracts` | `#[wcet(cycles=N)]` — worst-case execution time budget enforcement |
| `distributed_invariants` | Cross-node invariants for actor-message protocols |
| `ghost_types` | Proof-only types erased at codegen; Z3 can reason over them |
| `incremental_verify` | SHA-256 cache: re-verify only changed functions, skip unchanged |
| `property_tests` | `#[property_test]` generates 100 random inputs and checks the postcondition |

### Embedded & Hardware

| Module | What it does |
|--------|-------------|
| `mmio_regmap` | `#[mmio_regmap]` declares peripheral register maps; overlap detection |
| `power_contracts` | `#[power(budget_uj=N)]` energy budget enforcement with a per-statement model |
| `stack_contracts` | `#[max_stack(bytes=N)]` — static stack depth estimator |
| `no_alloc_cert` | Certifies `#[no_alloc]` functions contain no heap-allocating operations |
| `hw_state_machine` | Hardware state machine protocol (INIT→READY→RUNNING→FAULT) with transition locks |

### Concurrency

| Module | What it does |
|--------|-------------|
| `async_await` | `async fn` / `await` syntax scaffold; state-machine transform and scheduler hook |
| `atomic_types` | `AtomicI64` registry with `SeqCst` fetch_add / load / store |
| `lock_priority` | Lock priority inversion detection: lower-priority tasks must not hold higher-priority locks |

### Type System Completions

| Module | What it does |
|--------|-------------|
| `default_trait_methods` | Default method bodies in trait declarations |
| `associated_constants` | `const MIN: int` / `const MAX: int` on traits and impls |
| `derives` | `#[derive(Debug, Eq, Hash, Clone, Ord)]` struct auto-impls |
| `const_fn` | `const fn` — full compile-time evaluator for int/bool expressions |
| `macros` | `macro_rules!`-style hygienic macros (first-slice: parse + expand) |

### Modules & Ecosystem

| Module | What it does |
|--------|-------------|
| `full_modules` | `mod` / `use` graph with visibility (`pub`, `pub(crate)`, private) |
| `package_manager` | `rz.toml` manifest parser + semver `^`/`~`/exact dependency resolution |
| `iterator_protocol` | `Iterator` trait with `next()`, `map()`, `filter()`, `collect()` |

### Developer Experience

| Module | What it does |
|--------|-------------|
| `mutation_testing` | Simulates single-operator mutations; warns if no test detects the mutation |
| `causal_trace` | Causal event log with happens-before ordering; bounded ring buffer |
| `snapshot_regression` | `.snap` golden file comparison; hard error on unexpected output change |
| `coverage_warnings` | `#[coverage_required]` — warns if a function has no test exercising it |

### Ergonomics

| Module | What it does |
|--------|-------------|
| `param_destructuring` | Destructuring in function parameters: `fn rotate((int x, int y))` |
| `format_builtin` | `format("{:.2f}", value)` — printf-style format spec evaluation |
| `struct_exhaustiveness` | Match-arm exhaustiveness for struct patterns (not just enum variants) |
| `labeled_break` | Detects deep nested loops (depth ≥ 3) and recommends labeled break/continue |
| `fmt_validation` | Counts `{}` placeholders in `format()` calls; hard error on arity mismatch |
| `no_panic_cert` | `#[no_panic]` — certifies absence of `unwrap`/`expect`/`panic` in a function |

## Landed Post-PR-1076 — League-of-Its-Own Pass

| Module | What it does |
|--------|-------------|
| `ai_threat_model` | Formal threat model for LLM failure modes — 10 detection passes, `--ai-threats` CLI, `#[ai_review_required]` hard gate |
| `lean_spec` | Lean 4 operational semantics + per-function theorem emitter — `--emit-lean-spec=FN`, `lean-spec/` Lake project with proven `eval_int_lit_id`, `eval_add_comm`, `eval_const_fold_sound`, `eval_neg_involutive` |

## Partially Implemented

| Feature | Ticket | Status |
|--------|--------|--------|
| Sum type / enum payloads | RES-400 PR1 | Parser scaffold for payload-less variants only; PR2-5 (payloads, matching, exhaustiveness, eval) remain |
| Mutable closure capture | RES-328 | In progress — cell-based shared mutation works; auto-capture sugar deferred |

## Genuinely Missing (post-PR-1076)

Each 50-feature-pass module is a **first slice**. Items below represent the follow-up work
needed to bring each slice to production depth:

- **Macros**: expansion engine + hygiene rules (PR #1076 has parser scaffold only)
- **Async/Await**: state-machine lowering + scheduler integration (PR #1076 has syntax scaffold)
- **Full module system**: circular-import detection, re-export graph (partial in `full_modules`)
- **Cranelift / LLVM codegen for new nodes** (all 50 features interpret-only today)
- **Z3 integration for refinement types** (currently runtime-only predicate checks)

---

## Footprint Reality Check

Resilient at ~1.8 MB of `lib.rs` is a much more complete language than the original
"10 missing features" audit suggested. PR #1076 landed 51 new modules covering everything
that remained. The work ahead is deepening each first slice, not widening the surface.
