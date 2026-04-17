# Benchmark Results

Hardware: Darwin arm64, Apple M1 Max
Date: 2026-04-17 (refreshed for RES-106; sum/contract sections
unchanged from RES-082 / RES-095 era)
Resilient build: `cargo build --release` for interp/VM rows;
`cargo build --release --features jit` for the JIT row.

## fib(25) — recursive Fibonacci

Benchmark 1: Resilient (interp)
  Time (mean ± σ):     406.7 ms ±   6.4 ms    [User: 400.5 ms, System: 3.6 ms]
  Range (min … max):   398.1 ms … 414.4 ms    5 runs
 
Benchmark 2: Resilient (VM)
  Time (mean ± σ):      33.7 ms ±   0.4 ms    [User: 31.9 ms, System: 1.1 ms]
  Range (min … max):    33.1 ms …  34.1 ms    5 runs
 
Benchmark 3: Resilient (JIT)
  Time (mean ± σ):       2.8 ms ±   0.7 ms    [User: 1.6 ms, System: 0.8 ms]
  Range (min … max):     2.1 ms …   4.0 ms    5 runs
 
Benchmark 4: Python 3
  Time (mean ± σ):      32.5 ms ±   0.9 ms    [User: 25.4 ms, System: 5.4 ms]
  Range (min … max):    31.8 ms …  33.8 ms    5 runs
 
Benchmark 5: Node.js
  Time (mean ± σ):      62.8 ms ±   0.8 ms    [User: 55.5 ms, System: 8.6 ms]
  Range (min … max):    62.0 ms …  64.0 ms    5 runs
 
Benchmark 6: Lua
  Time (mean ± σ):       7.1 ms ±   0.4 ms    [User: 5.5 ms, System: 1.1 ms]
  Range (min … max):     6.5 ms …   7.5 ms    5 runs
 
Benchmark 7: Ruby
  Time (mean ± σ):      71.2 ms ±   1.1 ms    [User: 47.9 ms, System: 17.3 ms]
  Range (min … max):    69.7 ms …  72.7 ms    5 runs
 
Benchmark 8: Rust (native -O)
  Time (mean ± σ):       2.0 ms ±   0.4 ms    [User: 1.1 ms, System: 0.6 ms]
  Range (min … max):     1.4 ms …   2.4 ms    5 runs
 
Summary
  Rust (native -O) ran
    1.41 ± 0.45 times faster than Resilient (JIT)
    3.58 ± 0.73 times faster than Lua
   16.36 ± 3.20 times faster than Python 3
   16.97 ± 3.30 times faster than Resilient (VM)
   31.63 ± 6.15 times faster than Node.js
   35.89 ± 6.99 times faster than Ruby
  204.87 ± 39.88 times faster than Resilient (interp)

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `Resilient (interp)` | 406.7 ± 6.4 | 398.1 | 414.4 | 204.87 ± 39.88 |
| `Resilient (VM)` | 33.7 ± 0.4 | 33.1 | 34.1 | 16.97 ± 3.30 |
| `Resilient (JIT)` | 2.8 ± 0.7 | 2.1 | 4.0 | 1.41 ± 0.45 |
| `Python 3` | 32.5 ± 0.9 | 31.8 | 33.8 | 16.36 ± 3.20 |
| `Node.js` | 62.8 ± 0.8 | 62.0 | 64.0 | 31.63 ± 6.15 |
| `Lua` | 7.1 ± 0.4 | 6.5 | 7.5 | 3.58 ± 0.73 |
| `Ruby` | 71.2 ± 1.1 | 69.7 | 72.7 | 35.89 ± 6.99 |
| `Rust (native -O)` | 2.0 ± 0.4 | 1.4 | 2.4 | 1.00 |

**RES-082 / RES-095 result**: the bytecode VM runs fib(25) in
33.7 ms vs 406.7 ms for the tree walker — a **~12.1× speedup**,
well past the 3× target RES-076 set. The VM beats Node.js,
Ruby, is competitive with Python 3 (within noise), and lands
between Lua and native Rust.

**RES-106 result**: the Cranelift JIT (RES-072 Phases B–H,
closed across RES-096/099/100/102/103/104/105) runs fib(25) in
**2.8 ms** — **~12× faster than the VM**, **~145× faster than
the tree walker**, and only **~1.41× slower than native Rust
-O**. The JIT outperforms every other scripting language in the
table including Lua (7.1 ms) and is within statistical noise of
native Rust on this workload.

