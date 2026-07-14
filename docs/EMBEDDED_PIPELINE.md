---
title: AOT / Bytecode-on-Device Pipeline
parent: Design Philosophy
nav_order: 7
permalink: /embedded-pipeline
---

# AOT / Bytecode-on-Device Pipeline Design
{: .no_toc }

D-E1 — the design doc that unblocks the implementation PR sequence.
No source changes ship with this doc.
{: .fs-6 .fw-300 }

**Status (decomposition in section 5): items 1-4 done — the 1.0
embedded gate (a `.rz` program compiles to and runs on an embedded
target under CI) is now met for the supported scalar subset.** #4031
shipped the no_std `Instr`/`Vm` skeleton, #4034 shipped the `.rzbc`
encoder/decoder (`resilient_runtime::vm::serde`), and the `rz build
--target <TRIPLE>` subcommand (`resilient/src/rzbc_emit.rs` +
`lib.rs`'s `dispatch_build_subcommand`) now closes the loop end to
end for the Int/Bool/Float arithmetic/comparison/control-flow/locals
subset — see section 3.1's "Proposed" column, now real. #4042 added
the thin loader binary section 3.3 sketches
(`resilient-runtime-loader-demo/`, embedding the committed
`arithmetic_demo.rzbc` fixture), and the `embedded-runtime.yml` CI job
(section 4, item 4) now actually runs that binary under
`qemu-system-arm`'s `lm3s6965evb` machine and asserts on both its
semihosting output and QEMU's process exit status. Item 5 (the
RISC-V QEMU variant) and items 6-7 (`Value::Array`, interrupt
lowering) remain open follow-ups.

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## The gap, stated plainly

Resilient's embedded pitch is "write Resilient, run it on a Cortex-M or
RISC-V MCU." Today that pitch is **not backed by a pipeline**. Three
things exist that look adjacent to it, and none of them close the gap:

1. **`--target TRIPLE`** (`resilient/src/lib.rs`, flag parsing around
   line 33679) only sets a string consumed by
   `#[cfg(target = "...")]` predicates at parse time. The code comment
   says it outright:

   > "Pure metadata — the compiler doesn't currently cross-compile from
   > this flag, but it lets a hosted developer simulate an embedded
   > build's cfg-strip behaviour."

   Running `rz --target thumbv7em-none-eabihf foo.rz` (no `build`
   subcommand) today type-checks and *interprets `foo.rz` on the
   host* with a different `#[cfg]` view — that specific flag-only
   invocation is unchanged by this document's implementation. The
   new `rz build --target thumbv7em-none-eabihf foo.rz` **subcommand**
   (section 3, item 3 of the decomposition below) is a different code
   path that does close this gap for the supported subset: it
   compiles, validates, and emits a real `.rzbc` blob instead of
   interpreting on the host.
2. **`resilient-runtime-cortex-m-demo/`** is a hand-written Rust crate
   that links `resilient-runtime` (the sibling `#![no_std]` value-layer
   crate) with `embedded-alloc`, builds one `Value::String` and one
   `Value::Float`, calls `.add()` / `.eq()` on them, and spins forever.
   Its own README says this directly: *"The goal is onboarding
   evidence... not a runnable demo... We deliberately do **not** run
   the output under QEMU in CI."* It proves `resilient-runtime` links
   on a Cortex-M4F target. It proves nothing about compiling a `.rz`
   source file.
3. **The bytecode VM** (`resilient/src/vm.rs`, `resilient/src/bytecode.rs`)
   is the thing that actually executes compiled Resilient programs
   fastest on the host, and it is the natural AOT target — but its
   `Op` dispatch loop is written directly against the **host** `Value`
   enum (`use crate::Value;` at the top of `vm.rs`), which lives in
   `resilient/src/lib.rs` and is `std`-only: `Value::String(String)`,
   `Value::Array(Vec<Value>)`, `Value::Struct { name: String, fields:
   Vec<(String, Value)> }`, `Value::Map(HashMap<...>)`,
   `Value::Set(HashSet<...>)`, boxed `Result`/`Option`/`Closure`
   payloads, and an `Arc<ForeignSymbol>` for FFI. None of that
   compiles under `#![no_std]`. The VM crate is not in
   `resilient-runtime`'s dependency graph and never appears in any
   embedded CI job (`embedded.yml`, `size_gate.yml`) — only
   `resilient-runtime` and `resilient-runtime-cortex-m-demo` do.

