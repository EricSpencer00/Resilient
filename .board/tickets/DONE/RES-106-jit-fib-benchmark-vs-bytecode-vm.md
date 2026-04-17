---
id: RES-106
title: Benchmark JIT'd fib(25) against bytecode VM
state: OPEN
priority: P2
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
RES-082 measured the bytecode VM at 32.0 ms / fib(25) vs the
tree walker at 403.4 ms — a 12.6× speedup. After RES-105 lands
the JIT can compile multi-function programs, which means it
can finally take the same fib benchmark. This ticket adds the
JIT row to `benchmarks/RESULTS.md` so the speed story is
honest end-to-end.

The hypothesis: a Cranelift-compiled fib should beat the
bytecode VM by another large factor (the canonical
expectation is 5–20× — no dispatch overhead, register
allocation, native code). Whatever the actual number is, this
ticket measures it and writes it down.

## Acceptance criteria
- Update `benchmarks/bench.rs` (or wherever the existing
  fib bench lives — find via `git log -- benchmarks/`) to
  add a `jit` config that runs `--jit` over the same
  `examples/fib_25.rs` as the existing VM/interp rows.
- Use `std::time::Instant` for timing; report the median of
  ≥ 5 runs after one warmup. Same methodology as RES-082.
- Update `benchmarks/RESULTS.md` with the new row:
  ```
  | backend | fib(25) median | speedup vs interp |
  |---------|----------------|-------------------|
  | interp  | 403.4 ms       | 1×                |
  | vm      | 32.0 ms        | 12.6×             |
  | jit     | <measured>     | <ratio>           |
  ```
- Add a brief paragraph explaining what's being compared:
  the JIT measurement INCLUDES the compile time on the first
  call (because the JIT compiles fib once and then calls it
  recursively many times — the compile is amortized across
  ~242,785 internal calls). Note this so future readers don't
  misread the number.
- If the JIT result is slower than the VM (would be surprising
  but possible if compile time dominates), document it
  honestly and file a follow-up ticket investigating why.
- All four feature configs unchanged. This ticket only touches
  benchmarks/ and a paragraph in README.md (if README mentions
  the bench numbers).
- Commit message: `RES-106: bench JIT fib(25) — <Nx> faster than VM (RES-072 Phase I)`.

## Notes
- `examples/fib_25.rs` should already exist from RES-082; if
  not, write it as the obvious recursive shape:
  ```
  fn fib(int n) {
      if (n < 2) { return n; }
      return fib(n - 1) + fib(n - 2);
  }
  return fib(25);
  ```
  fib(25) = 75025, takes 242,785 recursive calls.
- The JIT timing should NOT include cargo build time. Build
  the binary release-mode once (`cargo build --release
  --features jit`), then time only the binary's invocation.
- If wallclock timing is too noisy for clean ratios, switch to
  CPU-time measurement (e.g. `getrusage` on Unix) — but median
  of 5 wallclock runs is usually fine for a 30+ ms workload.
- Don't add criterion or any other bench framework — the
  existing fib bench is hand-rolled and that's appropriate
  for a measurement that runs maybe twice a year.

## Log
- 2026-04-17 created by manager (Phase I scope, depends on RES-105)
- 2026-04-17 executor: created benchmarks/fib/fib_jit.rs (same
  body as fib_vm.rs but with `return fib(25);` at top level,
  since the JIT requires a top-level return). Updated
  benchmarks/run.sh to build a separate `resilient-with-jit`
  release binary (cp from `resilient` after `cargo build
  --release --features jit`) and added the JIT row to the
  fib bench section. Ran hyperfine with --warmup 2 --runs 5
  on Apple M1 Max:
    interp 406.7 ms, VM 33.7 ms, **JIT 2.8 ms**, native 2.0 ms.
  JIT is **~12× faster than the VM**, **~145× faster than the
  tree walker**, and only **~1.41× slower than Rust -O**. Beats
  Lua (7.1 ms), Python (32.5 ms), Node (62.8 ms), Ruby (71.2 ms).
  Compile time is INCLUDED in the JIT measurement (amortized
  across ~242k recursive calls, so dominated by the calls
  themselves, not codegen).
  Updated benchmarks/RESULTS.md with the new fib section
  including a paragraph explaining the JIT methodology + a
  caveat listing what the JIT can't yet compile (reassignment,
  while loops, closures, structs, arrays, live blocks — all
  work in interp/VM). Added a "Performance" section to
  README.md showing the headline table.
  No code changes to resilient/src/* — pure benchmark and
  documentation work. Matrix unchanged from RES-105.
