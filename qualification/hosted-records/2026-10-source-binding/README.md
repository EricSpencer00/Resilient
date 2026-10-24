# Retained source-input binding regression evidence

[Hosted run 40056681094](https://github.com/EricSpencer00/Resilient/actions/runs/40056681094)
executes the same 24 new CLI cases on a test-only production baseline and the
source-binding candidate. The baseline records 23 expected failures and one
passing legacy compatibility control; the candidate records 24 passes. All 48
bundles and 23 red-green attestations verified. These establish the tested
capability change, not 23 separately classified safety defects.

The baseline is revision `94630f8f45a5b8cd070afa7437ff715a6b293e5e` plus exactly
the two frozen new regression files; its dirty fingerprint is recorded in
`sources.json`. The clean candidate is tested merge
`065b5640925aefd5bc2656f1379057306b7aeb6f`, tree
`35ef3bbc8065d6aed643ea7a37fe9ee4e8bf05a4`. Apply each full-index patch to its
recorded base with a temporary Git index and compare `git write-tree` to the
recorded tree. CI performs that reconstruction and checks patch hashes.

`records.json` losslessly indexes all 265 original UTF-8 files beneath
`baseline/`, `final/` and `attestations/`, preserving each relative path, original
byte SHA-256 and exact text. No record whitespace is normalized. The hosted
validator materializes these files into a fresh confined temporary directory,
rejects absolute/traversal/duplicate paths, verifies every byte hash, then runs
the original bundle and attestation checks. This encoding keeps the evidence
within the repository's normal file-count guard without weakening its checks.
`verified-outcomes.json` indexes their observed outcomes; `raw-actions.log`
retains the complete hosted output. Z3 4.13.3 executable and linked-library
hashes match between phases in `solver.json`. `SHA256SUMS` covers every retained
file except itself. CI verifies the file inventory, solver identities, 48 bundles
and 23 attestations. The legacy control is intentionally not a red-green pair.

The supported operation authenticates the signed source-content inventory and
frozen import selection, including relocated identical snapshots. It does not
validate source-to-SMT translation, completeness of proof obligations, correctness
of the signing producer, current package resolution configuration or target
machine arithmetic. Use a controlled stable snapshot; filesystem checks/opens
are not atomic against adversarial mutation. The historical candidate differs
from later compiler binaries; their own source/hash and rerun records are needed.
No classification, independent regulatory review or assessor acceptance is
asserted by these records.
