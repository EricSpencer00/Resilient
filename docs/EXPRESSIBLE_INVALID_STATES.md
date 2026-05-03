# Expressible-but-invalid states — Resilient registry

This is a public, ticket-linked registry of program states that Resilient
**cannot yet structurally prevent** — i.e., the type system permits
expressing them today, even though they violate the language's safety
goals. Each row maps to a ticket that closes the gap.

This document is the honest answer to the question:

> *Can an invalid state even be expressed in your system?*
> — [r/VibeCodersNest, 2026](https://www.reddit.com/r/VibeCodersNest/comments/1ssv8ih/)

For the trust model and what *is* structurally enforced today, see
[STRUCTURAL_ENFORCEMENT.md](STRUCTURAL_ENFORCEMENT.md).

---

## Open gaps

| Class | Status | Closing ticket(s) | Notes |
|---|---|---|---|
| Use-after-move on conditional paths | partial — direct linearity lands; conditional consumption pending Z3 fallback | [#214 (RES-385a)](https://github.com/EricSpencer00/Resilient/issues/214), [#215 (RES-385b)](https://github.com/EricSpencer00/Resilient/issues/215), [#216 (RES-385c)](https://github.com/EricSpencer00/Resilient/issues/216) | RES-385b just landed (commit `76a2ff1`). |
| Dangling references across regions | regions are checked when annotated; **inference is missing** so unannotated code is permissive | [#220 (RES-394)](https://github.com/EricSpencer00/Resilient/issues/220), [#221 (RES-395)](https://github.com/EricSpencer00/Resilient/issues/221) | Currently blocked. |
| Aliasing under polymorphic regions | structural check only; SMT-backed alias analysis pending | [#219 (RES-393)](https://github.com/EricSpencer00/Resilient/issues/219) | Z3-backed extension. |
| Unbounded direct recursion | runtime depth cap (RES-267) catches it; **opt-in** static check via `--strict-termination` (RES-398) | [#328 (RES-398)](https://github.com/EricSpencer00/Resilient/issues/328) | Landed in this PR. Mutual recursion still escapes — see next row. |
| Unbounded *mutual* recursion | runtime cap only — `--strict-termination` is direct-only | follow-up to RES-398 | SCC-based call-graph analysis is the path. |
| Race-prone actor patterns | runtime supervision + Z3 commutativity check at the actor level; supervisor trees and primitives still landing | [#125 (RES-333)](https://github.com/EricSpencer00/Resilient/issues/125), [#124 (RES-332)](https://github.com/EricSpencer00/Resilient/issues/124), [#17 (RES-208)](https://github.com/EricSpencer00/Resilient/issues/17) | |
| Effect leakage in higher-order functions | annotated effects are checked; **polymorphic effects pending** | [#14 (RES-193)](https://github.com/EricSpencer00/Resilient/issues/14) | A `pure` HOF that takes an `io` callback currently has no clean way to express its effect. |
| Duck-typed structural mismatches | nominal typing only; no structural / trait constraints | [#82 (RES-290)](https://github.com/EricSpencer00/Resilient/issues/82) | Trait/interface system. |
| LLM-invented invariants without a paper trail | **lint L0012 (default warning)** — see RES-397 | [#327 (RES-397)](https://github.com/EricSpencer00/Resilient/issues/327) | Landed in this PR. Strict mode is a follow-up. |
| `assume(false)` discharging vacuous obligations | lint L0006 fires by default; `--safety-critical` promotes it to a hard compile error | [#782 (RES-778)](https://github.com/EricSpencer00/Resilient/issues/782), RES-198 | Default mode stays permissive for experimentation; safety-critical mode closes the vacuous-proof path. |
| Self-hosting trust loop | compiler is bootstrapped from Rust today; a local parity gate now cross-checks Rust vs self-hosted lexer/parser on a curated corpus | [#115 (RES-323)](https://github.com/EricSpencer00/Resilient/issues/115), [#171 (RES-379)](https://github.com/EricSpencer00/Resilient/issues/171) | Trust is improving, but the bootstrap root still lives in Rust. |
| FFI memory safety across the trampoline | runtime check; no static guarantee | [#175 (RES-383)](https://github.com/EricSpencer00/Resilient/issues/175) | Security audit of the FFI trampoline. |
| Cluster-wide invariants under partition | TLA+ model checking integration is **design phase only** | [#270 (RES-396)](https://github.com/EricSpencer00/Resilient/issues/270) | V2+ scope. |

---

## Closed gaps (for reference)

These were once on this list and have been closed:

| Class | Mechanism | Closed by |
|---|---|---|
| Unproven array index out-of-bounds | `--deny-unproven-bounds` rejects unproven accesses at compile time | RES-351 |
| `recovers_to:` postcondition violation | Z3 discharge of recovery invariant at fn declaration | RES-387 |
| Mixed `Meters + Seconds` arithmetic | Newtype nominal typing | RES-319 |
| `pure` fn calling an `io` fn | Effect lattice in the typechecker | RES-389 |

---

## How to read this document

- **`Status`** captures what works *today* on `main`. "Partial" means
  some sub-cases are caught; the ticket explains which.
- **`Closing ticket(s)`** points to the GitHub issue that finishes the
  job. If a ticket is `blocked`, it has an open prerequisite.
- **A row leaves this list** only when the gap is closed *structurally*,
  not just runtime-checked. A runtime check is recorded in the
  `Status` column but does not graduate the row.

If you find a gap that is not on this list, please open an issue —
the registry is meant to be exhaustive within the project's stated
goals (safety-critical embedded; AI-generated code constrained at
compile time). See [CONTRIBUTING.md](../CONTRIBUTING.md).
