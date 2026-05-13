# Compile-time benchmark results

Hardware: Darwin arm64
Date:     2026-05-13T04:38:44Z
Compiler: `rz 0.1.0: pre-1.0 — breaking changes possible (see STABILITY.md)`

Hyperfine settings: 2 warmup runs, 5 measured runs per row.

## typecheck + interpret (default features)

Benchmark 1: small.rz
  Time (mean ± σ):       4.8 ms ±   0.3 ms    [User: 3.2 ms, System: 1.6 ms]
  Range (min … max):     4.7 ms …   5.3 ms    5 runs
 
Benchmark 2: medium.rz
  Time (mean ± σ):      10.8 ms ±   0.2 ms    [User: 8.4 ms, System: 2.1 ms]
  Range (min … max):    10.5 ms …  11.0 ms    5 runs
 
Benchmark 3: large.rz
  Time (mean ± σ):      95.7 ms ±   1.2 ms    [User: 91.8 ms, System: 3.1 ms]
  Range (min … max):    94.7 ms …  97.7 ms    5 runs
 
Summary
  small.rz ran
    2.23 ± 0.12 times faster than medium.rz
   19.81 ± 1.06 times faster than large.rz

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `small.rz` | 4.8 ± 0.3 | 4.7 | 5.3 | 1.00 |
| `medium.rz` | 10.8 ± 0.2 | 10.5 | 11.0 | 2.23 ± 0.12 |
| `large.rz` | 95.7 ± 1.2 | 94.7 | 97.7 | 19.81 ± 1.06 |


## cargo check (compiler tree)

Benchmark 1: cargo check (cold)
  Time (mean ± σ):     115.0 ms ±   0.7 ms    [User: 72.5 ms, System: 34.9 ms]
  Range (min … max):   114.3 ms … 115.9 ms    5 runs
 
Benchmark 2: cargo check (warm)
  Time (mean ± σ):     114.8 ms ±   1.3 ms    [User: 72.1 ms, System: 34.9 ms]
  Range (min … max):   113.6 ms … 116.9 ms    5 runs
 
Summary
  cargo check (warm) ran
    1.00 ± 0.01 times faster than cargo check (cold)

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `cargo check (cold)` | 115.0 ± 0.7 | 114.3 | 115.9 | 1.00 ± 0.01 |
| `cargo check (warm)` | 114.8 ± 1.3 | 113.6 | 116.9 | 1.00 |


## cargo build (compiler tree, release)

Benchmark 1: cargo build --release (warm)
  Time (mean ± σ):     119.7 ms ±   3.7 ms    [User: 73.7 ms, System: 36.8 ms]
  Range (min … max):   115.6 ms … 124.8 ms    5 runs
 

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `cargo build --release (warm)` | 119.7 ± 3.7 | 115.6 | 124.8 | 1.00 |


