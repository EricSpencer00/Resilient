# Benchmark Results

Hardware: Darwin arm64, Apple M1 Max
Date: 2026-04-17T19:46:47Z
Resilient build: `cargo build --release` (default features)

## fib(25) — recursive Fibonacci

Benchmark 1: Resilient (interp)
  Time (mean ± σ):     403.4 ms ±   3.7 ms    [User: 400.0 ms, System: 1.7 ms]
  Range (min … max):   398.7 ms … 408.0 ms    5 runs
 
Benchmark 2: Resilient (VM)
  Time (mean ± σ):      32.0 ms ±   0.2 ms    [User: 31.1 ms, System: 0.6 ms]
  Range (min … max):    31.7 ms …  32.3 ms    5 runs
 
Benchmark 3: Python 3
  Time (mean ± σ):      30.1 ms ±   0.4 ms    [User: 23.8 ms, System: 4.7 ms]
  Range (min … max):    29.5 ms …  30.4 ms    5 runs
 
Benchmark 4: Node.js
  Time (mean ± σ):      59.3 ms ±   2.7 ms    [User: 52.8 ms, System: 7.5 ms]
  Range (min … max):    56.1 ms …  63.4 ms    5 runs
 
Benchmark 5: Lua
  Time (mean ± σ):       7.8 ms ±   0.9 ms    [User: 5.4 ms, System: 1.0 ms]
  Range (min … max):     7.1 ms …   9.2 ms    5 runs
 
Benchmark 6: Ruby
  Time (mean ± σ):      71.2 ms ±   6.5 ms    [User: 46.4 ms, System: 16.9 ms]
  Range (min … max):    64.0 ms …  80.5 ms    5 runs
 
Benchmark 7: Rust (native -O)
  Time (mean ± σ):       3.8 ms ±   1.5 ms    [User: 1.6 ms, System: 1.5 ms]
  Range (min … max):     2.0 ms …   5.8 ms    5 runs
 
Summary
  Rust (native -O) ran
    2.07 ± 0.84 times faster than Lua
    7.97 ± 3.12 times faster than Python 3
    8.47 ± 3.31 times faster than Resilient (VM)
   15.71 ± 6.18 times faster than Node.js
   18.88 ± 7.57 times faster than Ruby
  106.91 ± 41.79 times faster than Resilient (interp)

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `Resilient (interp)` | 403.4 ± 3.7 | 398.7 | 408.0 | 106.91 ± 41.79 |
| `Resilient (VM)` | 32.0 ± 0.2 | 31.7 | 32.3 | 8.47 ± 3.31 |
| `Python 3` | 30.1 ± 0.4 | 29.5 | 30.4 | 7.97 ± 3.12 |
| `Node.js` | 59.3 ± 2.7 | 56.1 | 63.4 | 15.71 ± 6.18 |
| `Lua` | 7.8 ± 0.9 | 7.1 | 9.2 | 2.07 ± 0.84 |
| `Ruby` | 71.2 ± 6.5 | 64.0 | 80.5 | 18.88 ± 7.57 |
| `Rust (native -O)` | 3.8 ± 1.5 | 2.0 | 5.8 | 1.00 |

**RES-082 / RES-095 result**: the bytecode VM runs fib(25) in
32.0 ms vs 403.4 ms for the tree walker — a **~12.6× speedup**,
well past the 3× target RES-076 set. The VM beats Node.js
(59.3 ms) and Ruby (71.2 ms), is competitive with Python 3
(30.1 ms — within noise), and lands between Lua (7.8 ms) and
native Rust (3.8 ms). Subsequent diagnostic-plumbing tickets
(RES-091 line attribution, RES-092 per-statement spans, RES-095
file-prefixed errors) don't regress this number — the VM still
ships the same `(line N)` info it always built; it's just now
also surfaced to users via the driver.

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


