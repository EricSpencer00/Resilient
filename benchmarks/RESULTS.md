# Benchmark Results

Hardware: Darwin arm64, Apple M1 Max
Date: 2026-04-17T16:27:01Z
Resilient build: `cargo build --release` (default features)

## fib(25) — recursive Fibonacci

Benchmark 1: Resilient (interp)
  Time (mean ± σ):     397.9 ms ±  10.7 ms    [User: 389.2 ms, System: 4.5 ms]
  Range (min … max):   388.7 ms … 414.4 ms    5 runs
 
Benchmark 2: Python 3
  Time (mean ± σ):      32.7 ms ±   0.5 ms    [User: 25.2 ms, System: 5.6 ms]
  Range (min … max):    32.0 ms …  33.4 ms    5 runs
 
Benchmark 3: Node.js
  Time (mean ± σ):      63.3 ms ±   1.0 ms    [User: 55.4 ms, System: 8.9 ms]
  Range (min … max):    62.3 ms …  64.4 ms    5 runs
 
Benchmark 4: Lua
  Time (mean ± σ):       7.7 ms ±   1.7 ms    [User: 5.5 ms, System: 1.0 ms]
  Range (min … max):     6.1 ms …  10.2 ms    5 runs
 
Benchmark 5: Ruby
  Time (mean ± σ):      74.1 ms ±   6.6 ms    [User: 47.9 ms, System: 17.2 ms]
  Range (min … max):    68.7 ms …  85.0 ms    5 runs
 
Benchmark 6: Rust (native -O)
  Time (mean ± σ):       1.2 ms ±   0.4 ms    [User: 1.2 ms, System: 0.5 ms]
  Range (min … max):     0.6 ms …   1.6 ms    5 runs
 
Summary
  Rust (native -O) ran
    6.38 ± 2.43 times faster than Lua
   27.01 ± 8.49 times faster than Python 3
   52.23 ± 16.42 times faster than Node.js
   61.15 ± 19.95 times faster than Ruby
  328.43 ± 103.52 times faster than Resilient (interp)

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `Resilient (interp)` | 397.9 ± 10.7 | 388.7 | 414.4 | 328.43 ± 103.52 |
| `Python 3` | 32.7 ± 0.5 | 32.0 | 33.4 | 27.01 ± 8.49 |
| `Node.js` | 63.3 ± 1.0 | 62.3 | 64.4 | 52.23 ± 16.42 |
| `Lua` | 7.7 ± 1.7 | 6.1 | 10.2 | 6.38 ± 2.43 |
| `Ruby` | 74.1 ± 6.6 | 68.7 | 85.0 | 61.15 ± 19.95 |
| `Rust (native -O)` | 1.2 ± 0.4 | 0.6 | 1.6 | 1.00 |


## sum 1..100000 — while-loop accumulator

Benchmark 1: Resilient (interp)
  Time (mean ± σ):      49.6 ms ±   0.4 ms    [User: 47.5 ms, System: 1.0 ms]
  Range (min … max):    49.2 ms …  50.2 ms    5 runs
 
Benchmark 2: Python 3
  Time (mean ± σ):      39.0 ms ±   7.8 ms    [User: 26.2 ms, System: 6.3 ms]
  Range (min … max):    34.2 ms …  52.6 ms    5 runs
 
Benchmark 3: Node.js
  Time (mean ± σ):      63.9 ms ±   1.6 ms    [User: 55.7 ms, System: 9.2 ms]
  Range (min … max):    61.8 ms …  66.4 ms    5 runs
 
Benchmark 4: Lua
  Time (mean ± σ):       4.8 ms ±   2.5 ms    [User: 2.0 ms, System: 0.8 ms]
  Range (min … max):     2.7 ms …   8.7 ms    5 runs
 
Benchmark 5: Ruby
  Time (mean ± σ):      69.2 ms ±   3.6 ms    [User: 42.4 ms, System: 18.0 ms]
  Range (min … max):    65.5 ms …  74.6 ms    5 runs
 
Benchmark 6: Rust (native -O)
  Time (mean ± σ):       2.3 ms ±   0.5 ms    [User: 1.1 ms, System: 0.8 ms]
  Range (min … max):     1.8 ms …   3.2 ms    5 runs
 
Summary
  Rust (native -O) ran
    2.06 ± 1.15 times faster than Lua
   16.75 ± 5.06 times faster than Python 3
   21.34 ± 4.85 times faster than Resilient (interp)
   27.48 ± 6.27 times faster than Node.js
   29.73 ± 6.92 times faster than Ruby

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `Resilient (interp)` | 49.6 ± 0.4 | 49.2 | 50.2 | 21.34 ± 4.85 |
| `Python 3` | 39.0 ± 7.8 | 34.2 | 52.6 | 16.75 ± 5.06 |
| `Node.js` | 63.9 ± 1.6 | 61.8 | 66.4 | 27.48 ± 6.27 |
| `Lua` | 4.8 ± 2.5 | 2.7 | 8.7 | 2.06 ± 1.15 |
| `Ruby` | 69.2 ± 3.6 | 65.5 | 74.6 | 29.73 ± 6.92 |
| `Rust (native -O)` | 2.3 ± 0.5 | 1.8 | 3.2 | 1.00 |


## contract overhead — 100k safe_div calls

Benchmark 1: Resilient + requires
  Time (mean ± σ):     155.5 ms ±   2.5 ms    [User: 150.8 ms, System: 1.6 ms]
  Range (min … max):   151.4 ms … 157.5 ms    5 runs
 
Benchmark 2: Resilient (no contract)
  Time (mean ± σ):     128.6 ms ±   2.9 ms    [User: 123.7 ms, System: 1.5 ms]
  Range (min … max):   125.3 ms … 132.1 ms    5 runs
 
Summary
  Resilient (no contract) ran
    1.21 ± 0.03 times faster than Resilient + requires

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `Resilient + requires` | 155.5 ± 2.5 | 151.4 | 157.5 | 1.21 ± 0.03 |
| `Resilient (no contract)` | 128.6 ± 2.9 | 125.3 | 132.1 | 1.00 |


