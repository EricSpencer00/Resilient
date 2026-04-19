---
title: Memory Model
nav_order: 9
permalink: /memory-model
---

# Memory Model
{: .no_toc }

How Resilient represents, owns, and reclaims values across its three
execution tiers.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Scope

Resilient is aimed at safety-critical embedded workloads (automotive,
aerospace, medical). Memory behaviour therefore has to be auditable
end-to-end: a reviewer needs to answer "where does this value live,
who owns it, when is it freed, and can this program allocate under
this feature config?" without tracing through the implementation.

This page documents all three execution tiers and the shared
properties that hold across them.

## Overview: three tiers

Resilient programs can execute on three backends. They share the same
language-level value semantics but differ in their implementation
memory strategy.

| Tier                     | Implementation                               | Allocation strategy                    |
|--------------------------|----------------------------------------------|----------------------------------------|
| Host interpreter         | `resilient/src/main.rs` — tree walker        | `Rc<RefCell<EnvFrame>>` (refcount + interior mutability) |
| Bytecode VM              | `resilient/src/vm.rs` — stack machine        | Owned `Vec<Value>` operand stack + locals slab |
| Cranelift JIT            | `resilient/src/jit_backend.rs`               | Cranelift-managed native stack frames + closure upvalues |
| Embedded runtime         | `resilient-runtime/` (`#![no_std]`)          | Stack-only by default; optional `alloc` feature |

The host tier optimises for iteration speed and closure semantics.
The VM tier keeps allocation out of the inner dispatch loop. The
embedded tier is the one safety auditors usually care about — it is
the configuration shipped onto hardware.

## Value representation

### Host `Value` enum

Defined at `resilient/src/main.rs:4000`. The variants are:

| Variant      | Payload                                          | Backing storage                     |
|--------------|--------------------------------------------------|-------------------------------------|
| `Int(i64)`   | 64-bit signed integer                            | Inline (stack / enum payload)       |
| `Float(f64)` | IEEE-754 double                                  | Inline                              |
| `Bool(bool)` | 1 bit logical                                    | Inline                              |
| `String`     | `std::string::String`                            | Heap (owned `Vec<u8>` in the String) |
| `Array`      | `Vec<Value>`                                     | Heap                                |
| `Struct`     | `Vec<(String, Value)>`                           | Heap (field list)                   |
| `Map`        | `HashMap<MapKey, Value>`                         | Heap                                |
| `Set`        | `HashSet<MapKey>`                                | Heap                                |
| `Bytes`      | `Vec<u8>`                                        | Heap                                |
| `Result`     | `{ ok: bool, payload: Box<Value> }`              | Heap (the payload box)              |
| `Return`     | `Box<Value>` (internal control-flow carrier)     | Heap                                |
| `Function`   | parameters + `Box<Node>` body + `Environment`    | Heap (AST body) + Rc (env)          |
| `Builtin`    | `&'static str` name + native fn pointer          | Inline                              |
| `Closure`    | `{ fn_idx: u16, upvalues: Box<[Value]> }` (VM)   | Heap (upvalue slab)                 |
| `Void`       | unit                                             | Inline                              |

Arithmetic semantics match across every tier: `Int` uses wrapping
i64 ops; `Float` is IEEE-754; mixed-type ops are a `TypeMismatch`
error, never an implicit coercion.

### Embedded runtime `Value` enum

The sibling crate at `resilient-runtime/src/lib.rs` deliberately
carries a narrower surface. Under the default feature set, it
contains only variants whose payload fits on the stack:

- `Int(i64)`
- `Bool(bool)`
- `Float(f64)`

With `--features alloc` enabled, it additionally exposes:

- `String(alloc::string::String)` — only variant that pulls `alloc`.

The enums are deliberately non-unified today. The host `Value`
transitively pulls in `std` (closures carry `Box<Node>`, maps use
`HashMap`); the embedded enum is kept small so users can audit
exactly what memory a deployed binary can touch.

## Ownership and lifetimes

Resilient is a value-semantics language at the source level. Users
do not write references, borrows, or lifetimes — those are
implementation details of the host runtime, not part of the
language.

### Bindings

- `let x = expr;` evaluates `expr`, producing a `Value`, and binds
  it to the name `x` in the current environment frame. The value is
  owned by that binding.
- `let` introduces a new binding; rebinding `x` in the same scope
  with `let` shadows the old one. Mutation of the original slot is
  a separate operation (assignment via `=`), not rebinding.
- Function parameters are bound to the argument values on entry.
  They are locals of the callee's frame and die when the frame
  returns.

### No language-level aliasing

