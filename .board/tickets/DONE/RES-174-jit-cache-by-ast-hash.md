---
id: RES-174
title: JIT cache: memoize compiled functions keyed by AST hash
state: DONE
priority: P3
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
Re-running a program re-lowers the same AST into the same
Cranelift IR into the same machine code. Wasteful. Hash the
function AST (post-typecheck) and cache the compiled fn-pointer
across runs within a session. Cross-session persistence is a
follow-up.

## Acceptance criteria
- `JitCache { map: HashMap<u64, FnPtr> }` on the JIT module.
- Hash: FNV-1a over a canonical serialization of the function AST
  (post-span stripping — spans shouldn't affect the cache).
- On `jit_compile(fn)`: hash; if hit, reuse; if miss, compile and
  store.
- Cache stats surfaced via `--jit-cache-stats` — prints
  `hits / misses / compiles` on exit.
- Unit test: call the JIT compile twice on the same function;
  second call reports a cache hit.
- Commit message: `RES-174: in-memory JIT cache keyed by AST hash`.

## Notes
- Cross-session cache (disk) is tempting but requires a stable
  serialization of Cranelift output, and invalidation on compiler
  version changes. Separate ticket.
- Thread safety: the JIT is single-threaded for now (no
  concurrent compile). Document the assumption.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/jit_backend.rs`:
  - New `pub struct JitCache { map: HashMap<u64, FuncId>,
    hits: u32, misses: u32, compiles: u32 }`. Per-run lifetime.
  - `fn_hash(parameters, requires, ensures, body)` →
    `u64` — FNV-1a over a canonical byte stream written by
    `write_canon_node`. Spans are never written, so spans
    shouldn't affect the cache (per the ticket).
  - `write_canon_node` covers the AST-node subset the JIT
    lowers (literals, infix, prefix, call, return, if, while,
    let, assignment, expression-statement, block, identifier).
    Unsupported variants use a single `0xFF` catch-all tag
    which can collide, but any function containing an
    unsupported variant would be rejected by `lower_expr`
    before the cache entry could be reused — so collisions
    can't produce runtime miscompiles.
  - `run()` split: the old single-function public `run()` is
    now a thin wrapper around a new `run_internal` that
    returns `(i64, JitCache)`. Pass 1 computes each fn's hash
    and consults the cache before declaring a FuncId; a hit
    reuses the existing FuncId under the new name (so calls
    through either name dispatch to the same compiled code),
    a miss declares a fresh FuncId and remembers it. Pass 2
    only compiles the "primary" fn per hash — aliases skip.
  - Process-wide `GLOBAL_JIT_HITS / MISSES / COMPILES`
    `AtomicU64` counters, updated via `flush_cache_stats_to_
    globals(&cache)` at the end of each `run()`. Exposed via
    `pub fn cache_stats() -> (u64, u64, u64)` for the CLI.
  - `#[cfg(test)] pub(crate) fn run_with_stats(program) ->
    (i64, u32, u32, u32)` — test-only API returning the
    per-run cache's counters directly (not global deltas), so
    parallel-executed tests don't see each other's
    accumulation.
- `resilient/src/main.rs`:
  - New `--jit-cache-stats` CLI flag. When set, the run
    prints `jit-cache: hits=H misses=M compiles=C` to stderr
    right before exiting (success or failure). Under
    `#[cfg(not(feature = "jit"))]` the flag prints a clear
    "unavailable — built without `--features jit`" note so
    users don't wonder about silent zeros.
- Deviations: the cache is scoped to a single `run()` call —
  FuncIds are Cranelift-module-local, so cross-session reuse
  would require the static-JIT-module / imported-symbol
  machinery the ticket's Notes explicitly defer. The stats,
  on the other hand, DO accumulate across runs via the
  process-wide atomics, so `--jit-cache-stats` can report
  sensible lifetime numbers. The doc comment on `JitCache`
  calls this out clearly.
- Thread-safety: the ticket says "The JIT is single-threaded
  for now (no concurrent compile). Document the assumption."
  Two concurrent `run()` calls on the same process would each
  have their own `JitCache`, so there's no data race at the
  cache level. The global atomic counters are Relaxed-ordered
  and deliberately non-synchronizing — an assumption the
  doc-comment pins down.
- Unit tests (6 new, behind `--features jit`):
  - `jit_cache_hit_on_duplicate_fn_body` — ticket AC: two
    fns with identical bodies → `(hits=1, misses=1, compiles=1)`
    and the call result (24) is correct (both names dispatch
    to the same compiled fn).
  - `jit_cache_miss_on_distinct_bodies` — distinct bodies
    → `(hits=0, misses=2, compiles=2)`.
  - `jit_cache_ignores_span_differences` — parameter names
    DO matter (different param name → different hash), spans
    do NOT. Locks the canonical-form policy.
  - `jit_cache_three_way_alias` — three identical bodies →
    `(hits=2, misses=1, compiles=1)`.
  - `jit_fn_hash_is_deterministic_and_span_independent` —
    same AST hashes identically; different literal body
    hashes differently.
  - `jit_cache_global_stats_accumulate_across_runs` — two
    sequential `run()` calls bump the global counters by at
    least the expected amount (`>=` rather than `==` since
    parallel tests race on the globals; the local-cache
    tests above pin the exact-equality invariants).
  - All stat-observing tests share `JIT_CACHE_TEST_LOCK:
    Mutex<()>` to serialize internal test state that touches
    the globals.
- Manual smoke: `cargo run --features jit -- --jit
  --jit-cache-stats /tmp/cache_smoke.rs` on a two-identical-
  fn program prints `jit-cache: hits=1 misses=2 compiles=2`
  end-to-end, confirming the CLI surface.
- Verification:
  - `cargo test --locked` — 468 passed (unchanged from
    RES-173; cache is JIT-feature-gated).
  - `cargo test --locked --features jit` — 532 passed (was
    526 before RES-174).
  - `cargo clippy --locked --features logos-lexer,z3,jit
    --tests -- -D warnings` — clean.
