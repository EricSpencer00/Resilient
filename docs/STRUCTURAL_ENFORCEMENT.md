# Structural Enforcement — the Resilient trust model

> **Scope.** Code from any author is subject to compiler checks. Deterministic
> verification tools check stated properties within a model; assurance also needs
> evidence that the model captures requirements and the implementation conforms.

This document answers a critique that came up publicly:

> *Preventing LLMs from violating invariants only works if those
> invariants are complete, correct, and enforced structurally — not
> checked post-hoc. Right now you've described a loop where agents
> generate specs, agents generate code, agents validate against those
> specs. That's not enforcement. That's self-consistency.*
>
> — [r/VibeCodersNest, 2026](https://www.reddit.com/r/VibeCodersNest/comments/1ssv8ih/)

Separating the invariant checker from its author is useful, but does not close
the specification gap. A correct checker can accept an incomplete or wrong
specification. Agent-written code can be excellent, and human authorship offers
no guarantee. See the [assurance provisions](ASSURANCE_PROVISIONS.md) for the
requirements, conformance and check-coverage evidence to review.

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
│  Trusted checking components (with assumptions)             │
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

The compiler checks supported obligations using its configured analyses and
optional verifier paths. The diagram is a trust-boundary sketch, not a claim
that every annotation is proved in every configuration. A specification that
contradicts its checked obligations may be rejected; one that omits an intended
requirement can still pass. Compiler translation and backend correspondence
need their own evidence, irrespective of the source's author.

## What's structural (compile-time, type-system-level)

| Mechanism | What it makes unrepresentable | Module / RES ticket |
|---|---|---|
| Linear types | use-after-move on linear values | [resilient/src/linear.rs](../resilient/src/linear.rs), RES-385 |
| Region annotations | dangling references across explicit regions | [resilient/src/lib.rs](../resilient/src/lib.rs) `check_region_aliasing`, RES-391 |
| Effect lattice (`pure` / `io` / `fails`) | a `pure` fn calling an `io` fn | [resilient/src/typechecker.rs](../resilient/src/typechecker.rs) `check_program_effects`, RES-389 / RES-387 |
| Newtype nominal typing | adding `Meters` to `Seconds` | [resilient/src/newtypes.rs](../resilient/src/newtypes.rs), RES-319 |
| Bounds-check obligations | unproven `arr[i]` under `--deny-unproven-bounds` | [resilient/src/bounds_check.rs](../resilient/src/bounds_check.rs), RES-351 |
| Termination annotation | unbounded direct recursion under `--strict-termination` | [resilient/src/termination.rs](../resilient/src/termination.rs), RES-398 |
| Spec-provenance lint (L0012) | Missing source comments on spec-bearing sites, regardless of author; default warning, opt-in denial | [resilient/src/lint.rs](../resilient/src/lint.rs), RES-397 |
| `no_std` enforcement | heap allocation in the embedded runtime | `resilient-runtime/Cargo.toml` |

## What's external (deterministic verifiers we delegate to)

| Solver | Role | Trust assumption |
|---|---|---|
| **Z3** | Checks supported contract, invariant, bounds and recovery obligations when enabled | Solver correctness and the compiler's source-to-query translation are separate assumptions; a successful solver result alone does not establish translation correctness or obligation completeness. |
| **TLA+ / TLC** | Expresses specifications / checks configured concurrent or distributed models | Review the spec, configuration, enabled properties and any translation to/from code. Generation from source alone does not establish model adequacy or conformance. |
| **`cargo clippy`** | Lints the compiler itself | Standard Rust toolchain. |
| **Embedded cross-compile gates** | Proves no-std / size / target-triple constraints | `thumbv7em-none-eabihf`, `thumbv6m-none-eabi`, `riscv32imac-unknown-none-elf`. |

## What we trust

- The Resilient compiler. (Audited like any compiler. Self-hosting goal —
  see [RES-323](https://github.com/EricSpencer00/Resilient/issues/115) and
  [RES-379](https://github.com/EricSpencer00/Resilient/issues/171) — closes
  the bootstrap path; self-hosting alone does not establish compiler correctness.)
- Z3 / TLA+ / the LLVM / Cranelift backends. Standard verifier and codegen
  infrastructure with decades of engineering behind them.
- The Rust toolchain that hosts the compiler.

## What we explicitly do not trust

- Authorship as an assurance signal. Human- and agent-written code, invariants
  and comments all need evidence; a verifier checks its supported obligations.
- The user-supplied spec, *as a spec*. See the spec-provenance section
  below — a wrong-but-internally-consistent spec is provable and useless,
  so provenance is a starting point, not validation of the spec.

## Spec provenance — the strongest version of the critique

The Reddit critique's strongest claim is about **spec provenance**: if the
author supplies an incomplete or wrong invariant, it may be provable while
missing the intended requirement. Provenance supports review of this gap:

1. **L0012** ([RES-397](https://github.com/EricSpencer00/Resilient/issues/327)):
   every spec-bearing site (`requires`, `ensures`, `recovers_to`, `fails`,
   `assume`) must be preceded by a `// source: <canonical-reference>`
   comment. Default warning, escalate with `--deny L0012`.

2. **Acceptable sources** are *external* anchors: an RFC, a hardware
   manual, a paper, an internal design doc, or a deliberate "derived from
   <api-contract>". The point is that the invariant has a paper trail
   that reviewers can compare with the requirement. A paper trail does not
   establish that the source is correct, applicable, or fully modeled.

3. **Citation validation boundary**: today L0012 requires the source
   comment; a later hardening pass should also validate that the cited
   source resolves (URL, ISBN, repo path) and promote unresolved
   citations to a hard gate for safety-critical builds.

## What's still expressible-but-invalid

See [EXPRESSIBLE_INVALID_STATES.md](EXPRESSIBLE_INVALID_STATES.md) — the
public registry of what Resilient *cannot yet* prevent at the type
level, with a ticket link for each gap.

That document is the honest answer to "*can an invalid state be expressed
in your system?*" — yes, here is the list, and here is the closure plan.

## See also

- [docs/VERIFICATION_LIMITS.md](VERIFICATION_LIMITS.md) —
  RES-202: where formal verification's guarantees end and real-world
  uncertainty begins.
- [STABILITY.md](../STABILITY.md) — the language-surface stability policy.
- [SECURITY.md](../SECURITY.md) — the security disclosure policy and
  no-`unsafe`-without-justification rule.
