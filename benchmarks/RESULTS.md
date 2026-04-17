# Benchmark Results

Hardware: Darwin arm64, Apple M1 Max
Date: 2026-04-17T18:09:05Z
Resilient build: `cargo build --release` (default features)

## fib(25) — recursive Fibonacci

Benchmark 1: Resilient (interp)
  Time (mean ± σ):     396.8 ms ±  11.9 ms    [User: 388.9 ms, System: 3.9 ms]
  Range (min … max):   386.8 ms … 413.8 ms    5 runs
 
Benchmark 2: Resilient (VM)
  Time (mean ± σ):      30.8 ms ±   0.7 ms    [User: 27.8 ms, System: 1.6 ms]
  Range (min … max):    29.9 ms …  31.9 ms    5 runs
 
Benchmark 3: Python 3
  Time (mean ± σ):      35.2 ms ±   5.0 ms    [User: 26.0 ms, System: 6.0 ms]
  Range (min … max):    31.9 ms …  44.0 ms    5 runs
 
Benchmark 4: Node.js
  Time (mean ± σ):      64.1 ms ±   2.0 ms    [User: 55.6 ms, System: 9.4 ms]
  Range (min … max):    62.0 ms …  67.2 ms    5 runs
 
Benchmark 5: Lua
  Time (mean ± σ):       8.5 ms ±   0.7 ms    [User: 5.8 ms, System: 1.7 ms]
  Range (min … max):     7.5 ms …   9.4 ms    5 runs
 
Benchmark 6: Ruby
  Time (mean ± σ):      70.8 ms ±   1.5 ms    [User: 47.4 ms, System: 16.9 ms]
  Range (min … max):    68.9 ms …  72.4 ms    5 runs
 
Benchmark 7: Rust (native -O)
  Time (mean ± σ):       1.7 ms ±   0.3 ms    [User: 1.2 ms, System: 0.7 ms]
  Range (min … max):     1.3 ms …   2.0 ms    5 runs
 
Summary
  Rust (native -O) ran
    4.92 ± 0.90 times faster than Lua
   17.85 ± 2.91 times faster than Resilient (VM)
   20.40 ± 4.38 times faster than Python 3
   37.16 ± 6.11 times faster than Node.js
   41.02 ± 6.69 times faster than Ruby
  229.94 ± 37.78 times faster than Resilient (interp)

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `Resilient (interp)` | 396.8 ± 11.9 | 386.8 | 413.8 | 229.94 ± 37.78 |
| `Resilient (VM)` | 30.8 ± 0.7 | 29.9 | 31.9 | 17.85 ± 2.91 |
| `Python 3` | 35.2 ± 5.0 | 31.9 | 44.0 | 20.40 ± 4.38 |
| `Node.js` | 64.1 ± 2.0 | 62.0 | 67.2 | 37.16 ± 6.11 |
| `Lua` | 8.5 ± 0.7 | 7.5 | 9.4 | 4.92 ± 0.90 |
| `Ruby` | 70.8 ± 1.5 | 68.9 | 72.4 | 41.02 ± 6.69 |
| `Rust (native -O)` | 1.7 ± 0.3 | 1.3 | 2.0 | 1.00 |

**RES-082 VM-vs-interp result**: the bytecode VM runs fib(25) in
30.8 ms vs 396.8 ms for the tree walker — a **~12.9× speedup**,
well past the 3× target RES-076 set. The VM lands on the fast
side of Python (35.2 ms) and between Lua (8.5 ms) and Node.js
(64.1 ms). The biggest remaining delta vs stock Lua is Value
cloning on every LoadLocal/StoreLocal — a register-based VM or
copy-on-write Value could close more of that gap. Tracked as a
future follow-up, not gating.

## sum 1..100000 — while-loop accumulator

Benchmark 1: Resilient (interp)
  Time (mean ± σ):      49.7 ms ±   0.6 ms    [User: 46.4 ms, System: 1.6 ms]
  Range (min … max):    49.2 ms …  50.6 ms    5 runs
 
Benchmark 2: Python 3
  Time (mean ± σ):      37.4 ms ±   6.8 ms    [User: 25.9 ms, System: 6.1 ms]
  Range (min … max):    32.1 ms …  48.1 ms    5 runs
 
Benchmark 3: Node.js
  Time (mean ± σ):      61.2 ms ±   1.9 ms    [User: 53.3 ms, System: 8.5 ms]
  Range (min … max):    58.9 ms …  63.5 ms    5 runs
 
Benchmark 4: Lua
  Time (mean ± σ):       4.3 ms ±   0.7 ms    [User: 2.0 ms, System: 0.9 ms]
  Range (min … max):     3.2 ms …   4.9 ms    5 runs
 
Benchmark 5: Ruby
  Time (mean ± σ):      64.8 ms ±   1.0 ms    [User: 41.7 ms, System: 16.1 ms]
  Range (min … max):    64.0 ms …  66.5 ms    5 runs
 
Benchmark 6: Rust (native -O)
  Time (mean ± σ):       1.9 ms ±   0.1 ms    [User: 1.0 ms, System: 0.6 ms]
  Range (min … max):     1.8 ms …   2.1 ms    5 runs
 
Summary
  Rust (native -O) ran
    2.28 ± 0.38 times faster than Lua
   19.71 ± 3.74 times faster than Python 3
   26.18 ± 1.46 times faster than Resilient (interp)
   32.27 ± 2.02 times faster than Node.js
   34.14 ± 1.94 times faster than Ruby

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `Resilient (interp)` | 49.7 ± 0.6 | 49.2 | 50.6 | 26.18 ± 1.46 |
| `Python 3` | 37.4 ± 6.8 | 32.1 | 48.1 | 19.71 ± 3.74 |
| `Node.js` | 61.2 ± 1.9 | 58.9 | 63.5 | 32.27 ± 2.02 |
| `Lua` | 4.3 ± 0.7 | 3.2 | 4.9 | 2.28 ± 0.38 |
| `Ruby` | 64.8 ± 1.0 | 64.0 | 66.5 | 34.14 ± 1.94 |
| `Rust (native -O)` | 1.9 ± 0.1 | 1.8 | 2.1 | 1.00 |


## contract overhead — 100k safe_div calls

Benchmark 1: Resilient + requires
  Time (mean ± σ):     147.0 ms ±   1.8 ms    [User: 143.7 ms, System: 1.5 ms]
  Range (min … max):   145.2 ms … 149.6 ms    5 runs
 
Benchmark 2: Resilient (no contract)
  Time (mean ± σ):     126.9 ms ±   1.9 ms    [User: 121.7 ms, System: 2.3 ms]
  Range (min … max):   125.1 ms … 129.2 ms    5 runs
 
Summary
  Resilient (no contract) ran
    1.16 ± 0.02 times faster than Resilient + requires

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `Resilient + requires` | 147.0 ± 1.8 | 145.2 | 149.6 | 1.16 ± 0.02 |
| `Resilient (no contract)` | 126.9 ± 1.9 | 125.1 | 129.2 | 1.00 |


