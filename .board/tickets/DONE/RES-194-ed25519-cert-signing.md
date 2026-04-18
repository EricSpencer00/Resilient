---
id: RES-194
title: Ed25519 signature on emitted verification certificates (RES-071 follow-up)
state: DONE
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

## Resolution

### Files added
- `resilient/src/cert_sign.rs` — new module. Public API:
  - `sign_payload(priv_b: &[u8; 32], payload: &[u8]) -> [u8; 64]`
  - `verify_payload(pub_b: &[u8; 32], payload: &[u8], sig: &[u8; 64]) -> Result<bool, String>`
  - `compute_cert_payload(dir) -> Result<Vec<u8>, String>` —
    concatenates `.smt2` files in lexicographic name order, joined
    with `\n` separators, skipping non-smt2 files (including
    `cert.sig` itself).
  - `parse_public_key_pem` / `parse_private_key_pem` /
    `format_public_key_pem` / `format_private_key_pem` —
    minimal hex-PEM codec.
  - `parse_signature_hex` / `format_signature_hex` — `cert.sig` is
    a single 128-char hex line.
  - `EMBEDDED_PUBLIC_KEY_PEM` — `include_str!("cert_key.pem")`.
  - 13 unit tests covering round-trip, tamper (payload, signature,
    public key), PEM validation, hex codec edge cases, the
    directory-walker's sort + ignore-non-smt2 policy, and an
    end-to-end sign-and-verify-with-tamper flow at the API level.

- `resilient/src/cert_key.pem` — committed 32-byte dev public
  key in the mini-PEM format. The corresponding private key
  lives off-repo per the ticket's key-management policy.

- `resilient/tests/verify_cert_smoke.rs` — 5 integration tests
  driving the real binary:
  - `sign_cert_and_verify_round_trip` — emit → verify OK
  - `verify_cert_fails_against_mismatched_key` — rc=1
  - `verify_cert_detects_payload_tamper` — rc=1 after an extra
    `.smt2` file lands in the cert dir
  - `verify_cert_errors_on_missing_sig` — rc=2 when `cert.sig`
    isn't there
  - `verify_cert_requires_directory_argument` — rc=2 on missing arg

### Files changed
- `resilient/Cargo.toml` — added `ed25519-dalek = { version = "2",
  features = ["rand_core"] }` and `rand_core = { version = "0.6",
  features = ["getrandom"] }` to dependencies. Test-time OS RNG
  access is needed for the fresh-keypair generator in unit
  tests (`SigningKey::generate(&mut OsRng)`). `Cargo.lock` picks
  up the transitive tree (curve25519-dalek, sha2, signature, etc.).
- `resilient/src/main.rs`
  - `mod cert_sign;` declaration.
  - `emit_certificates` signature grew an optional
    `sign_key_path: Option<&Path>`. When present it reads the
    PEM, computes the payload, signs, and writes
    `<dir>/cert.sig`. Both emissions (cert dir + signature)
    print their own cyan status line so the user sees both.
  - `execute_file` threads the same option through.
  - `--sign-cert <path>` (and `--sign-cert=<path>`) CLI flag.
  - `dispatch_verify_cert_subcommand(args)` — new top-level verb
    `resilient verify-cert <dir> [--pubkey <path>]`. Called from
    `main()` alongside the existing `dispatch_pkg_subcommand`.
    Uses the embedded public key by default; `--pubkey` override
    (exposed for tests + rotation). Exit codes: 0=valid,
    1=tampered/wrong key, 2=usage.

- `README.md` — new "Signed certificates (RES-194)" subsection
  under "Verification certificates" documenting `--sign-cert`,
  the `verify-cert` verb, exit codes, the PEM format, and the
  key-management policy.

### Design decisions
- **Mini-PEM over PKCS#8/SPKI.** 32-byte raw-bytes hex wrapped in
  `-----BEGIN ED25519 {PUBLIC,PRIVATE} KEY-----` markers. No
  base64 or ASN.1 dep — the `cert_sign::hex_*` helpers are 30
  lines. Interop story: a user can use any library to generate
  an Ed25519 keypair; they format the output as our mini-PEM
  themselves. Interop with `ssh-keygen -t ed25519` etc. is a
  follow-up.
- **Payload is sorted-file concatenation, not a Merkle tree.**
  `cert.sig` signs the whole payload as one Ed25519 signature;
  no per-file signatures. Simpler to emit + verify. A follow-up
  ticket could add per-file signatures if the auditing
  experience demands it.
- **Embedded-key-parses test.** A unit test parses
  `EMBEDDED_PUBLIC_KEY_PEM` at runtime so any future accidental
  breakage of `cert_key.pem` (e.g. committed in a state with a
  bad byte count) fails CI immediately instead of at first
  `verify-cert` use.
- **`--pubkey` override on `verify-cert`.** The ticket says the
  embedded key is the default; we added `--pubkey <path>` as an
  opt-in override. Reasons: lets integration tests generate
  ephemeral keypairs; lets users rotate without a recompile for
  ad-hoc verification. Production use-case still goes through
  the embedded key with no flag.

### End-to-end manual check
```
$ src=/tmp/p.rs && printf 'fn add(int a, int b) { return a + b; } fn main(int _d) { return add(1, 2); } main(0);\n' > $src
$ dir=$(mktemp -d)
$ resilient -t --seed 0 --emit-certificate $dir --sign-cert /tmp/priv.pem $src
Wrote 0 verification certificate(s) to /.../
Wrote Ed25519 signature to /.../cert.sig

$ resilient verify-cert $dir --pubkey /tmp/pub.pem
cert: signature verified for /.../

$ resilient verify-cert $dir        # uses embedded (non-matching) key
cert: SIGNATURE MISMATCH for /.../
$ echo $?
1
```

### Verification
- `cargo build` → clean
- `cargo test --locked` → 513 + 16 + 4 + 3 + 1 + 12 + 4 + 5 = 558
  tests pass (was 500; +13 cert_sign unit tests; +5 integration)
- `cargo test --locked --features lsp` → 540 + 16 + 4 + 3 + 1 +
  12 + 8 + 4 + 5 pass
- `cargo clippy --locked --features lsp,z3,logos-lexer --tests
  -- -D warnings` → clean

### Follow-ups (not in this ticket)
- **Key rotation story** (called out in the ticket's Notes). A
  `--pubkey-list` that accepts multiple trusted keys would let
  the binary trust both the current + previous signing key during
  a rotation window.
- **PKCS#8 / OpenSSL interop.** Today the mini-PEM isn't what
  `ssh-keygen` / `openssl pkey` emit. A helper that round-trips
  to/from those formats would smooth the rollout.
- **Per-file signatures.** Currently tampering any `.smt2` file
  invalidates the whole batch; per-file signatures would let a
  partial audit accept the untouched files.
- **`--sign-cert` without `--emit-certificate`.** Today the flag
  is silently ignored without its paired flag. A warning when
  `--sign-cert` is set without `--emit-certificate` would be
  friendlier.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 resolved by executor (Ed25519 signing via
  `--sign-cert`; `verify-cert` subcommand with embedded public
  key + override; 13 unit tests, 5 integration tests, end-to-end
  verified)