There is no way to spell a reference to another binding. Passing
a value into a function gives the callee a fresh `Value`. In the
host tier the underlying `Rc` may be shared (clone is a refcount
bump), but the user cannot observe this — mutation of heap-backed
values (`Array`, `Map`, `Struct`) is done through the binding that
owns it, and there is no syntax for "take a reference to x".

This matters for auditors: two distinct bindings cannot be proven
to alias at the source level, so reasoning about "did this
function modify that array?" reduces to "did this function receive
that array as an argument or as a captured closure upvalue?"

### Reclamation

- **Host**: `Rc<RefCell<EnvFrame>>` is dropped when the last
  reference goes away. Rust's `Drop` recursively frees the value
  graph. No tracing GC, no stop-the-world pause.
- **VM**: Values are owned in `Vec<Value>` slots (operand stack,
  locals slab). They are freed when their slot is overwritten or
  the frame is popped.
- **Embedded**: Stack-only variants need no reclamation. With
  `--features alloc`, `String` drops through the user-installed
  allocator when the owning binding is freed.

There is no cycle collector. Cycles in the host tier would require
a language-level reference type, which Resilient does not expose.

## The live-block memory contract

A `live { }` block is the language's recoverable-failure primitive.
The host implementation is at `resilient/src/main.rs:6679`
(`eval_live_block`).

### What is snapshotted

On entry to a `live { }` block, the interpreter calls
`self.env.deep_clone()`. That routine:

- Allocates a fresh `RefCell<EnvFrame>` for every frame in the
  scope chain up to the root.
- Copies each frame's `HashMap<String, Value>` by value. `Value`
  itself is `Clone`, so primitive variants copy inline and
  heap-backed variants clone their heap payload.
- Follows the `outer` chain so the entire captured environment is
  independent of the live state.

The snapshot is stashed for the duration of the block and
reinstated with another `deep_clone` on every retry so the first
retry's mutations do not pollute the second.

### What is restored on retry

On a recoverable error or invariant violation, the interpreter:

1. Increments the retry counter.
2. Optionally sleeps per the configured backoff
   (`with backoff(base: ..., factor: ..., max: ...)`).
3. Re-points `self.env` at a fresh deep clone of the snapshot.
4. Re-executes the body.

This means all source-level bindings — including `let`s introduced
inside the live block and mutations to outer-scope variables — are
rolled back. The block sees exactly the environment it saw on its
first attempt.

### What is NOT captured

The snapshot is purely the environment of source-level bindings.
It does not and cannot rewind:

- **I/O effects.** `println`, sink writes, file I/O, register
  writes. These have already left the program's memory boundary by
  the time the retry fires.
- **External system state.** Peripheral registers, DMA buffers,
  sensor readings, network peers.
- **Native allocator state.** If a `String` was allocated during
  the failing attempt and dropped, its slot in the allocator is
  already freed; the retry re-allocates.
- **Process-global counters.** `LIVE_TOTAL_RETRIES` and
  `LIVE_TOTAL_EXHAUSTIONS` are diagnostic counters and intentionally
  persist across retries.

### Retry and timeout budget

Default retry cap is 3 attempts. A `within <duration>` clause
imposes an additional wall-clock budget sampled once at block
entry; retries and backoff sleeps both count against it. Exceeding
either budget fails the block with a diagnostic that carries the
retry depth. See the `eval_live_block` implementation for exact
wording and `LIVE_RETRY_STACK` for how nested blocks compose.

## Embedded memory modes

The `resilient-runtime` crate ships three mutually-consistent
postures. Pick one at build time; they are enforced by feature
flags.

### Default: stack-only

```bash
cargo build --target thumbv7em-none-eabihf
```

The default feature set carries only `Int`, `Bool`, and `Float`.
None of these require an allocator — the payload of each enum
variant fits in the value itself. A binary built with default
features has no heap allocation path in the runtime crate.

### Explicit heap: `--features alloc`

```bash
cargo build --target thumbv7em-none-eabihf --features alloc
```

