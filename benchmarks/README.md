# Resilient — In Action and Benchmarks

A first look at the language doing real work, plus head-to-head
timings against five other languages on three workloads.

> **TL;DR.** Resilient is a tree-walking interpreter, not a JIT or a
> compiler — so it's slower than every comparison language on
> CPU-bound work. The interesting story isn't "is Resilient fast?"
> (it isn't). The interesting story is **how much speed you trade for
> contract verification you can ship**, and where the bottlenecks
> actually live in a stripped-down embedded-style runtime.

---

## In Action

A 100-line program ([sensor_monitor.rs](../resilient/examples/sensor_monitor.rs))
exercising every Phase 1–4 feature: structs, contracts, `Result` + `?`,
`match`, arrays, `for..in`, string coercion, `live { invariant ... }`.

```bash
$ cargo run -- --audit examples/sensor_monitor.rs
Running type checker...
Type check passed

--- Verification Audit ---
  contract decls (tautologies discharged): 0
  contracted call sites visited:           5
  call-site requires discharged statically: 5 / 5
  call-site requires left for runtime:      0 / 5
  static coverage:                          100%

[LIVE BLOCK] Starting execution of live block
[LIVE BLOCK] Successfully executed live block
low:   2
mid:   1
high:  1
alert: 1
processed: 5
Program executed successfully
```

That `100%` static coverage line is the language doing what its name
promises: every contract a runtime check would have fired for is
already proven at compile time.

---

## Benchmarks

Hardware: Apple M1 Max, macOS arm64. Run `./benchmarks/run.sh` to
reproduce. Each row is hyperfine's mean of 5 runs after 2 warmup
runs.

### 1. fib(25) — recursive Fibonacci

Exponential function calls. Stresses dispatch, environment cloning,
and recursion.

| Language | Mean | Slowdown vs Rust |
|---|---:|---:|
| Rust (native -O) | **2.8 ms** | 1.00× |
| Lua | 7.8 ms | 2.78× |
| Python 3 | 33.5 ms | 12.0× |
| Node.js | 62.8 ms | 22.5× |
| Ruby | 72.5 ms | 26.0× |
| **Resilient** | **2,908 ms** | **~1041×** |

This is the bad case for our interpreter: every fn call clones the
captured environment, allocates a new `Environment` struct, walks
the env chain on every identifier lookup, and pays for the
`apply_function` self-bind clone (the recursion fix shipped in
`c58c4b1`). With 2^25 ≈ 33M calls, the per-call overhead dominates
totally.

The fix is well-understood — `Environment = Rc<RefCell<...>>` would
remove most of the cloning (tracked as RES-050). A bytecode VM (or
Cranelift, RES-070) would close most of the rest of the gap.

### 2. sum 1..100,000 — while-loop accumulator

100k iterations of `total += i; i += 1`. No fn calls. Tests loop
and arithmetic overhead.

| Language | Mean | Slowdown vs Rust |
|---|---:|---:|
| Rust (native -O) | **2.4 ms** | 1.00× |
| Lua | 3.8 ms | 1.58× |
| Python 3 | 33.9 ms | 14.1× |
| **Resilient** | **49.8 ms** | **20.6×** |
| Node.js | 62.6 ms | 25.9× |
| Ruby | 66.6 ms | 27.6× |

**Surprise**: on this workload Resilient beats both Node.js and
Ruby. The loop-and-arithmetic path is tight in our interpreter; the
penalty hits when fn calls multiply.

### 3. Contract overhead — 100k `requires` checks

Same loop, with and without `requires b != 0` on the called
function.

| Variant | Mean | Overhead |
|---|---:|---:|
| `Resilient (no contract)` | 1,885 ms | baseline |
| `Resilient + requires`     | 2,049 ms | **+9%** (~1.5 µs / call) |

A static `--audit` showed 100% of these call sites are statically
discharged, which means **the runtime check is provably redundant**
— the optimizer could elide it entirely. Tracked as RES-068
("optimize-out runtime checks for statically-discharged contracts").
Even today, 9% is a modest tax for a feature that catches design
bugs at compile time.

---

## Honest framing

- **Throughput**: Resilient loses to every language with a JIT or AOT
  compiler. As an interpreter the language is in Lua's neighborhood
  for arithmetic and well behind for fn-heavy code.
- **What it's optimized for**: not throughput. It's optimized for
  **provable correctness** of contract clauses. The `--audit` flag
  shows what's been proven; the Z3 backend (`--features z3`) extends
  the proofs to universal cases. None of the comparison languages do
  this.
- **Hot paths to attack**: env cloning in `apply_function`, no
  bytecode (every call re-walks the AST), no inline caching for
  identifiers. RES-050 (shared env via `Rc<RefCell<>>`) is the next
  obvious win; RES-070 (Cranelift backend) would close most of the
  remaining gap to native.

---

## Reproducing

```bash
brew install hyperfine z3   # macOS prereqs (z3 only if you want --features z3)
cd /path/to/Resilient
./benchmarks/run.sh
```

Output goes to `benchmarks/RESULTS.md`. The `run.sh` script also
ensures the release binary and native Rust baselines are built.

## Files

```
benchmarks/
├── README.md       — this file
├── RESULTS.md      — most recent hyperfine output
├── run.sh          — driver
├── fib/            — recursive Fibonacci, 6 languages
├── sum/            — array sum, 6 languages
└── contracts/      — Resilient-only with/without requires
```
