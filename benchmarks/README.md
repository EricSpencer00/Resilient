# Resilient — In Action and Benchmarks

A first look at the language doing real work, plus head-to-head
timings against five other languages on three workloads.

> **TL;DR.** Resilient is a tree-walking interpreter, not a JIT or a
> compiler — so it's slower than every comparison language on
> CPU-bound work. The interesting story isn't "is Resilient fast?"
> The interesting story is **how much speed you trade for contract
> verification you can ship**, and where the bottlenecks actually live.

After the **RES-050 environment refactor** (Rc<RefCell<...>>),
function-call-heavy workloads got 7-15× faster. fib(25) is now
~400 ms instead of ~2.9 s, and the gap between Resilient and the
rest is mostly closed for tight arithmetic loops.

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
| Rust (native -O) | **1.2 ms** | 1.00× |
| Lua | 7.7 ms | 6.4× |
| Python 3 | 32.7 ms | 27× |
| Node.js | 63.3 ms | 52× |
| Ruby | 74.1 ms | 61× |
| **Resilient** | **397.9 ms** | **328×** |

**Before RES-050: 2,908 ms (1041×). After: 398 ms (328×). ~7.3× faster.**

The remaining ~300× to native is roughly evenly split between:

- AST walking (every fn call re-walks the body)
- Dynamic dispatch on every `Node` variant
- HashMap lookups for identifier resolution

A bytecode VM would close most of the dispatch + walk gap; cranelift
(RES-070) would close most of the rest.

### 2. sum 1..100,000 — while-loop accumulator

100k iterations of `total += i; i += 1`. No fn calls. Tests loop
and arithmetic overhead.

| Language | Mean | Slowdown vs Rust |
|---|---:|---:|
| Rust (native -O) | **2.3 ms** | 1.00× |
| Lua | 4.8 ms | 2.1× |
| Python 3 | 39.0 ms | 17× |
| **Resilient** | **49.6 ms** | **21×** |
| Node.js | 63.9 ms | 27× |
| Ruby | 69.2 ms | 30× |

This workload doesn't touch the fn-call path so the env refactor
didn't move it. Resilient sits in Python's neighborhood, beats
Node and Ruby. The remaining gap to Lua is mostly arithmetic
dispatch through the AST.

### 3. Contract overhead — 100k `requires` checks

Same loop, with and without `requires b != 0` on the called
function.

| Variant | Mean | Overhead |
|---|---:|---:|
| `Resilient (no contract)` | 128.6 ms | baseline |
| `Resilient + requires`     | 155.5 ms | **+21%** (~270 ns / call) |

A static `--audit` showed 100% of these call sites are statically
discharged, which means **the runtime check is provably redundant**
— the optimizer could elide it entirely (RES-068).

The before-refactor numbers were 1,885 ms / 2,049 ms — both improved
~14× from the env refactor. The relative overhead grew from 9% to
21% because the baseline shrank faster than the absolute check cost.
Eliding proven-safe checks (RES-068) would zero this out.

---

## Honest framing

- **Throughput**: Resilient loses to every language with a JIT or AOT
  compiler. As an interpreter the language is in Lua/Python's
  neighborhood for arithmetic and well behind for fn-heavy code.
- **What it's optimized for**: not throughput. It's optimized for
  **provable correctness** of contract clauses. The `--audit` flag
  shows what's been proven; the Z3 backend (`--features z3`) extends
  the proofs to universal cases. None of the comparison languages
  do this.
- **Hot paths still to attack**: every fn call still re-walks the AST
  (no bytecode), every identifier lookup walks the env chain (no
  inline caching), every operator dispatches through a `match` on
  Node variants. RES-068 (elide proven-safe runtime checks) would
  zero out contract overhead. RES-070 (Cranelift backend) would
  close most of the remaining gap to native.

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
