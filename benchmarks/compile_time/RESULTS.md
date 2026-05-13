# Compile-time benchmark results

Hardware: Darwin arm64
Date:     2026-05-13T06:04:19Z
Compiler: `rz 0.2.0: pre-1.0 — breaking changes possible (see STABILITY.md)`

Hyperfine settings: 2 warmup runs, 5 measured runs per row.

**Typecheck delta** = section 1 mean minus section 1b mean.
That's the wall-time cost of the 130-pass typechecker
fan-out for each input. The RES-1585 / 1590 / 1593 / 1597
/ 1599 / 1605 / 1607 / 1611 / 1612 / 1615 / 1616 / 1619 /
1620 / 1623 PR series is what shows up here per-PR.

## typecheck + interpret (default features)

Benchmark 1: small.rz
  Time (mean ± σ):       6.0 ms ±   1.6 ms    [User: 3.1 ms, System: 1.6 ms]
  Range (min … max):     4.6 ms …   8.6 ms    5 runs
 
Benchmark 2: medium.rz
  Time (mean ± σ):       8.7 ms ±   0.3 ms    [User: 6.6 ms, System: 1.4 ms]
  Range (min … max):     8.4 ms …   9.1 ms    5 runs
 
Benchmark 3: large.rz
  Time (mean ± σ):      94.9 ms ±   0.7 ms    [User: 90.5 ms, System: 2.6 ms]
  Range (min … max):    93.7 ms …  95.4 ms    5 runs
 
Summary
  small.rz ran
    1.46 ± 0.40 times faster than medium.rz
   15.90 ± 4.33 times faster than large.rz

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `small.rz` | 6.0 ± 1.6 | 4.6 | 8.6 | 1.00 |
| `medium.rz` | 8.7 ± 0.3 | 8.4 | 9.1 | 1.46 ± 0.40 |
| `large.rz` | 94.9 ± 0.7 | 93.7 | 95.4 | 15.90 ± 4.33 |


## interpret-only (--no-typecheck) baseline

Benchmark 1: small.rz  --no-typecheck
  Time (mean ± σ):       4.5 ms ±   0.2 ms    [User: 2.6 ms, System: 1.5 ms]
  Range (min … max):     4.3 ms …   4.8 ms    5 runs
 
Benchmark 2: medium.rz --no-typecheck
  Time (mean ± σ):       7.0 ms ±   0.5 ms    [User: 4.6 ms, System: 1.7 ms]
  Range (min … max):     6.6 ms …   7.8 ms    5 runs
 
Benchmark 3: large.rz  --no-typecheck
  Time (mean ± σ):      90.7 ms ±   0.8 ms    [User: 86.8 ms, System: 2.5 ms]
  Range (min … max):    89.6 ms …  91.5 ms    5 runs
 
Summary
  small.rz  --no-typecheck ran
    1.54 ± 0.12 times faster than medium.rz --no-typecheck
   19.95 ± 0.81 times faster than large.rz  --no-typecheck

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `small.rz  --no-typecheck` | 4.5 ± 0.2 | 4.3 | 4.8 | 1.00 |
| `medium.rz --no-typecheck` | 7.0 ± 0.5 | 6.6 | 7.8 | 1.54 ± 0.12 |
| `large.rz  --no-typecheck` | 90.7 ± 0.8 | 89.6 | 91.5 | 19.95 ± 0.81 |


## cargo check (compiler tree)

Benchmark 1: cargo check (cold)
  Time (mean ± σ):     112.2 ms ±   1.7 ms    [User: 71.9 ms, System: 33.4 ms]
  Range (min … max):   110.1 ms … 114.6 ms    5 runs
 
Benchmark 2: cargo check (warm)
  Time (mean ± σ):     112.4 ms ±   0.5 ms    [User: 72.1 ms, System: 33.4 ms]
  Range (min … max):   111.5 ms … 112.8 ms    5 runs
 
Summary
  cargo check (cold) ran
    1.00 ± 0.02 times faster than cargo check (warm)

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `cargo check (cold)` | 112.2 ± 1.7 | 110.1 | 114.6 | 1.00 |
| `cargo check (warm)` | 112.4 ± 0.5 | 111.5 | 112.8 | 1.00 ± 0.02 |


## cargo build (compiler tree, release)

Benchmark 1: cargo build --release (warm)
  Time (mean ± σ):     112.9 ms ±   1.1 ms    [User: 72.4 ms, System: 33.8 ms]
  Range (min … max):   111.7 ms … 114.2 ms    5 runs
 

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `cargo build --release (warm)` | 112.9 ± 1.1 | 111.7 | 114.2 | 1.00 |


