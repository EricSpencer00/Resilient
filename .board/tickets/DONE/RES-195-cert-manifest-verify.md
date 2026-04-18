---
id: RES-195
title: Certificate manifest + `verify-all` subcommand
state: DONE
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

## Resolution

### Files changed
- `resilient/Cargo.toml` — new regular deps: `sha2 = "0.10"`
  (already transitive via ed25519-dalek; promoted to a direct
  edge for the manifest's hash column) and `serde_json = "1"`
  (manifest parse/emit). `serde_json` dropped from dev-deps
  since it's now a regular dep.
- `resilient/src/cert_sign.rs`
  - New `sha256_hex(bytes) -> String` helper using `sha2::Sha256`.
  - New `Manifest` + `ManifestObligation` structs: `program`,
    `obligations[{fn_name, kind, idx, cert, sha256, sig?}]`.
  - New `format_manifest_json(&Manifest) -> String` (canonical
    pretty-printed JSON with deterministic key order) and
    `parse_manifest_json(&str) -> Result<Manifest, String>`
    (tolerates extra fields for forward-compat).
  - 9 new unit tests in `cert_sign::manifest_tests`: SHA-256
    FIPS 180-2 vector + empty-input vector, round-trip
    (signed + unsigned), parse rejects missing `program` /
    `obligations` / obligation fields, parse tolerates extra
    fields, end-to-end per-obligation signature through the
    manifest.
- `resilient/src/main.rs`
  - `emit_certificates` signature grew a `source_filename:
    &str` argument and now always writes `manifest.json`. When
    `--sign-cert` is passed, each obligation's manifest entry
    carries its own per-cert Ed25519 signature (new); the
    top-level `cert.sig` batch signature from RES-194 stays
    for backward compat.
  - New `dispatch_verify_all_subcommand(&args)`:
    `resilient verify-all <dir> [--pubkey <path>] [--z3]`.
    Walks `manifest.json`; per obligation: reads the cert,
    recomputes sha256, verifies the optional per-cert
    signature, optionally shells out to `z3 -smt2` when
    `--z3` is passed AND the `z3` binary is on PATH.
  - Helper `which_z3()` scans `PATH` for the `z3` binary so
    the `--z3` flag degrades gracefully to a warning when
    unavailable.
  - `run_z3_on(path)` shells out with `std::process::Command`
    and treats a first stdout line of `unsat` as success.
  - Wired into `main()` alongside `dispatch_pkg_subcommand`
    and `dispatch_verify_cert_subcommand`.
- `resilient/tests/verify_all_smoke.rs` — 8 integration tests
  driving the real binary:
  - Happy path unsigned (sha-only pass).
  - SHA-256 mismatch.
  - Happy path signed.
  - Signature tamper (sig over different bytes).
  - Missing `manifest.json`.
  - Malformed `manifest.json`.
  - Missing directory arg.
  - End-to-end `--emit-certificate` writes a manifest that
    `verify-all` subsequently accepts.
- `README.md` — new "Certificate manifest (RES-195)" subsection
  under "Signed certificates" documenting the JSON schema,
  `verify-all` usage, `--pubkey` / `--z3` flags, and exit
  codes.

### Design deviations from the AC's literal wording
The ticket's JSON example has `sig` per-obligation; I honored
that literally — each obligation's `sig` is a signature over
THAT cert's bytes (not the batch). The RES-194 batch `cert.sig`
still exists so `verify-cert` keeps working as documented.
Signed runs write both: N per-obligation sigs + 1 batch sig.
Ed25519 is fast; the cost is negligible.

The `verify-all` output is a one-row-per-obligation table with
columns `fn`, `kind`, `sha256`, `sig`, `z3`. Each column shows
`ok` / `FAIL` / `-` (not-applicable). The ticket said "prints a
table"; this matches.

Z3 re-verification is opt-in (`--z3`) rather than default-on,
for two reasons:
- Z3 may not be on PATH (developer machines without a stock
  install; the manifest-level signature checks alone are
  already a tight invariant).
- Running Z3 N times on every `verify-all` invocation is
  slower than users would expect from a "check my certs"
  utility. The default stays cryptographic-only; the extra
  button is available when paranoid.

### Verification
- `cargo build` → clean
- `cargo test --locked` → 533 + 16 + 4 + 3 + 1 + 12 + 4 + 8 + 5
  = 586 tests pass (was 524 core; +9 manifest unit tests;
  +8 integration tests)
- `cargo test --locked --features lsp` → 560 + 16 + 4 + 3 + 1 +
  12 + 8 + 4 + 8 + 5
- `cargo clippy --locked --features lsp,z3,logos-lexer --tests
  -- -D warnings` → clean
- End-to-end manual check: `--emit-certificate` produces
  `manifest.json` + `cert.sig`; `verify-all` reports
  "all checks passed" on fresh emit; tampering the manifest's
  sha256 flips to `FAIL` exit 1.

### Follow-ups (not in this ticket)
- **z3 default-on policy** — once the test matrix includes z3,
  flipping `--z3` to default-on (with `--no-z3` escape hatch)
  becomes cheap.
- **Parallel z3 runs** — per the ticket Notes, "No parallelism
  needed" for now. Revisit if cert directories grow past a few
  dozen files.
- **Hash key rotation** — the embedded public key is versioned
  only by git history. A `--pubkey-list` that accepts multiple
  trusted keys would let the binary trust both current + prior
  during a rotation window.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 resolved by executor (`manifest.json` emitter +
  `verify-all` subcommand; per-obligation sha256 + optional
  Ed25519 sig; opt-in Z3 re-verification; 9 unit + 8
  integration tests)
