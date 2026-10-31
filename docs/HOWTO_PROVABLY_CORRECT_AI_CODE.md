---
title: "How-To: Make AI-Generated Code Provably Correct"
nav_order: 13
permalink: /howto-provably-correct-ai-code
---

# How to Make AI-Generated Code Provably Correct
{: .no_toc }

A task-oriented walkthrough of the RES-3780 verification stack: tag
AI-authored code for the audit trail, opt a module into mandatory
contracts, bound its loops, and produce a portable proof artifact a
downstream consumer can check without re-running the compiler.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Before you start: provenance is not enforcement

Earlier drafts of this feature (RES-3780 as originally filed) tied
verification to the `@ai_generated` tag itself: tag a function and the
compiler would force contracts and loop bounds onto it. RES-3854 /
RES-3858 deliberately reversed that design — **correctness is not a
property of who (or what) wrote the code.** A hand-written function
deserves the same proof obligations as a model-generated one, and a
tag you can delete to skip checks was never a real guarantee.

The current model splits the two concerns:

| Concern | Mechanism |
|---|---|
| "Was this AI-authored?" (audit trail) | `@ai_generated` / `#[generated(...)]` — pure metadata |
| "Must this be verified?" (enforcement) | `@require_contracts` / `@require_contracts(strict)` — a module-level policy |

Adding or removing `@ai_generated` changes **zero** compiler
diagnostics. If you want the compiler to actually check something, you
need `@require_contracts`. The rest of this guide builds that up from
the tagging step to a signed-off proof certificate. See
[`AI_GENERATED_DESIGN.md`](AI_GENERATED_DESIGN.md) for the full design
history if you want the "why," not just the "how."

## Step 1 — Tag provenance (optional, informational only)

If you want an audit trail of what a model wrote, tag it:

```resilient
@ai_generated
fn clamp_positive(int x) -> int {
    if x < 0 { return 0; }
    return x;
}
```

For a richer trail (what prompt, what intent), use the underlying
annotation directly instead:

```resilient
#[generated(intent = "clamp a signed reading to non-negative", prompt_hash = "9f2a…")]
fn clamp_positive(int x) -> int { /* … */ }
```

`@ai_generated` is recorded as an alias of `#[generated(...)]`
(RES-3835 / RES-3858). Either form shows up in
`--emit-contract-certificate` output as an informational
`"provenance"` array on that function — nothing more. **Tagging alone
buys you no checking.** Continue to Step 2 for that.

## Step 2 — Opt the module into contract verification

Add `@require_contracts` at the top of the file to enrol **every**
function in it:

```resilient
@require_contracts

fn clamp_positive(int x) -> int requires x < 1000 ensures result >= 0 {
    if x < 0 { return 0; }
    return x;
}
```

Under the bare directive, any clause you *do* write must be
non-vacuous:

- every `requires` must reference at least one parameter
  (`requires true` is rejected);
- every `ensures` must reference `result`
  (`ensures true` or an input-only restatement is rejected).

Functions with **no** contract at all are still accepted under the
bare directive — it only stops you from writing a fake contract, not
from writing none.

### Mandatory contracts: `@require_contracts(strict)`

To also require every function to *have* a contract, use the strict
variant:

```resilient
@require_contracts(strict)

fn add(int x, int y) -> int requires x >= 0 requires y >= 0 ensures result >= 0 {
    return x + y;
}
```

Under `strict`, every named function except `main` must declare at
least one `ensures` clause, and at least one `requires` clause if it
takes parameters. `main` is exempt — it has no caller-supplied
parameters and returns no result the caller inspects. This is the
"safety-critical module" posture: nobody can opt a function out simply
by not writing a contract.

If you drop the `requires`/`ensures` above, the compiler reports:

```
error[contract_policy]: function `add` violates `@require_contracts`: strict policy
demands at least one `requires` clause constraining its inputs — add `requires <param_condition>`
```

## Step 3 — Bound your loops

