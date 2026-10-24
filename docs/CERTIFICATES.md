---
title: Certificate Manifest Schema
parent: Standards
nav_order: 2
permalink: /certificates
---

# Certificate Manifest Schema
{: .no_toc }

Schema reference for the proof-carrying certificate manifests that
`rz --emit-certificate <DIR>` produces.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Overview

When you compile a Resilient program with `--emit-certificate <DIR>`,
the compiler writes:

- One `<fn>__<kind>__<idx>.smt2` file per discharged Z3 obligation
  (see `format_cert_filename` in `cert_sign.rs`).
- A `manifest.json` enumerating every obligation, its certificate
  filename, and a SHA-256 over the certificate bytes.
- (Optional, with `--sign-cert <key>`) Ed25519 signatures attached to
  each obligation entry, signing the certificate bytes.

The manifest indexes discharged queries, not the compiled binary or the full
requirement set. A downstream consumer can replay each listed query through Z3
with `rz verify-all <DIR> --z3`; source-to-query and source-to-binary correctness
require separate evidence.

## Manifest JSON shape

The current writer emits the two top-level fields below. Verifiers
must tolerate unknown top-level and obligation-entry fields, so future
releases can add optional metadata such as schema version, compiler
version, Z3 version, or timestamps without breaking older consumers.

```json
{
  "program": "examples/sensor.rz",
  "obligations": [
    {
      "fn": "read_sensor",
      "kind": "requires",
      "idx": 0,
      "cert": "read_sensor__requires__0.smt2",
      "sha256": "8c1f3e...",
      "sig": "a1b2c3..."
    }
  ]
}
```

### Top-level fields

| Field         | Type   | Required | Description                                           |
| ------------- | ------ | -------- | ----------------------------------------------------- |
| `program`     | string | yes      | Path to the source file (relative or absolute).       |
| `obligations` | array  | yes      | One entry per discharged obligation. Order is stable. |

### Obligation entry fields

| Field    | Type     | Required | Description                                                |
| -------- | -------- | -------- | ---------------------------------------------------------- |
| `fn`     | string   | yes      | Function name the obligation belongs to.                   |
| `kind`   | string   | yes      | One of: `requires`, `ensures`, `invariant`, `recovers_to`. |
| `idx`    | integer  | yes      | Zero-based index within the function for this `kind`.      |
| `cert`   | string   | yes      | Filename of the `.smt2` file (no directory prefix).        |
| `sha256` | string   | yes      | Hex-encoded SHA-256 over the cert file's bytes.            |
| `sig`    | string   | no       | Hex-encoded Ed25519 signature when `--sign-cert` is used.  |

## Outcome semantics

Each obligation entry corresponds to a verification *outcome*:

- **`proven`** — Z3 returned `unsat` for the negation. The `.smt2`
  file contains the SMT query to replay. This is the default for entries
  that appear in the manifest.
- **`runtime`** — Z3 timed out or returned `unknown`; the obligation
  was retained as a runtime check. Runtime-only obligations do **not**
  appear in the manifest (the manifest only enumerates *discharged*
  proofs).

If a downstream verifier needs to know the full set of obligations
(both proven and runtime-retained), inspect the source program rather
than the manifest. The manifest is intentionally a positive record
of "what we proved", not a negative one.

### `#[overflow_checked]` obligations (RES-4112) are not yet certified

A fn annotated `#[overflow_checked]` gets an additional BV64
(wraparound-aware) re-check of its `requires`/`ensures` clauses via
`verifier_z3::prove_overflow_safe` (see `docs/VERIFICATION_MODEL.md`
for the full theory-selection rationale). That pass does **not** yet
emit an SMT-LIB2 certificate — `--emit-certificate` only captures the
default LIA verdict for a clause, exactly as before this ticket. A
manifest entry with `"kind": "requires"` proves the clause holds under
unbounded arithmetic; it says nothing about whether the same clause
also survived the stricter BV64 pass. Certificate support for the
BV64 obligation is tracked as a follow-up (mirrors the existing gap
for the BV32 bitwise-op prover).

## Verification flow

```text
$ rz --emit-certificate ./certs --sign-cert priv.pem prog.rz
... compiles, emits ./certs/manifest.json + per-obligation .smt2 files
... signs each cert with Ed25519

$ rz verify-all ./certs --pubkey pub.pem --z3
... walks manifest.json
... for each obligation:
...   1. recompute SHA-256 over cert bytes
...   2. verify each present Ed25519 sig using the selected key
...   3. invoke z3 on the cert and check (check-sat) returns `unsat`
... exits 0 only if every obligation passes
```