So: **there is no code path from a `.rz` file to a binary that runs
on an embedded target.** `resilient-runtime/docs/no-std.md`'s own
roadmap section names this as unstarted future work ("port a subset
of the bytecode VM into `resilient-runtime` so embedded programs can
run pre-compiled bytecode without a host toolchain"). This document is
the design for doing that.

---

## 1. Opcode portability audit

`resilient/src/bytecode.rs` defines `Op` (the VM's instruction set,
`#[derive(Debug, Clone, Copy, PartialEq)]`, 54 variants as of this
writing) and `Chunk` / `Program` (the containers around it). The
question this section answers: **which opcodes could dispatch inside
a `#![no_std]`, alloc-free loop today, and which are inherently tied
to a heap-bearing `Value` variant?**

Classification key:
- **(a) no_std-clean** — the opcode's semantics only ever touch
  `Int`/`Bool`/`Float` operands and flat index/offset data. No
  variant it reads or produces requires `String`, `Vec`, `HashMap`,
  `HashSet`, or `Box<Value>`.
- **(b) alloc-required** — the opcode's defined semantics construct,
  destructure, or index a heap-bearing `Value` variant (`String`,
  `Array`, `Struct`, `Map`, `Set`, `Closure`, boxed `Result`/`Option`,
  or an `Arc`-held FFI symbol).

| Opcode | Class | Why |
|---|---|---|
| `Add`, `Sub`, `Mul`, `Div`, `Mod`, `Neg` | (a) | i64/f64 arithmetic only (per doc comments in `bytecode.rs`) |
| `LoadLocal`, `StoreLocal` | (a) | Index into a flat locals slab |
| `LoadGlobal`, `StoreGlobal` | (a) | Same slab access, frame 0 |
| `Jump`, `JumpIfFalse`, `JumpIfTrue` | (a) | Relative PC arithmetic; truthiness on `Bool`/`Int`/`Float` needs no heap read for the numeric case (the truthy check *also* covers non-empty `String`/compound types, but a no_std VM restricted to scalar `Value`s never hits that branch) |
| `IncLocal` | (a) | In-place `Int` increment |
| `Eq`, `Neq`, `Lt`, `Le`, `Gt`, `Ge`, `Not` | (a) | Scalar comparison/boolean negation |
| `Call`, `ReturnFromCall`, `TailCall` | (a) | Function-table index + fixed-size `CallFrame` push/pop; no heap needed if the call stack is a bounded array |
| `Return` | (a) | Halts the loop, returns TOS |
| `Band`, `Bor`, `Bxor`, `Shl`, `Shr` | (a) | Integer bitwise ops |
| `AssertBool` | (a) | Type-tag check on TOS, no payload extraction |
| `Const(u16)` | **conditional** | No_std-clean when the constant pool entry is `Int`/`Float`/`Bool`/`Void`; alloc-required the moment a chunk contains a `Value::String` constant (string literals, format templates) |
| `MakeArray`, `LoadIndex`, `LoadIndexUnchecked`, `StoreIndex` | (b) | Construct/read/write `Value::Array(Vec<Value>)` |
| `MakeTuple` | (b) | Constructs `Value::Tuple`-shaped `Vec<Value>` |
| `StructLiteral`, `GetField`, `SetField` | (b) | `Value::Struct { name: String, fields: Vec<(String, Value)> }` |
| `MakeEnumTuple`, `MakeEnumNamed` | (b) | Same struct-shaped payload plus `String` type/variant tags |
| `MakeClosure`, `LoadUpvalue`, `StoreUpvalue`, `CallClosure` | (b) | `Value::Closure { upvalues: Box<[Value]>, source_slots: Box<[u16]> }` — boxed slices. (Also: the VM dispatch for these is still `Unsupported` even on the host — see `vm.rs` doc comments on `MakeClosure`/`LoadUpvalue`. They are not yet a "just port it" item; they need host semantics finished first.) |
| `CallMethod` | (b) | Resolves `{struct_name}${method}` — needs `Value::Struct`'s `String` name and a function-name lookup |
| `CallForeign` | (b) *today*, portable path exists | Host FFI holds `Arc<ForeignSymbol>` (heap). `resilient-runtime` already ships a **heap-free alternative** shape for this exact problem: `ffi_static.rs`'s fixed-capacity registry (`ffi-static-64`/`-256`/`-1024` features). A no_std VM would need to re-target `CallForeign` at that registry, not at the host's `Arc`-based one. |
| `CallBuiltin` | (b) | `name_const` indexes a `Value::String` constant, and most registered builtins operate on `String`/`Array`/`Map` inputs. A no_std subset (e.g. `abs`, integer/float math) is plausible but the *dispatch mechanism* (name-string lookup) itself assumes a string constant pool entry. |
| `AssertFail` | (b) | Pops a `Value::String` failure message |
| `TryUnwrap` | (b) | Unwraps `Box<Value>` inside `Result`/`Option` |
| `IterPrepare` | (b) | Normalizes `Array`/`Map` for `for-in`; both heap types |
| `EnterTry`, `ExitTry` | (b) | `TryHandlerEntry`/`CatchArm` carry `variant: String` |

**Tally: 31 opcodes are (a) no_std-clean, 22 are (b) alloc-required,
and 1 (`Const`) is conditional on what's in the constant pool.**
That is a genuinely usable subset — every arithmetic, comparison,
control-flow, and local/global-slot opcode in the language is
alloc-free — but it excludes strings, arrays, structs, maps, sets,
closures, and enum payloads. A "port the VM to no_std" ticket that
doesn't say this up front will over-promise.

---

## 2. Minimal no_std VM design

### 2.1 What the dispatch loop needs

`vm.rs::run_inner` today needs, per the existing `std` implementation:
an operand stack (`Vec<Value>`), a locals slab (`Vec<Value>`), a
`Vec<CallFrame>` call stack, and a `Vec<TryHandlerFrame>` handler
stack — all growable. A no_std port replaces every one of those with
a **fixed-capacity array plus a length cursor**, sized at compile
time or via a `const N: usize` generic parameter on the VM struct:

```rust
#![no_std]

pub struct NoStdVm<const STACK: usize, const LOCALS: usize, const FRAMES: usize> {
    operand_stack: [Value; STACK],
    op_sp: usize,
    locals: [Value; LOCALS],
    frames: [CallFrame; FRAMES],
    frame_sp: usize,
}
```

Overflowing any of these bounds must be a `VmError` return, never a
panic (per this repo's no-panic rule for `resilient-runtime/`) — this
mirrors the host VM's existing `CallStackOverflow` variant, just with
a static cap instead of a `Vec` growing until the OS kills the
process.

### 2.2 Which `Value` variants are heap-free

`resilient-runtime::Value` (in `resilient-runtime/src/lib.rs`) is
*already* the right starting shape: `Int(i64)`, `Bool(bool)`,
`Float(f64)` unconditionally, and `String` gated behind
`#[cfg(feature = "alloc")]`. It does not yet have `Array`, `Struct`,
`Map`, `Set`, `Closure`, or boxed `Result`/`Option` in *any* posture —
those are host-only today. That is a feature, not a gap to backfill
blindly: section 1's audit says a no_std VM can be useful long before
those land, because the (a)-class opcodes cover a real subset of
programs (arithmetic, control flow, local functions — no collections,
no structs).

### 2.3 The `vm` feature on `resilient-runtime`

Following the crate's existing feature-gate discipline (`alloc`,
`static-only`, `std-sink`, `ffi-static-*` in
`resilient-runtime/Cargo.toml`), the port lands as a new **opt-in,
additive** feature:

```toml
[features]
vm = []                    # dispatch loop over Int/Bool/Float-only chunks
vm-alloc = ["vm", "alloc"]  # + String constants, once alloc is on anyway
```

- `--features vm` alone: compiles a chunk containing only (a)-class
  opcodes (per section 1) and `Const` entries restricted to
  `Int`/`Float`/`Bool`/`Void`. Any `Const` referencing a `String`, or
  any (b)-class opcode, is a **build-time or load-time rejection**,
  not a runtime panic — the loader validates the chunk before handing
  it to the dispatch loop (see section 3.2).
- `--features vm-alloc`: same dispatch loop, plus `Const(String)` and
  whichever (b)-class opcodes get ported in follow-up tickets, each
  gated on `resilient-runtime` growing the corresponding `Value`
  variant (`Array` first — it's the highest-leverage collection type;
  `Struct`/`Map`/`Set`/`Closure` follow in the order real example
  programs demand them).
- Mutual exclusion with `static-only` follows the same pattern as
  `alloc`/`static-only` today: a `compile_error!` if both `vm-alloc`
  and `static-only` are set, since `vm-alloc` implies heap use.

This keeps the crate's core promise intact: the *default* feature set
stays exactly what it is today (alloc-free value ops), and everything
in this section is opt-in and additive, matching the "Feature configs"
table already published in `docs/no-std.md`.

---

## 3. Pipeline design: `rz build --target <TRIPLE>`

**Status: shipped for the Int/Bool/Float scalar subset** (decomposition
item 3, this document's section 5). `resilient/src/rzbc_emit.rs` maps
`compiler::compile`'s `Op` stream onto `resilient_runtime::vm::Instr`
and serializes with `resilient_runtime::vm::serde::encode` — the
"Proposed" column below is what actually ships, with two adjustments
from the original sketch: the `.rzbc` format that shipped in #4034 has
no separate constant pool or function table (every constant is inlined
directly onto `Instr::PushConst`, and there is no `vm_profile` header
field), and there is no "scaffold a loader crate" step yet (section 3.3
remains a follow-up) — `rz build` emits the `.rzbc` blob only.

### 3.1 Today vs. proposed

| | Before this document's implementation PRs | Now |
|---|---|---|
| `rz build --target thumbv7em-none-eabihf foo.rz` | `build` was not a recognized subcommand, so the generic flag parser treated it as a throwaway positional argument (overwritten by the later `foo.rz` positional) — the command silently **type-checked + interpreted `foo.rz` on the host**, identical to plain `rz --target thumbv7em-none-eabihf foo.rz` below, with `--target` only flipping `#[cfg]` predicates. No artifact, and the leading `build` word was pure noise. | `build` is now a real subcommand (`dispatch_build_subcommand` in `lib.rs`, intercepted before the generic flag loop). It compiles `foo.rz` to a `Program` (via the existing `compiler.rs` → `bytecode.rs` path), rejects any construct outside the no_std-clean subset (section 1) with a clear diagnostic, and serializes the result to a `.rzbc` blob (`resilient_runtime::vm::serde`'s wire format) written to `-o <path>` (default: `foo.rzbc`). No loader-crate scaffolding yet (section 3.3 is still a follow-up). |
| `rz --target thumbv7em-none-eabihf foo.rz` (no `build` subcommand) | Type-checks + **interprets on host** with `#[cfg(target="thumbv7em-none-eabihf")]` predicates active. No artifact. | Unchanged — this flag-only invocation still only flips `#[cfg]` predicates for a host-interpreted run; it is a separate code path from the new `build` subcommand. |
| Consumer of `--target` | Only `#[cfg(target = "...")]` conditional-compilation predicates inside the `.rz` source itself | Same predicates (unchanged, still useful for source-level portability), *plus* it now also selects the opcode/constant validation profile and the loader template. |

`target_profiles.rs` (`resilient/src/target_profiles.rs`, RES-2614)
already parses `[target.TRIPLE]` sections from `rz.toml` — `features`,
`opt_level`, `stack_size`, `cfg`. This is the natural place to add a
`vm_profile` field (`"host"` | `"no_std-scalar"` | `"no_std-alloc"`)
that the build subcommand reads to pick the validation profile and
loader template. No new manifest syntax is needed — just a new
recognized key in an existing table-driven parser.

### 3.2 Artifact format

A compiled `Program` (from `bytecode.rs`) serializes to a flat,
versioned binary blob:

```
[4]  magic: b"RZBC"
[2]  format_version: u16
[2]  vm_profile: u16          (0 = no_std-scalar, 1 = no_std-alloc, ...)
[4]  const_pool_len: u32
[N]  const_pool: tagged (tag: u8, payload) entries
       tag 0 = Int(i64)   → 8 bytes
       tag 1 = Float(f64) → 8 bytes
       tag 2 = Bool(bool) → 1 byte
       tag 3 = Void       → 0 bytes
       tag 4 = String     → u32 len + UTF-8 bytes (vm_profile >= 1 only)
[4]  main_chunk_len: u32
[N]  main_chunk: Op stream, fixed-width encoding (each Op discriminant
                  + operands packed to a constant width — mirrors the
                  in-memory `Op` enum's `Copy`, ≤8-byte-per-variant
                  discipline already documented in `bytecode.rs`)
[4]  function_table_len: u32
[N]  function_table: [ (name_len: u32, name: bytes, arity: u8,
                         local_count: u16, chunk_len: u32, chunk: ...) ]
```

Emitting a *fixed-width* opcode encoding (rather than a variable-length
bytecode like CPython's) keeps the no_std loader dead simple: no
variable-length instruction decode logic, no risk of a malformed
length prefix walking off the end of flash. The cost is a slightly
larger blob than a packed encoding would produce — acceptable given
the 64 KiB `.text` budget this repo already enforces is a *code* size
budget (see `size_gate.yml`), not a flash-image-size budget.

### 3.3 Loader responsibilities

The "thin no_std loader template" is a small `#![no_std]` `#![no_main]`
binary crate (structured like
`resilient-runtime-cortex-m-demo/src/main.rs` today) that:

1. Embeds the `RZBC` blob as a `static` byte array (via
   `include_bytes!` at build time — the blob is produced by `rz build`
   and the loader crate is a template `rz build` scaffolds, analogous
   to how `cargo new` scaffolds a `Cargo.toml`).
2. Validates the magic + format_version + vm_profile header before
   doing anything else (reject a mismatched profile at boot rather
   than mis-decoding opcodes).
3. Constructs a `resilient_runtime::vm::NoStdVm<STACK, LOCALS, FRAMES>`
   (section 2.1) sized from the manifest's `stack_size` /
   a new `vm_locals_cap` / `vm_frames_cap` target-profile fields.
4. Wires whatever `#[global_allocator]` the `vm-alloc` profile needs
   (same `embedded-alloc::LlffHeap` pattern `docs/no-std.md` already
   documents for the binary side) — skipped entirely for
   `no_std-scalar` profiles, keeping the true zero-alloc case
   allocator-free end to end.
5. Runs the VM to completion or error, and reports the result via
   whatever the target's semihosting/UART/telemetry sink is (see
   section 4 for the QEMU case, which uses semihosting).

`rz build --target <TRIPLE>` therefore produces **two** files: the
`.rzbc` blob and a scaffolded (or previously-scaffolded, then
re-linked) loader crate directory — not a single self-contained
executable. This mirrors how the language's own docs already describe
the layering: "the host build (`resilient/`) uses the full interpreter
/ VM / JIT; the embedded build uses just this runtime crate plus a
future `Program` evaluator" (`docs/no-std.md`, "What it is" section).

---

## 4. QEMU CI plan

**Status: item 2 (Cortex-M path) shipped.** `embedded-runtime.yml`
(new workflow, `qemu_cortex_m` job) builds
`resilient-runtime-loader-demo` for `thumbv7em-none-eabihf` via the
existing `scripts/build_loader_demo.sh`, then runs the ELF under
`qemu-system-arm -M lm3s6965evb -cpu cortex-m4 -nographic
-semihosting-config enable=on,target=native -kernel <elf>`
(`resilient-runtime-loader-demo/run_qemu.sh`, wrapped in a 30s
`timeout`). The job fails on a QEMU timeout, a non-zero QEMU exit
status, or semihosting output that doesn't contain the fixture's
expected `loader ok: Int(21)` string — the same three failure modes
item 2's design called for. One deviation from the original sketch:
item 4 below ("golden comparison" via an `.expected.txt` sidecar) is
simplified to a single hardcoded expected-string check in
`run_qemu.sh`, since there is exactly one on-device example today (the
committed `arithmetic_demo.rzbc` fixture, which already has a
host-side expected-value assertion in `resilient-runtime/src/vm/loader.rs`'s
test suite) — a sidecar-file convention is worth adopting once a
second on-device example exists to make the pattern actually reusable.
Items 3 (RISC-V variant) and 5's multi-example scope-out remain as
originally planned; see section 5, item 5.

`embedded.yml` was explicit before this landed that it ran **build
gates, not runtime exercises** ("No QEMU runners here... A runtime job
would need per-target QEMU and is out of scope for this ticket" —
comment at the top of the file), and
`resilient-runtime-cortex-m-demo/README.md` said the same for its one
hand-written demo. Closing D-E1 for real meant adding an actual
runtime gate:

1. **New job, `embedded-runtime.yml`** (separate workflow file, so it
   doesn't block on `embedded.yml`'s existing build-only jobs and can
   be deferred-while-draft the same way via the existing `run_heavy`
   change-detection pattern both `embedded.yml` and `size_gate.yml`
   already use).
2. **Cortex-M path**: `qemu-system-arm -M lm3s6965evb -cpu cortex-m4
   -semihosting-config enable=on,target=native -kernel <elf>`. The
   loader template (section 3.3) uses `cortex-m-semihosting`'s
   `hprintln!`/`debug::exit()` to report pass/fail and a process exit
   code QEMU forwards, so CI can just check the QEMU process's exit
   status — no serial-port scraping needed.
3. **RISC-V variant**: `qemu-system-riscv32 -M virt -kernel <elf>`
   with `riscv-rt` + the equivalent semihosting exit convention
   (`riscv-semihosting`'s `sprintln!`/`syscall::exit()`). Same
   pass/fail contract as the Cortex-M path so the CI step is
   target-parameterized rather than duplicated. Not yet shipped — see
   section 5, item 5.
4. **Golden comparison**: each embedded example ships the same
   `<name>.expected.txt` sidecar convention the host corpus already
   uses (`resilient/examples/*.expected.txt`). The CI step captures
   QEMU's semihosting stdout and diffs it against the sidecar —
   reusing the existing golden-file discipline rather than inventing
   a new one. Simplified for the single-example case — see the
   "Status" note above.
5. **Scope boundary**: this job exercises the `no_std-scalar` VM
   profile only, on 1–2 example programs, until section 1's (b)-class
   opcodes grow real no_std `Value` backing. It is intentionally not
   a full port of the host example corpus. The job also starts as
   **advisory, not required**: it is not yet in `main`'s
   required-status-checks list, so it can prove itself flake-free
   against a brand-new CI dependency (`qemu-system-arm` via apt) for a
   few cycles before it can block auto-merge.

---

## 5. Decomposition — the D-E1 child sequence

Each item below is sized to land as an independent, green PR, per this
repo's decomposition convention (`CLAUDE.md`, "Tackling complex
tickets"):

1. **`resilient-runtime`: `vm` feature skeleton.** Add the `vm`
   feature (section 2.3), a `NoStdVm` struct with fixed-capacity
   stacks (section 2.1), and dispatch arms for the 31 (a)-class
   opcodes only (section 1). No serialization yet — tests construct
   `Chunk`/`Op` values by hand in-crate. This is the PR that proves
   the scalar subset actually runs under `#![no_std]` on host tests
   and cross-compiles to all three embedded targets.
2. **Bytecode serialization: the `.rzbc` format.** Implement the
   fixed-width encoder/decoder from section 3.2 in `resilient/`
   (host side, `std`-based) plus a no_std-compatible decoder in
   `resilient-runtime` behind the `vm` feature. Round-trip tests:
   compile a `.rz` program on host, serialize, deserialize inside a
   `#![no_std]` unit test, compare `Op` streams.
3. **`rz build --target <TRIPLE>` subcommand — DONE.** Shipped as a
   real `build` subcommand (`resilient/src/rzbc_emit.rs` +
   `dispatch_build_subcommand` in `lib.rs`): compile → reject (with a
   clear diagnostic, never a silent host-interpret fallback or a
   malformed blob) any construct outside the no_std-clean scalar
   subset, including every `fn` declaration (no call-frame stack yet)
   → serialize straight to a `.rzbc` blob via
   `resilient_runtime::vm::serde::encode`. Scoped down from the
   original sketch: no `target_profiles.rs`/`vm_profile` wiring (the
   `Instr` subset is fixed, not target-parameterized, so there's
   nothing to select between yet) and no loader-crate
   scaffold/refresh (section 3.3) — both remain follow-up work once
   more than one `vm_profile` exists to choose from.
4. **QEMU CI job (Cortex-M) — DONE.** Landed in two PRs rather than
   one: #4042 first shipped the loader binary itself
   (`resilient-runtime-loader-demo/`) as a hand-authored crate rather
   than an `rz build`-scaffolded template — section 3.3's
   scaffold-on-demand behavior remains a follow-up, this PR just needed
   *a* binary to point QEMU at. Then `embedded-runtime.yml`
   (`qemu_cortex_m` job) added the actual CI wiring: section 4, items
   1–2 and 4 (simplified per that section's "Status" note), for the
   `lm3s6965evb` + Cortex-M4 case only. Advisory, not required yet
   (section 4, item 5).
5. **QEMU CI job (RISC-V).** Same job, RISC-V variant (section 4,
   item 3). Split from #4 so a flaky QEMU/semihosting setup on one
   architecture doesn't block the other from merging.
6. **`Value::Array` in `resilient-runtime` + no_std VM
   `MakeArray`/`LoadIndex`/`StoreIndex`.** First (b)-class opcode
   group ported, gated on `vm-alloc`. Chosen first because arrays are
   the highest-leverage collection type in real embedded example
   programs (sensor buffers, ring buffers).
7. **Interrupt lowering for the no_std VM.** Today `#[interrupt(...)]`
   handlers compile to Rust-level `extern "C"` symbols
   (`resilient-runtime-cortex-m-demo`'s vector table, per
   `docs/no-std.md`'s GPIO/interrupt example) in the *host-compiled*
   path. A bytecode-VM program needs an equivalent: the loader
   template reserves a small table of `(interrupt_name, chunk_index)`
   entries from the `.rzbc` function table, and the vector table's
   weak-alias trampoline calls into the VM at that chunk instead of a
   native Rust function. This is its own PR because it touches the
   vector-table wiring in `resilient-runtime-cortex-m-demo`, not just
   `resilient-runtime`.

PRs 1–5 are the minimum viable "a `.rz` scalar program really runs on
real embedded targets under CI" slice. PRs 6–7 begin closing the gap
between that minimum slice and the full language.

---

## See also

- [`docs/no-std.md`](/no-std) — the `resilient-runtime` crate's
  current (host-verified, cross-compile-verified) feature matrix and
  roadmap pointer this document expands on.
- [`docs/STDLIB_PORTABILITY.md`](/stdlib-portability) — the tiering
  model this pipeline's `vm`/`vm-alloc` split follows for the rest of
  the standard library.
- `resilient/src/bytecode.rs`, `resilient/src/vm.rs` — the host `Op`
  enum and dispatch loop this document audits.
- `resilient-runtime/src/lib.rs`, `resilient-runtime/Cargo.toml` — the
  existing no_std `Value` type and feature-gate conventions this
  design extends rather than replaces.