Once a function is enrolled (via either `@require_contracts` variant),
any `while` loop in its body must carry a `#[loop_bound(N)]` attribute
naming its maximum iteration count:

```resilient
@require_contracts(strict)

#[loop_bound(100)]
fn count_up(int n) -> int requires n >= 0 requires n <= 100 ensures result >= 0 {
    let i = 0;
    while (i < n) {
        i = i + 1;
    }
    return i;
}
```

Omit `#[loop_bound(N)]` on an enrolled function with a `while` loop and
the compiler reports:

```
error[loop_bound]: function `count_up` enrolled in contract verification: contains
a while-loop and requires #[loop_bound(N)] (RES-3780 Tier 2)
```

With `--features z3`, the compiler additionally tries to *prove* (or
refute) the declared bound for loops that follow a simple
monotonic-counter shape (`while (i < n) { i = i + step; }`-style). A
bound the prover can't statically match falls back to a runtime check
instead of a hard compile error — you get a warning, not a failure.

`#[loop_bound(N)]` is unaffected by provenance: it fires purely off
`@require_contracts` enrolment plus the presence of a `while` loop,
regardless of whether `@ai_generated` is attached.

## Step 4 — Make violations fatal: `--typecheck-strict`

**This is the step people miss.** By default, `rz` runs the type
checker but treats violations as *soft* diagnostics: they print to
stderr, and the program still compiles and runs (RES-1088 — this keeps
legacy scripts executing while surfacing what they're missing). That
means everything in Steps 2–3 above will *warn* by default, not fail
your build.

To make contract-policy and loop-bound violations (or any type error)
fatal, pass `--typecheck-strict`:

```console
$ rz --typecheck-strict my_module.rz
```

This is the flag to wire into CI — it exits non-zero the moment any
enrolled function violates its contract policy or loop-bound
requirement, with none of the extra "Running type checker…" /
"Type check passed" lines that plain `-t`/`--typecheck` prints.

## Step 5 — Verify with Z3

Build (or run) with `--features z3` to get real proof attempts instead
of `warning[partial-proof]: Z3 returned Unknown` placeholders:

```bash
cargo build --manifest-path resilient/Cargo.toml --features z3
```

With Z3 available, each `requires`/`ensures` clause and each
`#[loop_bound(N)]` is discharged against an SMT query. A clause that
Z3 refutes is a hard type error with a counterexample; a clause it
can't decide (unsupported theory, solver timeout) degrades to
`unknown` and falls back to a runtime check — it never silently
"passes." See
[`VERIFICATION_MODEL.md`](VERIFICATION_MODEL.md) and
[`VERIFICATION_LIMITS.md`](VERIFICATION_LIMITS.md) for what the
prover can and can't discharge.

## Step 6 — Emit a portable proof certificate

`--emit-contract-certificate <FILE>` writes a deterministic JSON
document — per function, per clause — attesting what the verifier
established, independent of whether the caller has Z3 or even the
Resilient toolchain installed:

```console
$ rz my_module.rz --emit-contract-certificate cert.json
```

```json
{
  "schema": "resilient-contract-certificate/v1",
  "schema_version": 1,
  "source": "my_module.rz",
  "functions": [
    {
      "name": "count_up",
      "enrolled": true,
      "provenance": ["ai_generated"],
      "clauses": [
        { "clause": "n >= 0", "kind": "requires", "verdict": "unknown" },
        { "clause": "n <= 100", "kind": "requires", "verdict": "unknown" },
        { "clause": "result >= 0", "kind": "ensures", "basis": "clause-only", "verdict": "unknown" }
      ]
    }
  ]
}
```

Each clause's `"verdict"` is one of:

- `"pass"` — Z3 proved it; a replayable SMT-LIB2 dump appears as
  `"smtlib2"` on the same object.
- `"fail"` — Z3 refuted it; a `"counterexample"` appears alongside.
- `"unknown"` — out of the supported theory subset, solver timeout, or
  a build without `--features z3`. The runtime check still guards the
  clause at execution time; this is not "unverified and unguarded," it
  is "not statically proved."

### What an `ensures` proof actually attests — the `"basis"` field

An `ensures` clause carries a `"basis"` field recording **against what**
the verdict was established. This is the difference between "the
postcondition is *self-consistent*" and "the function *actually
computes* a value the postcondition admits":

- `"basis": "implementation"` — the verifier substituted the function
  body's return expression for `result` before proving, so the
  obligation is grounded in what the code returns. A `"pass"` here means
  the returned value provably satisfies the clause for every admitted
  input; a `"fail"` means there is a concrete input (in the
  `"counterexample"`) that satisfies the preconditions yet returns a
  value the clause forbids. This is the guarantee that makes the
  certificate meaningful: a wrong `max` that returns `x`

  ```
  fn max(int x, int y) -> int ensures result >= x && result >= y { return x; }
  ```

  is **refuted** (`x >= x && x >= y` fails for `y > x`), while the
  correct `if x >= y { return x; } else { return y; }` **passes** — the
  two no longer verify identically.

- `"basis": "clause-only"` — the body is outside the substituted subset
  (only single `return E;` and single `if/else`-of-returns over pure
  arithmetic/boolean expressions are modelled today), so `result` was
  left as a free variable. A `"pass"` here attests only that the clause
  is a tautology / consistent with the preconditions — **not** that the
  body returns a conforming value. Treat `clause-only` passes as
  "well-formed postcondition," not "verified implementation," and lean
  on the retained runtime check.

Bodies with loops, local `let` bindings, or function calls in the
return position fall back to `clause-only`; extending the substituted
subset to richer control flow is tracked as follow-up work.

Unlike `--emit-certificate <DIR>` (the older RES-071 SMT-LIB2 manifest
+ signature format, see [`CERTIFICATES.md`](CERTIFICATES.md)),
`--emit-contract-certificate` works in **every** build configuration —
including the default, non-z3 one most CI runners use — precisely
because `"unknown"` is a legitimate, documented verdict rather than a
failure. `provenance` is informational only: it never changes a
verdict, only records that the function carried `@ai_generated` and/or
`#[generated(...)]`.

### Trusting the certificate — `schema_version` and tamper-evidence

The `"schema_version"` field (currently `1`) is a numeric contract on
the document's shape: a consumer checks it before parsing anything
else, so a future field addition or reinterpretation fails closed
(a typed error) instead of silently misparsing. Resilient's own
`contract_certificate::verify_schema_version` never panics on
malformed or missing input — see `resilient/src/contract_certificate.rs`.

The JSON document itself carries no cryptographic material by
default — trusting it means trusting the `rz` binary that produced
it. Under `--features z3`, `contract_certificate::sign_bytes` /
`verify_signed` reuse the same RES-194 Ed25519 primitives as
`--emit-certificate`/`--sign-cert` (see
[`CERTIFICATES.md`](CERTIFICATES.md)) to bind a certificate's exact
bytes to a keypair: any tampering, down to a single flipped bit,
fails verification.

## Step 7 — Cheap CI signal: `--vibe-gate`

If you want a fast, syntactic, no-Z3-required smoke check — "did
*anything* in this file ship without contracts at all?" — use
`--vibe-gate <threshold>`. It runs the `vibe_debt` analyzer (a
lightweight heuristic, *not* the Tier 1–3 machinery above) and exits
based on a `[0.0, 1.0]` debt threshold:

```console
$ rz --vibe-gate 0.3 my_module.rz
```

For each top-level function, `vibe_debt` scores 4 boolean signals —
has a `requires`, has an `ensures`, is referenced elsewhere in the
program, carries a `pure`/`io`/`@pure` effect annotation — and reports
`1 - (sum_score / (4 * fn_count))` as the program's debt percentage.
The gate prints one line of JSON to stderr and sets the exit code:

```console
$ cat contracted.rz
fn add(int x, int y) -> int requires x >= 0 requires y >= 0 ensures result >= 0 {
    return x + y;
}
fn main() {
    println(add(1, 2));
}
main();

$ rz --vibe-gate 0.8 contracted.rz
{"vibe_debt": 0.50, "threshold": 0.80, "passed": true}
$ echo $?
0

$ rz --vibe-gate 0.01 contracted.rz
{"vibe_debt": 0.50, "threshold": 0.01, "passed": false}
$ echo $?
2
```

(`add` scores 2/4 signals — `requires` and `ensures` present, but
never called elsewhere and carries no effect annotation — and `main`
scores 0/4, so the file-wide debt lands at 50%. A `// @requires` /
`// @ensures` *comment* does not count; only the real `requires`
/ `ensures` keywords do.)

`--vibe-gate=<threshold>` (equals form) works too. An out-of-range
(`<0.0` or `>1.0`) or non-numeric threshold exits `2` with a clean
`error: --vibe-gate …` message — never a panic.

`--vibe-gate` is a **heuristic triage signal**, not a soundness proof —
it doesn't know about `@require_contracts` enrolment, `#[loop_bound]`,
or Z3 verdicts at all; it only looks at what's syntactically present.
Use it as an early, cheap gate on a large/legacy codebase where wiring
up full `@require_contracts(strict)` everywhere isn't realistic yet;
use Steps 2–6 above for functions where you actually need a proof.

> This repository does not wire `--vibe-gate` into its own required CI
> gates (doing so on a project this size would risk blocking auto-merge
> on unrelated work) — it's exercised by
> `resilient/tests/it/vibe_gate.rs` and documented here so downstream
> projects can adopt it for their own CI.

## Putting it all together

```resilient
@require_contracts(strict)

#[loop_bound(100)]
@ai_generated
fn count_up(int n) -> int requires n >= 0 requires n <= 100 ensures result >= 0 {
    let i = 0;
    while (i < n) {
        i = i + 1;
    }
    return i;
}

fn main() {
    println(count_up(0));
    println(count_up(5));
    println(count_up(10));
}

main();
```

```bash
# Fail the build on any contract-policy or loop-bound violation:
rz --typecheck-strict loop_bound_demo.rz

# Attempt real Z3 proofs instead of "unknown" placeholders:
cargo build --features z3 --manifest-path resilient/Cargo.toml
./resilient/target/debug/rz --typecheck-strict loop_bound_demo.rz

# Emit a portable, offline-checkable proof artifact:
rz loop_bound_demo.rz --emit-contract-certificate cert.json

# Cheap heuristic pre-check on a large tree before you wire up contracts everywhere:
rz --vibe-gate 0.5 loop_bound_demo.rz
```

This exact program (modulo comments) is [`resilient/examples/loop_bound_demo.rz`](../resilient/examples/loop_bound_demo.rz);
its golden output lives in the sibling `.expected.txt`, and
`resilient/tests/it/contract_certificate_e2e_smoke.rs` exercises the
`--emit-contract-certificate` step end-to-end (asserting certificate
*structure*, since verdicts are `"unknown"` without `--features z3`).

## Related reading

- [`AI_GENERATED_DESIGN.md`](AI_GENERATED_DESIGN.md) — the design
  history of why provenance and enforcement were split apart.
- [`VERIFICATION_MODEL.md`](VERIFICATION_MODEL.md) — what the Z3
  verifier proves and how.
- [`VERIFICATION_LIMITS.md`](VERIFICATION_LIMITS.md) — the boundary of
  what today's prover can and cannot discharge.
- [`CERTIFICATES.md`](CERTIFICATES.md) — the older `--emit-certificate`
  SMT-LIB2 manifest + signature format, for comparison.
- [`AI_THREAT_MODEL.md`](AI_THREAT_MODEL.md) — `--ai-threats` /
  `#[ai_review_required]`, a complementary static-pattern scanner for
  AI-introduced security anti-patterns (a different concern from
  contract correctness).