Solver replay requires `--z3`. A missing or unlaunchable solver is an error;
replay never falls back to integrity-only checks. An empty obligation set is
also rejected in replay mode. Without `--z3`, empty manifests retain their
integrity-only behavior and no solver is required.

Each solver invocation has a 30-second default deadline and a 64 KiB limit
for each captured stream. Set `RZ_SOLVER_TIMEOUT_MS` to an integer from 1 to
300000 to select a different deadline. Invalid settings fail replay before
launching the solver. Standard input is closed. A timeout or capture failure
terminates and reaps the direct solver child; this is not containment of an
arbitrary process tree. Use a trusted solver executable rather than a wrapper
that leaves descendants running or holding output pipes open.

With `--z3`, acceptance requires a
successful solver exit, valid UTF-8 stdout containing exactly one `unsat`
response (surrounding whitespace is permitted), and empty stderr. Additional
results, Unknown, solver errors and failed processes fail closed. This is
solver-checked replay of the saved query under trusted encoding and Z3; it
is not an independent kernel proof or a source-to-query correctness proof.

A non-zero exit code from `verify-all` indicates one of:

- A cert file was tampered with (SHA-256 mismatch).
- A signature does not validate against the supplied public key.
- Z3 disagreed with a proven obligation (this should not happen
  unless the Z3 version differs significantly).
- The manifest itself is malformed.

## Qualification evidence

The [saved-query replay validation record](https://github.com/EricSpencer00/Resilient/tree/main/qualification/certificate-replay)
contains operational requirements, real-solver queries, baseline-bound execution
records and remaining limitations. It does not establish tool qualification.

## Compatibility

- The current manifest shape is the inaugural version.
- New optional fields may be added at any time without changing the
  required field set above.
- Removing or renaming a required field requires a versioned migration
  path in this doc.
- Verifiers MUST tolerate unknown top-level and obligation-entry
  fields (forward compatibility).

## See also

- [Certification and Safety Standards](/certification) — what
  Resilient contributes to DO-178C / ISO 26262 / IEC 61508.
- `resilient/src/cert_sign.rs` — `Manifest` and
  `ManifestObligation` definitions.
- `rz verify-all` (RES-195) — the inverse path that re-validates a
  manifest end-to-end.

### Loop proof artifacts

Loop certificates use isolated solver queries. A while invariant emits one
`loop_invariant` base obligation and one `loop_invariant_step` obligation.
A supported finite accumulator invariant emits its base plus one step for
each literal integer element. Step entries use `<loop:N>` as their function
identifier, where N is the base entry index; their indices enumerate the steps.
The proven-invariant statistic counts the invariant once, not its subproofs.
Each entry retains the normal hash and optional signature checks.

Finite pre-body annotations reach the verifier. Empty iterables, invariants
that refer to the iteration variable, and bodies that assign that variable
remain outside this certificate model and receive no static proof. Runtime
checks remain active. Broader iteration-binding semantics are tracked in
[issue #4904](https://github.com/EricSpencer00/Resilient/issues/4904).

The round-trip regression verifies signed emission, actual Z3 replay and the
exact number of subproofs for representative while and finite loops. It does
not authenticate the completeness of an arbitrary caller-supplied manifest;
that remains a separate qualification requirement.

### Authenticate the complete manifest

Signed emission also writes `manifest.sig`, an Ed25519 signature over the exact
bytes of `manifest.json`. To require that signature and replay every listed
obligation, use:

```bash
rz verify-all certs/ --pubkey approved-public.pem --require-manifest-signature --z3
```

This mode rejects a missing or invalid manifest signature before checking
individual obligations. Editing program metadata, removing an obligation or
reordering entries invalidates the signature. The manifest signature sidecar
is hexadecimal text, limited to 4096 bytes. Choose the public key through a
trusted process; a key supplied by the same untrusted bundle is not a trust anchor.

Without `--require-manifest-signature`, verification retains its legacy
per-obligation behavior and does not authenticate the enumeration. Manifest
signing authenticates what the signer emitted; it does not establish that every
source requirement was discovered, translated correctly or proven. The existing
solver deadlines, output limits and strict UNSAT acceptance still apply.
