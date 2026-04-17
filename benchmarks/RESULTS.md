# Benchmark Results

Hardware: Darwin arm64, Apple M1 Max
Date: 2026-04-17T15:46:33Z
Resilient build: `cargo build --release` (default features)

## fib(25) — recursive Fibonacci

Benchmark 1: Resilient (interp)
  Time (mean ± σ):      2.892 s ±  0.006 s    [User: 2.848 s, System: 0.030 s]
  Range (min … max):    2.885 s …  2.898 s    5 runs
 
Benchmark 2: Python 3
  Time (mean ± σ):      33.3 ms ±   0.3 ms    [User: 25.4 ms, System: 6.0 ms]
  Range (min … max):    32.9 ms …  33.6 ms    5 runs
 
Benchmark 3: Node.js
  Time (mean ± σ):      62.7 ms ±   1.8 ms    [User: 54.6 ms, System: 9.3 ms]
  Range (min … max):    60.3 ms …  64.6 ms    5 runs
 
Benchmark 4: Lua
  Time (mean ± σ):       7.4 ms ±   0.6 ms    [User: 5.5 ms, System: 1.1 ms]
  Range (min … max):     7.0 ms …   8.4 ms    5 runs
 
Benchmark 5: Ruby
  Time (mean ± σ):      73.6 ms ±   2.8 ms    [User: 48.2 ms, System: 17.6 ms]
  Range (min … max):    71.7 ms …  78.5 ms    5 runs
 
Benchmark 6: Rust (native -O)
  Time (mean ± σ):       2.0 ms ±   0.1 ms    [User: 1.2 ms, System: 0.8 ms]
  Range (min … max):     1.8 ms …   2.2 ms    5 runs
 
Summary
  Rust (native -O) ran
    3.68 ± 0.36 times faster than Lua
   16.49 ± 1.00 times faster than Python 3
   31.04 ± 2.06 times faster than Node.js
   36.39 ± 2.58 times faster than Ruby
 1430.56 ± 85.80 times faster than Resilient (interp)

| Command | Mean [s] | Min [s] | Max [s] | Relative |
|:---|---:|---:|---:|---:|
| `Resilient (interp)` | 2.892 ± 0.006 | 2.885 | 2.898 | 1430.56 ± 85.80 |
| `Python 3` | 0.033 ± 0.000 | 0.033 | 0.034 | 16.49 ± 1.00 |
| `Node.js` | 0.063 ± 0.002 | 0.060 | 0.065 | 31.04 ± 2.06 |
| `Lua` | 0.007 ± 0.001 | 0.007 | 0.008 | 3.68 ± 0.36 |
| `Ruby` | 0.074 ± 0.003 | 0.072 | 0.079 | 36.39 ± 2.58 |
| `Rust (native -O)` | 0.002 ± 0.000 | 0.002 | 0.002 | 1.00 |


## sum 1..100000 — while-loop accumulator

Benchmark 1: Resilient (interp)
  Time (mean ± σ):      50.0 ms ±   0.3 ms    [User: 46.8 ms, System: 1.6 ms]
  Range (min … max):    49.5 ms …  50.1 ms    5 runs
 
Benchmark 2: Python 3
  Time (mean ± σ):      33.9 ms ±   1.1 ms    [User: 25.3 ms, System: 5.9 ms]
  Range (min … max):    33.0 ms …  35.6 ms    5 runs
 
Benchmark 3: Node.js
  Time (mean ± σ):      63.3 ms ±   1.8 ms    [User: 54.4 ms, System: 9.6 ms]
  Range (min … max):    61.7 ms …  66.3 ms    5 runs
 
Benchmark 4: Lua
  Time (mean ± σ):       4.1 ms ±   0.4 ms    [User: 2.0 ms, System: 0.9 ms]
  Range (min … max):     3.6 ms …   4.7 ms    5 runs
 
Benchmark 5: Ruby
  Time (mean ± σ):      67.4 ms ±   1.4 ms    [User: 42.5 ms, System: 17.4 ms]
  Range (min … max):    65.7 ms …  69.4 ms    5 runs
 
Benchmark 6: Rust (native -O)
  Time (mean ± σ):       2.4 ms ±   0.3 ms    [User: 1.1 ms, System: 0.9 ms]
  Range (min … max):     2.1 ms …   2.8 ms    5 runs
 
Summary
  Rust (native -O) ran
    1.74 ± 0.26 times faster than Lua
   14.39 ± 1.68 times faster than Python 3
   21.23 ± 2.38 times faster than Resilient (interp)
   26.89 ± 3.10 times faster than Node.js
   28.62 ± 3.26 times faster than Ruby

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `Resilient (interp)` | 50.0 ± 0.3 | 49.5 | 50.1 | 21.23 ± 2.38 |
| `Python 3` | 33.9 ± 1.1 | 33.0 | 35.6 | 14.39 ± 1.68 |
| `Node.js` | 63.3 ± 1.8 | 61.7 | 66.3 | 26.89 ± 3.10 |
| `Lua` | 4.1 ± 0.4 | 3.6 | 4.7 | 1.74 ± 0.26 |
| `Ruby` | 67.4 ± 1.4 | 65.7 | 69.4 | 28.62 ± 3.26 |
| `Rust (native -O)` | 2.4 ± 0.3 | 2.1 | 2.8 | 1.00 |


## contract overhead — 100k safe_div calls

Benchmark 1: Resilient + requires
  Time (mean ± σ):      2.049 s ±  0.159 s    [User: 1.970 s, System: 0.022 s]
  Range (min … max):    1.961 s …  2.332 s    5 runs
 
Benchmark 2: Resilient (no contract)
  Time (mean ± σ):      1.885 s ±  0.019 s    [User: 1.849 s, System: 0.020 s]
  Range (min … max):    1.865 s …  1.911 s    5 runs
 
Summary
  Resilient (no contract) ran
    1.09 ± 0.09 times faster than Resilient + requires

| Command | Mean [s] | Min [s] | Max [s] | Relative |
|:---|---:|---:|---:|---:|
| `Resilient + requires` | 2.049 ± 0.159 | 1.961 | 2.332 | 1.09 ± 0.09 |
| `Resilient (no contract)` | 1.885 ± 0.019 | 1.865 | 1.911 | 1.00 |


