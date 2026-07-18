# RES-4134: JIT startup latency, memory, and native-lowering coverage

Follow-up to #4111 (B-E4 JIT completeness) and #4134's remaining scope
item 3. This directory measures two things the existing `benchmarks/fib`
and `benchmarks/jit` micro-benchmarks don't:

1. **Fixed startup cost** of standing up a Cranelift module + running a
   do-nothing program, contrasted with the walker and VM's own fixed
   cost.
2. **How much of the example corpus actually executes through native
   JIT lowering today**, versus RES-4019's transparent VM fallback.

## Startup latency + peak RSS

Workload: `trivial.rz` — a single top-level `return 0;`. No calls, no
loops, no literals worth executing; wall time is (almost) entirely
per-invocation fixed cost, not workload time.

Run `./benchmarks/jit_startup/run.sh` (hyperfine, 20 runs / 5 warmup,
`--shell=none`; peak RSS from one warm `/usr/bin/time -l`/`-v` run per
backend).

Machine: Darwin arm64 (Apple M-series), 2026-07-18.

| Backend | Median wall time | Peak RSS |
|---|---:|---:|
| walker | 4.80 ms | 9664 KB |
| VM     | 4.67 ms | 9392 KB |
| JIT    | 4.81 ms | 10400 KB |

Takeaways:

- On a do-nothing program, the JIT's Cranelift module setup adds no
  measurable wall-clock overhead over the walker/VM here — all three
  are within noise of each other at ~4.7-4.8 ms, which is process-
  spawn + arg-parsing cost, not backend-specific work.
- The JIT does carry a real, consistent ~1 MB peak-RSS premium
  (10.4 MB vs ~9.4-9.7 MB) — Cranelift's ISA/module machinery — that
  the other two backends don't pay. Fixed, not scaling with workload
  size, but worth tracking if it grows as native lowering covers more
  constructs.

## Native-vs-fallback coverage

Run `./benchmarks/jit_startup/coverage.sh`. Sweeps every file in
`resilient/examples/*.rz` through `--jit --verbose` and classifies each
by the presence/absence of the `note: --jit fell back to the VM for
...` line RES-4019 emits (see that script's header for exact
detection logic and its caveats).

Current numbers (619 examples swept; 99 error out under `--verbose`'s
implied `--typecheck` for reasons unrelated to the JIT — deliberately
non-type-clean examples — and are excluded from the native/fallback
split):

| | Count | % of runnable (520) |
|---|---:|---:|
| Native (no fallback) | 0 | 0% |
| Fallback to VM | 520 | 100% |

**Still 0% — but for a different, now-diagnosed reason.** The
original 0% headline (previous revision of this doc) blamed missing
builtin-call lowering. That diagnosis was incomplete. This PR
(RES-4134) fixed two real, *verified* gaps and found — but did not
ship — the actual dominant blocker:

1. **String-literal lowering (fixed, shipped).** Every plain `"..."`
   literal parses to `Node::StringInternLiteral` (RES-2612 string
   interning) — not `Node::StringLiteral`, which is what
   `jit_backend.rs`'s `lower_expr` actually had an arm for.
   `Node::StringLiteral` is effectively dead: the parser stopped
   emitting it once interning landed, except via one
   interpolation-fallback path. So any `println("...")` call's
   *string argument* — not the call itself — was the unsupported
   construct, independent of `println`'s call-site handling (which
   already existed and works correctly for i64 args). Added a mirror
   arm for `Node::StringInternLiteral`.
2. **Two new JIT builtins (fixed, shipped): `abs_diff`, `sign`.**
   Neither had a Cranelift shim; calling either hit
   `jit: unsupported: call to unknown function`. Added
   `res_jit_abs_diff`/`res_jit_sign` extern shims following the
   existing `abs`/`min`/`max` pattern (both are total, panic-free i64
   functions, matching the interpreter's `Value::Int` semantics for
   each).
3. **The actual dominant blocker (diagnosed, NOT fixed — see below):
   zero examples in the corpus have a top-level `return`.** Every
   example uses the `fn main() { ... } main();` shape. The JIT's
   `compile_statements` requires an explicit top-level `return` and
   raises `EmptyProgram` (a precompile, safe-fallback error)
   otherwise — a structural gate that applies regardless of how much
   of the body is otherwise natively lowerable, including after fixes
   1 and 2 above.

**Why the top-level-fallthrough gate isn't just flipped to 0.** This
PR *tried* making top-level fallthrough implicitly `return 0` (to
match the walker/VM, which both silently discard a top-level
non-`return` result — RES-3991). Locally this raised native coverage
from 0/520 to real double-digit percentages. But it also made the JIT
*execute* bodies that used to always precompile-fail first, which
surfaced a wide, pre-existing class of value-*display* bugs in
`jit_runtime.rs`'s tagged-value scheme (`TAG_INT`/`TAG_BOOL`/
`TAG_STRING`/`TAG_STRUCT`/...): `Node::BooleanLiteral` lowers to
untagged raw `0`/`1` (RES-100, deliberate — arithmetic/comparison
treat bools as plain ints), so `println`'s tag-based
`jit_value_display` can't tell a bare `false` from the integer `0`
once it reaches a print call; failures on the corpus sweep showed
`true`/`false` printing as `1`/`0`, and separately a `String`-typed
value printing as a raw pointer-sized integer for at least one example
(`type_name_aliases.rz`) whose root cause wasn't fully isolated in
this session. `interpreter_and_jit_agree_on_all_examples` (the
existing differential parity test) caught every one of these — this
PR reverted the top-level-fallthrough change rather than paper over
real output-correctness divergences with denylist entries, since
`UNSUPPORTED_BY_JIT` only skips the *test comparison*, not what `--jit`
actually does at runtime for a real user. See `jit_backend.rs`'s
`compile_statements` for the revert and its rationale comment.

**Follow-up (not filed as a numbered ticket in this session, per
project convention of noting it here since #4134 owns "native
lowering remainder"):** fix the JIT's value-tagging so every literal
kind (starting with `Node::BooleanLiteral`) that can reach a
`println`/`to_string`/generic-equality call site carries a runtime tag
consistent with `jit_runtime.rs`'s scheme, then re-attempt the
top-level-fallthrough fix. That combination is what actually moves
this number off zero for the real corpus — either change alone either
regresses correctness (fallthrough without tag fixes) or doesn't move
the metric (tag fixes without fallthrough, since `EmptyProgram` still
gates every example first).

## Reproducing

```bash
cargo build --release --features jit    # from resilient/
./benchmarks/jit_startup/run.sh          # startup latency + RSS
./benchmarks/jit_startup/coverage.sh     # native-vs-fallback sweep
```

Both scripts reuse (or build) the same release / `--features jit`
binaries the other `benchmarks/*/run.sh` scripts use
(`resilient/target/release/rz`, `resilient/target/release/rz-with-jit`).
Not wired into the `perf-gate` CI job in this PR — see the PR body for
why, and for the follow-up to add regression baselines once the
native-lowering numbers start moving off zero.
