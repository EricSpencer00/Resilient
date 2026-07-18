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

Current numbers (615 examples swept; 98 error out under `--verbose`'s
implied `--typecheck` for reasons unrelated to the JIT — deliberately
non-type-clean examples — and are excluded from the native/fallback
split):

| | Count | % of runnable (517) |
|---|---:|---:|
| Native (no fallback) | 0 | 0% |
| Fallback to VM | 517 | 100% |

**Headline finding: 0% of the example corpus executes natively through
the JIT today**, even though the differential pass (#4135) shows
interpreter/`--jit` output parity across nearly the whole corpus. The
dominant cause is not the string-literal gap #4134 names first — it's
that **the JIT has no builtin-call lowering at all** yet
(`jit: unsupported: call to unknown function`), and `println(...)` is
used by virtually every example to produce observable output. A purely
arithmetic example that only *prints* its result already falls back.

This reframes the rest of #4134's remaining scope: string/struct
lowering will matter, but builtin-call lowering (starting with
`println`) is the higher-leverage next increment for moving this
number off zero, since almost nothing in the corpus can stay native
without it. Filed as a note here rather than a new ticket since #4134
already owns "native lowering remainder."

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
