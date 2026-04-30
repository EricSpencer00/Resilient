---
title: Certificate Manifest Schema v1
parent: Standards
nav_order: 2
permalink: /certificates
---

# Certificate Manifest Schema v1
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
- A `MANIFEST.json` enumerating every obligation, its certificate
  filename, and a SHA-256 over the certificate bytes.
- (Optional, with `--sign-cert <key>`) Ed25519 signatures attached to
  each obligation entry, signing the certificate bytes.

The manifest is the **proof-carrying binary** artifact:
hand it to a downstream consumer alongside the binary, and they can
re-verify every obligation through Z3 with `rz verify-all <DIR>`.

## Schema v1

```json
{
  "schema": "v1",
  "compiler": "resilient 0.1.0",
  "z3": "4.13.0",
  "timestamp": "2026-04-29T03:24:00Z",
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

| Field         | Type    | Required | Description                                              |
| ------------- | ------- | -------- | -------------------------------------------------------- |
| `schema`      | string  | yes      | Always `"v1"` for this version.                          |
| `compiler`    | string  | yes      | Compiler name + version that produced the manifest.      |
| `z3`          | string  | yes      | Z3 version used (from `z3 --version`).                   |
| `timestamp`   | string  | yes      | ISO-8601 UTC timestamp at emit time.                     |
| `program`     | string  | yes      | Path to the source file (relative or absolute).          |
| `obligations` | array   | yes      | One entry per discharged obligation. Order is stable.    |

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
  file contains the unsat-core proof. This is the default for entries
  that appear in the manifest.
- **`runtime`** — Z3 timed out or returned `unknown`; the obligation
  was retained as a runtime check. Runtime-only obligations do **not**
  appear in the manifest (the manifest only enumerates *discharged*
  proofs).

If a downstream verifier needs to know the full set of obligations
(both proven and runtime-retained), inspect the source program rather
than the manifest. The manifest is intentionally a positive record
of "what we proved", not a negative one.

## Verification flow

```text
$ rz --emit-certificate ./certs --sign-cert priv.pem prog.rz
... compiles, emits ./certs/MANIFEST.json + per-obligation .smt2 files
... signs each cert with Ed25519

$ rz verify-all ./certs --pubkey pub.pem
... walks MANIFEST.json
... for each obligation:
...   1. recompute SHA-256 over cert bytes
...   2. verify Ed25519 sig (if --pubkey supplied)
...   3. invoke z3 on the cert and check (check-sat) returns `unsat`
... exits 0 only if every obligation passes
```

A non-zero exit code from `verify-all` indicates one of:

- A cert file was tampered with (SHA-256 mismatch).
- A signature does not validate against the supplied public key.
- Z3 disagreed with a proven obligation (this should not happen
  unless the Z3 version differs significantly).
- The manifest itself is malformed.

## Compatibility

- Schema v1 is the inaugural version.
- New fields may be added at any time without bumping the schema.
- Removing or renaming a field requires bumping to v2 and emitting a
  migration path in this doc.
- Verifiers MUST tolerate unknown top-level and obligation-entry
  fields (forward compatibility).

## See also

- [Certification and Safety Standards](/certification) — what
  Resilient contributes to DO-178C / ISO 26262 / IEC 61508.
- `resilient/src/cert_sign.rs` — `Manifest` and
  `ManifestObligation` definitions.
- `rz verify-all` (RES-195) — the inverse path that re-validates a
  manifest end-to-end.
