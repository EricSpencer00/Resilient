# Benchmark Results

Hardware: Darwin arm64, Apple M1 Max
Date: 2026-04-17T16:55:05Z
Resilient build: `cargo build --release` (default features)

## fib(25) — recursive Fibonacci

Benchmark 1: Resilient (interp)
  Time (mean ± σ):     391.3 ms ±   3.3 ms    [User: 384.2 ms, System: 4.5 ms]
  Range (min … max):   387.3 ms … 394.7 ms    5 runs
 
Benchmark 2: Python 3
  Time (mean ± σ):      33.2 ms ±   0.9 ms    [User: 25.5 ms, System: 5.9 ms]
  Range (min … max):    32.2 ms …  34.4 ms    5 runs
 
Benchmark 3: Node.js
  Time (mean ± σ):      63.2 ms ±   0.5 ms    [User: 55.1 ms, System: 9.0 ms]
  Range (min … max):    62.6 ms …  63.8 ms    5 runs
 
Benchmark 4: Lua
  Time (mean ± σ):       7.3 ms ±   0.7 ms    [User: 5.5 ms, System: 0.9 ms]
  Range (min … max):     6.6 ms …   8.1 ms    5 runs
 
Benchmark 5: Ruby
  Time (mean ± σ):      72.3 ms ±   2.6 ms    [User: 47.4 ms, System: 17.5 ms]
  Range (min … max):    70.1 ms …  76.7 ms    5 runs
 
Benchmark 6: Rust (native -O)
  Time (mean ± σ):       2.4 ms ±   0.5 ms    [User: 1.4 ms, System: 0.8 ms]
  Range (min … max):     1.8 ms …   3.0 ms    5 runs
 
Summary
  Rust (native -O) ran
    3.09 ± 0.70 times faster than Lua
   14.08 ± 2.93 times faster than Python 3
   26.75 ± 5.52 times faster than Node.js
   30.63 ± 6.41 times faster than Ruby
  165.74 ± 34.19 times faster than Resilient (interp)

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `Resilient (interp)` | 391.3 ± 3.3 | 387.3 | 394.7 | 165.74 ± 34.19 |
| `Python 3` | 33.2 ± 0.9 | 32.2 | 34.4 | 14.08 ± 2.93 |
| `Node.js` | 63.2 ± 0.5 | 62.6 | 63.8 | 26.75 ± 5.52 |
| `Lua` | 7.3 ± 0.7 | 6.6 | 8.1 | 3.09 ± 0.70 |
| `Ruby` | 72.3 ± 2.6 | 70.1 | 76.7 | 30.63 ± 6.41 |
| `Rust (native -O)` | 2.4 ± 0.5 | 1.8 | 3.0 | 1.00 |


## sum 1..100000 — while-loop accumulator

Benchmark 1: Resilient (interp)
  Time (mean ± σ):      48.7 ms ±   0.4 ms    [User: 45.9 ms, System: 1.5 ms]
  Range (min … max):    48.1 ms …  49.2 ms    5 runs
 
Benchmark 2: Python 3
  Time (mean ± σ):      33.4 ms ±   0.8 ms    [User: 24.8 ms, System: 5.8 ms]
  Range (min … max):    32.4 ms …  34.5 ms    5 runs
 
Benchmark 3: Node.js
  Time (mean ± σ):      63.1 ms ±   2.2 ms    [User: 54.6 ms, System: 9.8 ms]
  Range (min … max):    61.1 ms …  66.9 ms    5 runs
 
Benchmark 4: Lua
  Time (mean ± σ):       3.8 ms ±   0.4 ms    [User: 2.0 ms, System: 0.9 ms]
  Range (min … max):     3.4 ms …   4.3 ms    5 runs
 
Benchmark 5: Ruby
  Time (mean ± σ):      67.5 ms ±   1.9 ms    [User: 42.3 ms, System: 17.4 ms]
  Range (min … max):    65.4 ms …  70.3 ms    5 runs
 
Benchmark 6: Rust (native -O)
  Time (mean ± σ):       2.2 ms ±   0.2 ms    [User: 1.1 ms, System: 0.7 ms]
  Range (min … max):     2.0 ms …   2.4 ms    5 runs
 
Summary
  Rust (native -O) ran
    1.71 ± 0.21 times faster than Lua
   14.95 ± 1.28 times faster than Python 3
   21.76 ± 1.79 times faster than Resilient (interp)
   28.22 ± 2.51 times faster than Node.js
   30.18 ± 2.61 times faster than Ruby

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `Resilient (interp)` | 48.7 ± 0.4 | 48.1 | 49.2 | 21.76 ± 1.79 |
| `Python 3` | 33.4 ± 0.8 | 32.4 | 34.5 | 14.95 ± 1.28 |
| `Node.js` | 63.1 ± 2.2 | 61.1 | 66.9 | 28.22 ± 2.51 |
| `Lua` | 3.8 ± 0.4 | 3.4 | 4.3 | 1.71 ± 0.21 |
| `Ruby` | 67.5 ± 1.9 | 65.4 | 70.3 | 30.18 ± 2.61 |
| `Rust (native -O)` | 2.2 ± 0.2 | 2.0 | 2.4 | 1.00 |


## contract overhead — 100k safe_div calls

Benchmark 1: Resilient + requires
  Time (mean ± σ):     147.0 ms ±   0.7 ms    [User: 142.7 ms, System: 2.6 ms]
  Range (min … max):   145.9 ms … 147.7 ms    5 runs
 
Benchmark 2: Resilient (no contract)
  Time (mean ± σ):     126.2 ms ±   1.9 ms    [User: 121.4 ms, System: 2.4 ms]
  Range (min … max):   123.8 ms … 129.2 ms    5 runs
 
Summary
  Resilient (no contract) ran
    1.16 ± 0.02 times faster than Resilient + requires

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `Resilient + requires` | 147.0 ± 0.7 | 145.9 | 147.7 | 1.16 ± 0.02 |
| `Resilient (no contract)` | 126.2 ± 1.9 | 123.8 | 129.2 | 1.00 |


