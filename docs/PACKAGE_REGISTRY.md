# Package Registry Index

Tracking issue: [#4114](https://github.com/EricSpencer00/Resilient/issues/4114)
(E-E2: package-manager registry protocol).

This document specifies the static JSON index format used to resolve
a bare package name (e.g. `rz pkg add mylib`) without requiring a
`path:`/`git:` source specifier, and the checksum-verification rules
applied to anything resolved from it.

This is **increment 1** of the registry protocol: the index format
and integrity verification, implemented in
`resilient/src/pkg_registry.rs`. It is deliberately network-free and
testable hermetically. Fetching an index (or package contents) over
HTTP for `rz pkg add <name>` / `rz pkg update` is later work — this
document only specifies the shape of the index and how its checksums
are checked once bytes are in hand, from whatever source.

## Index schema

The index is a single JSON document:

```json
{
  "packages": {
    "mylib": {
      "versions": {
        "1.0.0": {
          "source": "https://example.com/mylib-1.0.0.tar.gz",
          "sha256": "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        },
        "1.1.0": {
          "source": "https://example.com/mylib-1.1.0.tar.gz",
          "sha256": "3608bca1e44ea6c4d268eb6db02260269892c0b42b86bbf1e77a6fa16c3e58f"
        }
      }
    }
  }
}
```

### Fields

- **`packages`** (object, required) — top-level map from package name
  to a package entry. Package names are index keys and are not
  otherwise validated by this increment (naming rules can be tightened
  in a follow-up without breaking the schema).
- **`packages.<name>.versions`** (object, required) — map from version
  string to a version entry. At least one version key must be present
  for a package to resolve to "latest"; an empty `versions` object
  parses successfully but every resolution against it fails with
  `NoVersions`.
- **`versions.<version>.source`** (string, required, non-empty) —
  opaque locator for where the package's contents live. This
  increment does not interpret `source` (a later increment defines
  how `rz pkg update` fetches from it, e.g. treating it as an HTTPS
  URL to a tarball).
- **`versions.<version>.sha256`** (string, required) — the expected
  SHA-256 digest of the package contents referenced by `source`,
  encoded as exactly 64 lowercase hex characters. Any other length,
  non-hex character, or uppercase hex character is a schema-validation
  error at parse time, not a checksum-verification error at use time.

### Version ordering

"Latest" resolution (no version pinned) currently picks the
lexicographically greatest version *string*. This is a known
limitation for unpadded semver (e.g. `"9.0.0" < "10.0.0"` sorts the
other way lexicographically) and is called out explicitly rather than
silently mishandled; a follow-up can add real semver-aware ordering
without changing the index schema.

## Checksum verification

Checksum verification is an **integrity** check (detects corruption,
truncation, or a tampered/compromised mirror) — it is **not** a
signing or trust mechanism, and it does not attempt to authenticate
the publisher. Once package bytes are obtained (however they were
obtained), verification is:

1. Compute the SHA-256 digest of the bytes.
2. Compare (case-insensitively) against the `sha256` field of the
   resolved `packages.<name>.versions.<version>` entry.
3. On mismatch: hard error. Callers must never fall back to using the
   unverified bytes, ignore the mismatch, or retry a different source
   silently.

See `resilient/src/pkg_registry.rs::verify_checksum` and its tests for
the exact behavior, including the three cases required by the tracking
issue: a successful resolve + verify, a rejected checksum mismatch,
and a "package not in index" error.

## What this increment does *not* do

- No network fetch of the index or of package contents (`rz pkg
  update` and index-backed `rz pkg add <name>` resolution are
  follow-up increments on #4114).
- No index signing (only content-integrity checksums, not
  provenance/trust).
- No real semver range matching (`^1.0`, `~1.2`, etc.) — only exact
  version pins or "latest" (lexicographic-max version string).
