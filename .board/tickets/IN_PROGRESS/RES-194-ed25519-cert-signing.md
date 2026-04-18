---
id: RES-194
title: Ed25519 signature on emitted verification certificates (RES-071 follow-up)
state: IN_PROGRESS
priority: P3
goalpost: G19
created: 2026-04-17
owner: executor
---

## Summary
RES-071 emits SMT-LIB2 certificates for each proven contract.
Certificates are user-verifiable via stock Z3, but not yet
cryptographically tied to the tool that produced them. Ed25519-sign
each certificate so downstream consumers can trust the provenance.

## Acceptance criteria
- CLI flag `--sign-cert <path-to-ed25519-key>`; without it,
  certificates are emitted unsigned (current behavior).
- With the flag, each certificate directory gains a
  `cert.sig` file containing the Ed25519 signature of the SMT-LIB2
  payload.
- New `verify-cert` subcommand: `resilient verify-cert <dir>` —
  checks the signature against a public key embedded in the binary
  at build time (PEM in `resilient/src/cert_key.pem` or equivalent).
- Use `ed25519-dalek` for signing/verification.
- Unit tests: sign → verify round-trip; tamper detection.
- Commit message: `RES-194: Ed25519-signed certificates`.

## Notes
- Key management is a human concern — signing key is supplied by
  the invoker, not committed to the repo. Document clearly in
  README.
- A public key IS committed to the repo (the one our signing
  pipeline uses); a follow-up ticket can introduce a key-rotation
  story.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