The JIT timing INCLUDES Cranelift's compile time (parse → AST
→ lower → register-alloc → x86_64/aarch64 codegen → finalize)
because we measure the binary's full invocation. Compile cost
is amortized across ~242,785 recursive `fib` calls, so the
per-call overhead is dominated by the call itself, not by
compilation. For shorter-running programs (e.g. one-shot
arithmetic) the JIT would be SLOWER than the VM since compile
time would dominate; the JIT is the right backend for
long-running or hot-loop workloads.

What the JIT doesn't yet do (for honest comparison): it can't
compile programs that use reassignment (`x = x + 1`),
while loops, closures, structs, arrays, or `live` blocks. The
tree walker and VM accept all of those. RES-107 lifts the
reassignment + while-loop limit; closures + structs are deeper
(no PRD ticket yet).

## sum 1..100000 — while-loop accumulator

Benchmark 1: Resilient (interp)
  Time (mean ± σ):      47.1 ms ±   0.3 ms    [User: 45.6 ms, System: 0.7 ms]
  Range (min … max):    46.8 ms …  47.4 ms    5 runs
 
Benchmark 2: Python 3
  Time (mean ± σ):      33.8 ms ±   1.9 ms    [User: 24.9 ms, System: 5.9 ms]
  Range (min … max):    31.6 ms …  36.1 ms    5 runs
 
Benchmark 3: Node.js
  Time (mean ± σ):      62.3 ms ±   3.1 ms    [User: 54.2 ms, System: 8.7 ms]
  Range (min … max):    57.6 ms …  66.1 ms    5 runs
 
Benchmark 4: Lua
  Time (mean ± σ):       4.6 ms ±   0.4 ms    [User: 2.0 ms, System: 0.9 ms]
  Range (min … max):     4.0 ms …   5.0 ms    5 runs
 
Benchmark 5: Ruby
  Time (mean ± σ):      60.4 ms ±   3.2 ms    [User: 39.1 ms, System: 15.2 ms]
  Range (min … max):    57.7 ms …  65.8 ms    5 runs
 
Benchmark 6: Rust (native -O)
  Time (mean ± σ):       1.7 ms ±   0.2 ms    [User: 0.9 ms, System: 0.6 ms]
  Range (min … max):     1.5 ms …   2.0 ms    5 runs
 
Summary
  Rust (native -O) ran
    2.72 ± 0.38 times faster than Lua
   19.92 ± 2.30 times faster than Python 3
   27.78 ± 2.82 times faster than Resilient (interp)
   35.61 ± 4.06 times faster than Ruby
   36.73 ± 4.15 times faster than Node.js

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `Resilient (interp)` | 47.1 ± 0.3 | 46.8 | 47.4 | 27.78 ± 2.82 |
| `Python 3` | 33.8 ± 1.9 | 31.6 | 36.1 | 19.92 ± 2.30 |
| `Node.js` | 62.3 ± 3.1 | 57.6 | 66.1 | 36.73 ± 4.15 |
| `Lua` | 4.6 ± 0.4 | 4.0 | 5.0 | 2.72 ± 0.38 |
| `Ruby` | 60.4 ± 3.2 | 57.7 | 65.8 | 35.61 ± 4.06 |
| `Rust (native -O)` | 1.7 ± 0.2 | 1.5 | 2.0 | 1.00 |


## contract overhead — 100k safe_div calls

Benchmark 1: Resilient + requires
  Time (mean ± σ):     149.3 ms ±   3.2 ms    [User: 146.8 ms, System: 1.1 ms]
  Range (min … max):   147.0 ms … 154.7 ms    5 runs
 
Benchmark 2: Resilient (no contract)
  Time (mean ± σ):     126.5 ms ±   2.1 ms    [User: 122.8 ms, System: 1.2 ms]
  Range (min … max):   122.9 ms … 128.4 ms    5 runs
 
Summary
  Resilient (no contract) ran
    1.18 ± 0.03 times faster than Resilient + requires

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `Resilient + requires` | 149.3 ± 3.2 | 147.0 | 154.7 | 1.18 ± 0.03 |
| `Resilient (no contract)` | 126.5 ± 2.1 | 122.9 | 128.4 | 1.00 |


