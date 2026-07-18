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

Current numbers (619 examples swept; 96 error out under `--verbose`'s
implied `--typecheck` for reasons unrelated to the JIT — deliberately
non-type-clean examples — and are excluded from the native/fallback
split):

| | Count | % of runnable (523) |
|---|---:|---:|
| Native (no fallback) | 24 | 3.8% |
| Fallback to VM | 499 | 96.2% |

**Off zero (RES-4153).** The previous revision of this doc diagnosed
the dominant blocker as "zero examples have a top-level `return`" and
described an attempted implicit-`return 0` fallthrough fix that had to
be reverted because it surfaced pre-existing value-*display* bugs
(booleans printing as `1`/`0`) and a memory-corruption bug (a string
printing as a raw pointer). RES-4153 fixed both root causes and
re-landed the fallthrough:

1. **Value tagging (`jit_backend.rs`'s `static_kind`/`ValueKind`).** A
   best-effort, AST-level static classifier (`Int`/`Bool`/`Float`/
   `String`/`Unknown`) tracked through locals, function/method return
   types, and struct-literal fields. `println`/`print`/`to_string`
   route through boolean-aware `res_jit_*_bool` shims
   (`jit_runtime.rs`) whenever the argument is statically `Bool`,
   fixing the `1`/`0` display bug without changing how booleans are
   computed (RES-100's untagged-int representation is unchanged, so
   arithmetic/comparisons are unaffected).
2. **String-concat-via-`+` and float-arithmetic-via-`+`/`-`/`*`/`/`/`%`
   dispatch.** `InfixExpression` lowering previously always emitted
   `iadd`/`isub`/... regardless of operand type, corrupting
   heap-tagged string/float values (e.g. `"hello " + name` produced
   garbage). Now dispatches to `res_jit_string_concat` /
   `res_jit_float_{add,sub,mul,div,rem}` when `static_kind` says either
   operand is `String`/`Float`.
3. **RES-175 leaf-inliner dangling-pointer fix.** The trivial-leaf-fn
   inliner clones a callee's AST into a call-site-local temporary
   before re-lowering it; a string/float literal inside that clone
   bakes a `Cranelift iconst` pointing at the clone's bytes, which are
   freed once the temporary drops — a genuine use-after-free that
   produced the raw-pointer-string symptom. Fixed by disqualifying
   bodies containing `StringLiteral`/`StringInternLiteral`/
   `FloatLiteral` from inlining (`has_disqualifying_construct`); a
   non-inlined call lowers from the stable, whole-run-lifetime AST
   instead.
4. **Implicit top-level `return 0`, gated.** `compile_statements` now
   lowers non-terminating top-level fallthrough (the
   `fn main() { ... } main();` shape) to an implicit `return 0`,
   matching the walker/VM (RES-3991) — but only when
   `program_has_unsound_native_fallthrough_construct` finds none of
   `Match`, `IndexExpression`/`IndexAssignment` (negative-index
   handling), or an `impl Add/Sub/Mul/Div for T` operator-overload
   block anywhere in the program. Enabling fallthrough surfaced these
   three as separate, pre-existing, unrelated JIT-lowering
   correctness gaps (wrong match-arm selection, negative-index
   mishandling, and operator-overload dispatch corrupting/crashing on
   heap-tagged struct operands) that are each their own follow-up
   ticket, not RES-4153's scope. Programs touching any of them keep
   raising `EmptyProgram` and transparently VM-fall-back, per repo
   policy ("anything that diverges must fall back to VM, not
   denylist") — this is why coverage is 3.8%, not higher: it is
   deliberately conservative (whole-program, not
   reachable-from-`main`) in favor of never running natively with a
   wrong answer.

`interpreter_and_jit_agree_on_all_examples` (the differential parity
test) is green with zero denylist additions — every one of the bugs
above was root-caused and fixed rather than papered over.

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
