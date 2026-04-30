# Structural Enforcement — the Resilient trust model

> **TL;DR.** The LLM is a *client* of the type system, not a participant in
> the proof. Z3 and TLA+ are external deterministic solvers that don't
> trust the LLM. The compiler — not the agent — enforces.

This document answers a critique that came up publicly:

> *Preventing LLMs from violating invariants only works if those
> invariants are complete, correct, and enforced structurally — not
> checked post-hoc. Right now you've described a loop where agents
> generate specs, agents generate code, agents validate against those
> specs. That's not enforcement. That's self-consistency.*
>
> — [r/VibeCodersNest, 2026](https://www.reddit.com/r/VibeCodersNest/comments/1ssv8ih/)

The framing is right: filtering ≠ safety, and structural unrepresentability
is the goal. The framing is also incomplete — it conflates the *invariant
checker* with the *invariant author*. Resilient separates the two.

## The boundary

```
┌──────────────────────────────────────────────────────────────┐
│  Outside the trust boundary (untrusted)                      │
│                                                              │
│   ┌───────────┐   writes    ┌────────────────────────────┐   │
│   │    LLM    │ ──────────► │   .rz / .rz source code   │   │
│   └───────────┘             └────────────────────────────┘   │
└──────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌──────────────────────────────────────────────────────────────┐
│  Inside the trust boundary (deterministic, audited)          │
│                                                              │
│   ┌────────────┐   ┌────────────┐   ┌──────────────────┐     │
│   │  Compiler  │ + │  Z3 SMT    │ + │  TLA+ checker    │     │
│   │ (typecheck │   │  (proves   │   │ (model-checks    │     │
│   │  + lints + │   │ invariants │   │  concurrent      │     │
│   │  borrowck) │   │ + bounds)  │   │  protocols)      │     │
│   └────────────┘   └────────────┘   └──────────────────┘     │
│         │                │                    │              │
│         └────────────────┴────────────────────┘              │
│                          │                                   │
│                          ▼                                   │
│              accept — or reject with a diagnostic            │
└──────────────────────────────────────────────────────────────┘
```

The compiler **does not trust** anything written above the boundary. Every
invariant the LLM asserts is re-derived: every `requires`/`ensures` is
discharged by the typechecker (and Z3 when the predicate is symbolic),
every linear-type usage is checked, every `recovers_to:` postcondition is
re-proved. A self-consistent wrong claim from the LLM is **rejected** at
the boundary the same way a wrong claim from a human would be.

## What's structural (compile-time, type-system-level)

| Mechanism | What it makes unrepresentable | Module / RES ticket |
|---|---|---|
| Linear types | use-after-move on linear values | [resilient/src/linear.rs](../resilient/src/linear.rs), RES-385 |
| Region annotations | dangling references across explicit regions | [resilient/src/main.rs](../resilient/src/main.rs) `check_region_aliasing`, RES-391 |
| Effect lattice (`pure` / `io` / `fails`) | a `pure` fn calling an `io` fn | [resilient/src/typechecker.rs](../resilient/src/typechecker.rs) `check_program_effects`, RES-389 / RES-387 |
| Newtype nominal typing | adding `Meters` to `Seconds` | [resilient/src/newtypes.rs](../resilient/src/newtypes.rs), RES-319 |
| Bounds-check obligations | unproven `arr[i]` under `--deny-unproven-bounds` | [resilient/src/bounds_check.rs](../resilient/src/bounds_check.rs), RES-351 |
| Termination annotation | unbounded direct recursion under `--strict-termination` | [resilient/src/termination.rs](../resilient/src/termination.rs), RES-398 |
| Spec-provenance lint (L0012) | LLM-invented invariants without a paper trail | [resilient/src/lint.rs](../resilient/src/lint.rs), RES-397 |
| `no_std` enforcement | heap allocation in the embedded runtime | `resilient-runtime/Cargo.toml` |

## What's external (deterministic verifiers we delegate to)

| Solver | Role | Trust assumption |
|---|---|---|
| **Z3** | Discharges `requires`/`ensures`, loop invariants, bounds, recovery postconditions | We trust Z3's soundness as published; we don't trust the SMT-LIB query the LLM produced — the *compiler* produces the query from the typed AST. |
| **TLA+** | Model-checks concurrent / distributed protocols | Same — we trust the checker; the spec it consumes is generated from Resilient source, not pasted from an LLM. |
| **`cargo clippy`** | Lints the compiler itself | Standard Rust toolchain. |
| **Embedded cross-compile gates** | Proves no-std / size / target-triple constraints | `thumbv7em-none-eabihf`, `thumbv6m-none-eabi`, `riscv32imac-unknown-none-elf`. |

## What we trust

- The Resilient compiler. (Audited like any compiler. Self-hosting goal —
  see [RES-323](https://github.com/EricSpencer00/Resilient/issues/115) and
  [RES-379](https://github.com/EricSpencer00/Resilient/issues/171) — closes
  the trust loop end-to-end once the bootstrap is complete.)
- Z3 / TLA+ / the LLVM / Cranelift backends. Standard verifier and codegen
  infrastructure with decades of engineering behind them.
- The Rust toolchain that hosts the compiler.

## What we explicitly do not trust

- The LLM. Anything it writes — code, invariants, comments — passes
  through the boundary. The verifier re-derives every safety claim.
- The user-supplied spec, *as a spec*. See the spec-provenance section
  below — a wrong-but-internally-consistent spec is provable and useless,
  so we require provenance.

## Spec provenance — the strongest version of the critique

The Reddit critique's strongest claim is about **spec provenance**: if the
LLM invents the invariant, a self-consistent wrong spec is provable and
useless. This is a real gap. Our answer:

1. **L0012** ([RES-397](https://github.com/EricSpencer00/Resilient/issues/327)):
   every spec-bearing site (`requires`, `ensures`, `recovers_to`, `fails`,
   `assume`) must be preceded by a `// source: <canonical-reference>`
   comment. Default warning, escalate with `--deny L0012`.

2. **Acceptable sources** are *external* anchors: an RFC, a hardware
   manual, a paper, an internal design doc, or a deliberate "derived from
   <api-contract>". The point is that the invariant has a paper trail
   independent of the LLM that wrote the surrounding code.

3. **Future work**: validate that the cited source actually exists
   (resolvable URL, ISBN, repo path), promote the lint to a hard gate
   for safety-critical builds.

## What's still expressible-but-invalid

See [EXPRESSIBLE_INVALID_STATES.md](EXPRESSIBLE_INVALID_STATES.md) — the
public registry of what Resilient *cannot yet* prevent at the type
level, with a ticket link for each gap.

That document is the honest answer to "*can an invalid state be expressed
in your system?*" — yes, here is the list, and here is the closure plan.

## See also

- [docs/formal-verification-limitations.md](formal-verification-limitations.md) —
  RES-202: where formal verification's guarantees end and real-world
  uncertainty begins.
- [STABILITY.md](../STABILITY.md) — the language-surface stability policy.
- [SECURITY.md](../SECURITY.md) — the security disclosure policy and
  no-`unsafe`-without-justification rule.
