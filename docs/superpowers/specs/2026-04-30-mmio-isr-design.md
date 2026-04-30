# Volatile MMIO + Interrupt-Handler ABI

**Date:** 2026-04-30
**Status:** Design lock-in for [#369 RES-406](https://github.com/EricSpencer00/Resilient/issues/369) (`needs-design before implementation`)
**Tracking:** RES-406
**Cross-cuts:** RES-072 (Cranelift JIT — eventually lowers volatile), RES-407 (live-block runtime — interacts with ISR safety),
[`resilient-runtime-cortex-m-demo/`](../../../resilient-runtime-cortex-m-demo/) (the existing cross-compile target)

---

## Why this document exists

[#369](https://github.com/EricSpencer00/Resilient/issues/369)
is tagged `needs-design` precisely so the implementation-side
acceptance criteria don't drift while the ABI is debated mid-PR.
The ticket calls out three open design questions:

1. The volatile-intrinsic surface — generic `volatile_read<T>`
   vs. fixed-width `volatile_read_u32` etc.
2. The `unsafe { … }` block — how it interacts with existing
   contract / verification surfaces.
3. The `#[interrupt(name = "…")]` attribute ABI — how the
   compiler emits a vector-table entry that the linker
   resolves correctly across Cortex-M / RISC-V / RV32IMAC.

This document gives each a recommendation + tradeoffs, and
folds the answers back into [#369](https://github.com/EricSpencer00/Resilient/issues/369)'s
acceptance criteria so implementation can start without further
design rounds.

---

## Q1. Volatile intrinsic shape

### Question

> `volatile_read<T>(addr: usize) -> T` and `volatile_write<T>(addr: usize, v: T)`
> intrinsics for `T ∈ {u8, u16, u32, u64}`.

The ticket's acceptance criterion already names a generic
shape, but the language doesn't have generics yet
([#368 RES-405](https://github.com/EricSpencer00/Resilient/issues/368)
is still open). What ships in V1?

### Recommendation: **fixed-width family for V1; generic facade lands when [#368 RES-405](https://github.com/EricSpencer00/Resilient/issues/368) does**

Ship four pairs of fixed-width intrinsics:

```rz
volatile_read_u8(addr: int) -> int
volatile_read_u16(addr: int) -> int
volatile_read_u32(addr: int) -> int
volatile_read_u64(addr: int) -> int
volatile_write_u8(addr: int, v: int)
volatile_write_u16(addr: int, v: int)
volatile_write_u32(addr: int, v: int)
volatile_write_u64(addr: int, v: int)
```

Address is `int` (alias for `i64`); the runtime checks at the
boundary that the address fits in `usize`. Value is `int` for
read/write — caller is responsible for the unsigned-widening
narrative; the volatile read returns the bit pattern, the
volatile write reads the low N bits.

When [#368 RES-405](https://github.com/EricSpencer00/Resilient/issues/368)
(generic monomorphisation) lands, the eight intrinsics get
consolidated behind a single generic `volatile_read<T>` /
`volatile_write<T>` facade as a follow-up. The fixed-width
names stay around as `pub use` aliases so existing code
continues to compile.

### Tradeoffs

| Option | Pro | Con |
|---|---|---|
| Generic now (`volatile_read<T>`) | Cleaner surface, matches the ticket text verbatim | Blocks on [#368](https://github.com/EricSpencer00/Resilient/issues/368) (which is also complex and not yet started); adds an inter-ticket dependency that pushes V1 ship-date further |
| Fixed-width family (recommended) | Implementable today; narrow surface — eight intrinsics, no type-level machinery; easy to audit | Eight intrinsics instead of two; users have to pick the right one |
| Single `volatile_read_word(addr)` | Smallest surface | Hides the access-width choice — users with a 16-bit register write 32 bits; bug factory |

### Why fixed-width wins

The fixed-width family is implementable in V1 today and ships
the embedded value (`-O the LED can be toggled`) without
waiting on the generics work. The "eight intrinsics is too
many" complaint vanishes once `volatile_read<T>` exists as a
facade — the eight names become an implementation detail
under a generic surface. Until then, eight named intrinsics
is a fair cost for unblocking embedded users.

### V1 acceptance criteria absorbed

- Eight intrinsics in `resilient-runtime/src/mmio.rs` (new
  module). All `pub` from the runtime; all `#[inline(always)]`
  so the compiler can lower them to a single load/store
  instruction in release builds.
- Compiler / interpreter routes calls through these names; the
  walker uses `core::ptr::read_volatile` / `write_volatile`
  directly. The bytecode VM emits a runtime-only path against
  a fault-injection test fixture (per the ticket's note); JIT
  lowering is a [#0](https://github.com/EricSpencer00/Resilient/issues/72)
  follow-up.
- The fixed-width naming convention extends mechanically to
  more widths (`u128` if anyone needs it) without an
  ABI break.

---

## Q2. `unsafe { … }` block — semantics + verification interaction

### Question

> `unsafe { ... }` block syntax required for any volatile access — no
> silent privilege grants. Outside `unsafe`, calling a volatile
> intrinsic is a compile-time error.

What does `unsafe` mean for the rest of the system —
specifically the `requires`/`ensures` contract surface and the
Z3 verifier?

### Recommendation: **`unsafe` is a compile-time gate only; contracts and verification continue to work inside it**

The `unsafe { … }` block is a **lexical capability gate**: it
authorizes the caller to invoke `volatile_*` intrinsics (and,
later, FFI / raw-pointer operations). It is not a verification
escape hatch — `requires` / `ensures` clauses on the enclosing
function are still proven; assertions inside the block are
still checked; live-block recovery still applies; the type
checker still type-checks.

Concretely:

- Calling a `volatile_*` intrinsic outside an `unsafe` block is
  a compile-time error (`error: volatile_read_u32 requires an
  unsafe block`).
- `unsafe { … }` makes that error go away; everything else is
  unchanged.
- The Z3 verifier treats volatile reads as nondeterministic
  (return value is `CHOOSE x \in T : true`); volatile writes
  are no-ops at the Z3 level (they don't constrain user-visible
  state). This matches the V1 verification model — Z3 reasons
  about pure functions; MMIO is opaque.

### Tradeoffs

| Option | Pro | Con |
|---|---|---|
| `unsafe` disables verification | Matches Rust semantics; users who write `unsafe` know to not expect proofs | Most volatile usage is brief — one register read inside an otherwise-pure function; disabling verification for the whole function is too coarse |
| `unsafe` is a capability gate (recommended) | Verification still works for the surrounding code; only the volatile primitive itself is opaque to the verifier | Users coming from Rust expect `unsafe` to disable more than it does — needs documentation |
| `unsafe` doesn't exist; volatile is a regular function | Smallest surface | Loses the "no silent privilege grants" goal from the ticket; volatile usage becomes invisible at call sites |

### Why capability-gate wins

Two reasons:

1. **The ticket asks for capability semantics, not Rust
   semantics.** The acceptance criterion says "no silent
   privilege grants". That's a capability requirement (the
   syntax flags the use), not a verification-disable
   requirement. Implementing the broader Rust meaning would
   over-deliver and complicate the verifier's job.
2. **Z3 already handles "opaque" effects.** Volatile reads /
   writes are no different from FFI calls in this respect (see
   the [TLA+ V2.0 design lock-in's Q4](2026-04-30-tla-v2-design-lock-in.md#q4-ffi-side-effects--choose-or-contract)).
   The verifier already has a "treat this as nondeterministic"
   path; volatile reuses it.

### V1 acceptance criteria absorbed

- New `Token::Unsafe` keyword + parser entry for `unsafe {
  block }`. Same shape as a regular block; just gated for
  privileged calls.
- Typechecker pass: every call to a `pub(crate)`
  `unsafe`-marked builtin (the eight volatile intrinsics
  initially; FFI later) walks up the AST looking for an
  enclosing `unsafe { }`. Without one: compile-time error
  with file:line:col and the suggested fix.
- Z3 verifier: volatile reads and writes do not constrain or
  read symbolic state. Existing nondeterministic-call handling
  (the same path FFI uses) covers this.
- `requires` / `ensures` on a function containing `unsafe { }`
  are unchanged — the contracts apply to the function as a
  whole, not just the safe parts.

---

## Q3. `#[interrupt(name = "…")]` ABI

### Question

> `#[interrupt(name = "TIM1_UP")] fn handler() { ... }` attribute that
> registers an ISR with the embedded runtime's vector table at link time.

How does the compiler emit a vector-table entry that the
linker resolves correctly across Cortex-M, RISC-V, and
RV32IMAC?

### Recommendation: **emit a `__resilient_isr_<NAME>` extern symbol; runtime crate ships per-target weak vector-table aliases**

The compiler lowers `#[interrupt(name = "TIM1_UP")] fn handler() { ... }`
to:

```rust
#[unsafe(no_mangle)]
pub extern "C" fn __resilient_isr_TIM1_UP() {
    // body of `handler`
}
```

The `resilient-runtime-cortex-m-demo/` (and equivalents for
RISC-V / RV32IMAC) ship a vector table whose entries are
**weak aliases** to `__resilient_isr_<NAME>` symbols. If the
user's program defines `__resilient_isr_TIM1_UP`, the linker
binds the vector entry to it; otherwise the weak symbol
resolves to a default `unhandled_interrupt` stub.

This pattern is what `cortex-m-rt` already uses for Cortex-M;
RISC-V's `riscv-rt` uses an equivalent. We're not inventing a
new mechanism — we're adopting the existing one.

### Tradeoffs

| Option | Pro | Con |
|---|---|---|
| Per-target inline asm `#[link_section = ".vector_table"]` | Fully under our control | Different syntax per target; we'd own the vector-table layout for every chip family forever |
| Weak-alias to `__resilient_isr_<NAME>` (recommended) | Reuses cortex-m-rt's / riscv-rt's existing infrastructure; we don't own the vector layout, the runtime crate (which is target-specific) does | Adds a dependency contract: the runtime crate must define the weak aliases for every interrupt name we want to support |
| `#[interrupt]` becomes a regular function with no special handling | Simplest compiler-side | Doesn't actually wire anything into the vector table; users have to write the `#[unsafe(no_mangle)]` themselves; defeats the syntactic-help purpose |

### Why weak-alias wins

The weak-alias pattern is the de facto standard for embedded
Rust and is reused by every chip-vendor crate. By adopting it
we:

- Don't have to maintain per-target vector layouts.
- Get free interoperability with users who pull in chip
  crates that already define the weak aliases.
- Get a clean layering: the language compiler emits the
  symbol, the runtime crate (per target) defines the table
  that resolves it, the linker connects them. Each layer is
  separately testable.

### V1 acceptance criteria absorbed

- Parser: `#[interrupt(name = "STRING_LITERAL")]` attribute on
  a `fn` with no parameters and no return type. Other
  attribute shapes (e.g., `#[interrupt(priority = 3)]`) are
  errors with a forward-pointing diagnostic — V1 only handles
  the name.
- Lowering: the function body is emitted under the mangled
  symbol `__resilient_isr_<NAME>` with `#[unsafe(no_mangle)]`
  + `pub extern "C"` so the linker can find it. The original
  function name stays as a `pub use` alias so the user can
  also call it directly (handy for unit testing the body
  without simulating an interrupt).
- Runtime: `resilient-runtime-cortex-m-demo` (and equivalents)
  define weak aliases for the chip's interrupt names. The
  initial set is what the demo program actually uses
  (SysTick, TIM1_UP for the LED-blink + tick-count examples
  the ticket calls for); extending to a full vendor chip's
  interrupts is a follow-up.
- Effect tracking: ISR handlers carry a special `isr` effect
  that prevents calling them from non-ISR code (calling an ISR
  handler directly from `main` is almost certainly a bug).
  The effect is automatically inferred from the
  `#[interrupt(...)]` attribute; no user annotation required.
- Size budget: the ISR + the weak-table additions stay under
  the 64 KiB `.text` budget for the Cortex-M4F demo. The
  cortex-m-rt-style weak table is ~512 bytes for a typical
  STM32 chip; we have plenty of headroom.

---

## Sign-off summary

| # | Question | Recommendation | Risk if wrong |
|---|---|---|---|
| Q1 | Volatile intrinsic shape | Fixed-width family for V1; generic facade follows [#368](https://github.com/EricSpencer00/Resilient/issues/368) | Low — adding the generic facade later is purely additive |
| Q2 | `unsafe` semantics | Capability gate only; verification continues to work | Medium — flipping to "verification escape hatch" later would break programs that rely on contracts holding through `unsafe` |
| Q3 | ISR ABI | `__resilient_isr_<NAME>` weak-alias pattern | Low — matches existing embedded-Rust convention; future chip-vendor crates work out of the box |

Q2 is the highest-stakes call: the verification interaction
is binding for every program that uses `unsafe`.

---

## What this spec does NOT decide

- Specific cross-target chip support beyond the Cortex-M4F
  demo. RISC-V / RV32IMAC vector tables are mechanically
  similar but each ships separately as the corresponding
  runtime crate matures.
- The semantics of `unsafe` for FFI calls vs raw pointer
  dereferencing. This spec only covers the volatile case;
  FFI's `unsafe` story is in the
  [TLA+ V2.0 design lock-in](2026-04-30-tla-v2-design-lock-in.md#q4-ffi-side-effects--choose-or-contract)
  (contract-required); raw pointers are out of scope until
  Resilient grows them as a first-class concept.
- A chip-vendor extension mechanism — i.e., how the user
  defines a custom weak alias for a chip not shipped in the
  runtime. V1 covers the demo's chips; the extension story
  is a V2.x follow-up.
- Whether the compiler should warn about `unsafe` blocks that
  contain only safe code. Stylistic; tracked separately.

---

## V1 implementation order (informational)

The V1 implementation work for [#369](https://github.com/EricSpencer00/Resilient/issues/369)
naturally splits into roughly five PRs, in this order:

1. **PR 1**: `unsafe` keyword + parser + AST node + typechecker
   gate. No runtime impact yet — just enforces the
   capability check (statically rejects calls to functions
   that don't exist yet, so the test set is small).
2. **PR 2**: the eight `volatile_read_*` / `volatile_write_*`
   intrinsics in `resilient-runtime/src/mmio.rs`; interpreter
   + bytecode VM dispatch; runtime-only path against the
   fault-injection fixture. Now `unsafe { volatile_read_u32(0x40021000) }` actually does something.
3. **PR 3**: `#[interrupt(name = "…")]` parser + AST node +
   lowering to the mangled extern symbol.
4. **PR 4**: weak-alias vector table in
   `resilient-runtime-cortex-m-demo`; SysTick + TIM1_UP
   handler examples; `.text` budget passes.
5. **PR 5**: documentation update (SYNTAX.md gets `unsafe` and
   `#[interrupt]`; STABILITY.md gets entries for both;
   `docs/no-std.md` gets a "Hello, GPIO" example walking
   through the LED-blink demo).

This is informational — the maintainer can re-shape, but the
ordering reflects the dependencies between pieces (parser
before lowering before runtime).