Enables the `Value::String` variant. Pulls in
`alloc::string::String`, which requires `extern crate alloc` and
therefore a `#[global_allocator]` somewhere in the final binary.
The runtime crate does **not** pick one — the user is responsible
for wiring it. The Cortex-M4F demo uses
[`embedded-alloc::LlffHeap`](https://docs.rs/embedded-alloc) with
a static 4 KiB pool; see [`/no-std`](no-std#wiring-an-allocator-binary-side)
for the canonical pattern.

### Assertion posture: `--features static-only`

```bash
cargo build --target thumbv7em-none-eabihf --features static-only
```

Asserts no-heap intent. The crate emits a `compile_error!` if
`static-only` and `alloc` are both enabled in the same build
graph, catching accidental heap pull-in from transitive
dependencies at link time rather than at runtime. Test coverage
at `lib.rs:306+` verifies that the `String` variant is
exhaustively absent when this flag is active.

### User responsibility

The runtime does not enforce a heap size, does not install an OOM
handler, and does not allocate eagerly. Binary authors choose:

- Which allocator (`LlffHeap`, `TlsfHeap`, a custom bump, ...).
- The backing memory (usually a `static mut [MaybeUninit<u8>; N]`).
- The OOM handler (`#[alloc_error_handler]`).
- Whether to link `alloc` at all.

## Supported embedded targets

Verified via CI cross-compilation:

| Target                          | Class             |
|---------------------------------|-------------------|
| `thumbv7em-none-eabihf`         | Cortex-M4F / M7F  |
| `thumbv6m-none-eabi`            | Cortex-M0+        |
| `riscv32imac-unknown-none-elf`  | RISC-V 32-bit IMAC |

All three build cleanly under both the default and `alloc` feature
sets. Cortex-M4F additionally has a runnable demo at
`resilient-runtime-cortex-m-demo/`.

## Atomics

The runtime crate does not use atomics today. Some embedded targets
(notably `thumbv6m-none-eabi` on Cortex-M0+) lack native
compare-and-swap, and the runtime avoids features that would force
a `compiler_builtins` shim. Tickets RES-141 (host-side retry
counters) and RES-177 (runtime-level sink sequencing) track where
atomic use has been considered and what the platform story will be.

## Comparison to other approaches

### vs Rust ownership

Resilient exposes **value semantics**, not borrow semantics.
The language has no `&T`, no `&mut T`, no lifetimes, and no
borrow checker. The host runtime uses Rust's ownership internally
but the user never names a reference.

Tradeoff: users give up Rust's zero-copy sharing but gain a
simpler mental model — an argument passed to a function is
conceptually a copy, period. For safety auditing this is a
feature: the set of ways two pieces of code can affect the same
value is strictly smaller.

### vs manual C malloc/free

There is no `malloc`, no `free`, and no pointer type in the
language. The runtime frees values deterministically (scope exit
in the VM; Rc drop in the host), so:

- No `use-after-free` — a freed slot is no longer reachable.
- No `double-free` — Rust's drop discipline enforces linearity.
- No dangling pointers — references do not exist at the source
  level.

### vs tracing GC

The host tier uses reference counting, not a tracing GC. There is
no stop-the-world pause, no mark phase, no global heap walk. Drop
work is bounded by the size of the value graph going out of scope
at that moment.

The embedded tier has no GC at all in the default configuration
(nothing to collect — stack-only values). With `--features alloc`,
reclamation is still deterministic drop-on-scope-exit through the
user-installed allocator.

## Safety properties

Properties that hold across all three tiers unless noted.

- **No null.** The `Value` enum has no null variant. `Void` is a
  unit value, not a null pointer. An absent result is either a
  `Value::Result { ok: false, ... }` or a live-block error.
- **No use-after-free.** Bindings own their values. Once a
  binding's scope ends its value is dropped; there is no syntax
  that yields a handle to a dropped value.
- **No dangling pointers.** Pointers are not exposed to the
  language. The host tier's `Rc` prevents the underlying frame
  from being freed while any clone exists.
- **Bounded call depth.** The VM enforces `MAX_CALL_DEPTH = 1024`
  frames and returns `VmError::CallStackOverflow` past that. The
  host interpreter inherits the native Rust stack's limit
  (typically 8 MiB) and will overflow with a Rust panic on deeper
  recursion — this is a known gap and tracked for parity.
- **Bounded live-block retries.** Default cap 3 attempts, plus an
  optional `within <duration>` wall-clock cap.
- **Provably heap-free builds.** With `--features static-only` on
  `resilient-runtime`, no runtime code path can reach an allocator.
  The `compile_error!` guard prevents the `alloc` feature from
  sneaking in via a transitive dependency.
- **Deterministic reclamation.** Every tier reclaims memory at a
  statically predictable point (scope exit, `ReturnFromCall`, or
  `Rc` refcount reaching zero). No background collector, no
  unpredictable pause.

## Cross-references

- [`/no-std`](no-std) — feature flags, cross-compile targets,
  allocator wiring walkthrough.
- [`/philosophy`](philosophy) — the resilience / verifiability /
  simplicity pillars that the memory model is designed to serve.
- [`/syntax`](syntax) — source-level syntax for `live`, `let`,
  bindings, and structs.
