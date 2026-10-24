# Authenticated source inputs

`rz verify-all certs/ --source-root snapshot/ --pubkey public.pem --z3`
authenticates the manifest signature, checks the complete frozen source-input
inventory, then replays certificates. The new flag implies manifest signature
verification. Legacy verification without this flag keeps its existing behavior.

Certificate emission records SHA-256 and byte length from each exact buffer
passed to the entry parser or import parser. It signs the entry, file inventory,
resolved import edges, and builtin module declarations inside `manifest.json`.
This capture is independent of the incremental compilation cache, including
`--no-cache`. A file changing between repeated parsing operations is rejected.

The source root is the common canonical ancestor of the entry and imported
files at emission. Inventory paths are relative to that root. Supply the
corresponding root of an identical relocated tree to verification; preserve its
relative directory structure. Absolute paths and parent traversal in the
inventory are rejected.

Verification parses the source snapshots and checks every import against the
signed resolution graph, including transitive imports, then checks complete
file/edge/builtin reachability. It never executes the program, invokes package
resolution, runs Git, downloads dependencies, or materializes caches. The graph
freezes the producer's resolved translation unit: this is not evidence that the
current package manifest, lockfile or registry configuration would resolve the
same dependencies. Those configuration files are not independently bound by
this inventory. Builtin modules are implemented by the compiler; bind the
compiler executable and its libraries separately in the release evidence.

Use a stable, controlled source snapshot. Canonical containment, regular-file
checks and size checks occur before source reads, but filesystem name checks
and file opens are not atomic against concurrent filesystem or symlink changes.
This mechanism does not claim race-resistant access to an adversarially changing
filesystem.

Version 1 accepts at most 256 inputs, 1,024 import/builtin edges, 10 MiB per
source, 64 MiB total source bytes, and 64 nested import levels. Reads are bounded
independently of the signed byte lengths. Cycles, missing inputs, malformed
inventory, duplicate entries, unused edges and unreachable files fail verification.

This establishes tested authenticated source-content identity and frozen import
selection. It does not prove source-to-SMT translation correctness, completeness
of emitted proof obligations, correctness of the signing producer, or external
qualification acceptance. The paired hosted workflow retains the same new CLI
regressions on the pinned production baseline and candidate, with real signed
manifests and solver executable/library identity.

The [retained paired record](../hosted-records/2026-10-source-binding/README.md)
preserves all 48 source-bound bundles, 23 verified red-green attestations, raw
hosted output and reconstructible source trees from run 40056681094. Its tested
revision is explicit; later binary baselines must rerun the applicable suites.
