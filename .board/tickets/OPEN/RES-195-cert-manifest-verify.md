---
id: RES-195
title: Certificate manifest + `verify-all` subcommand
state: OPEN
priority: P3
goalpost: G19
created: 2026-04-17
owner: executor
---

## Summary
RES-071 / RES-194 emit per-obligation certificates. A program has
many. A manifest lists every obligation + its cert path + its
hash, and `verify-all` re-verifies the whole set in one shot.
This is what a downstream regulator would consume.

## Acceptance criteria
- `--emit-certificate <DIR>` (already exists) additionally writes
  `DIR/manifest.json`:
  ```json
  { "program": "fib.rs",
    "obligations": [
      {"fn": "fib", "kind": "ensures", "cert": "fib_ensures.smt2",
       "sha256": "...", "sig": "..."}
    ] }
  ```
- New subcommand `resilient verify-all <DIR>` walks the manifest,
  re-runs Z3 on each cert, verifies the signature if present,
  prints a table + exits 0 if all pass.
- Integration test: a known-good program emits + verifies; a
  tampered cert fails verification.
- Commit message: `RES-195: cert manifest + verify-all`.

## Notes
- sha256 of the cert is separate from the Ed25519 sig — lets
  consumers detect tampering even without the pubkey.
- `verify-all` is cheap wall-clock; each cert is small and Z3
  discharges quickly. No parallelism needed.

## Log
- 2026-04-17 created by manager
