# Independent source contract inventory

This tool discovers lexical declaration and clause sites from source text in
`restricted-declarations-v1`. It consumes a caller-reviewed, hash-pinned source
catalog, not a list of expected contract sites. Unsupported declaration syntax
rejects the whole inventory. Success means only that this restricted lexical
inventory was produced; it does not establish compiler validity, resolved calls,
proof eligibility, signature authentication or qualification acceptance.

The accepted declaration forms are plain functions, inline `mod` blocks,
non-generic `impl Type` methods, simple structs and literal local `.rz` uses with
optional `as` aliases. Function headers accept typed parameters (or method
`self`), a simple return type, and scalar contract expressions. Bodies are
balanced token regions, not a complete statement parser or type checker.
Declaration-bearing keywords inside bodies reject the grammar rather than hide
nested contract sites. Generic/trait/attribute/package/builtin syntax and
executable entry statements are outside this version. Identifiers are ASCII;
comments and string contents use UTF-8. Reported spans are byte offsets, not
compiler character offsets or resolved binding identities.

## The input is a literal-use occurrence catalog

`--graph` retains its initial CLI spelling, but its input role is an independent
**literal-use occurrence catalog v1**. It contains `version: 1`, `entry`, exact
`files` records (`path`, byte count, SHA256), `imports` and an empty `builtins`
list. Each imports record has `from`, the literal `specifier`, the confined
`target` and `namespace`, which here means the explicit source `as` alias or
null. Every literal-use occurrence is counted, including repeated edges and two
aliases of the same file. Each physical source file is enumerated once; its
contract sites do not become semantic import instances merely because aliases
exist.

This is **not interchangeable with compiler source-binding graph v1**. That
compiler graph records resolution edges, can deduplicate importer/specifier
pairs, and ordinarily records `namespace: null`. It does not authenticate every
literal-use alias occurrence. The compatible-looking raw JSON shape and version
number do not make the schemas or trust claims equivalent. Passing that compiler
graph for a source with missing alias occurrences must reject as an import graph
mismatch or remain unresolved in a later mapping stage. Do not normalize away
missing occurrences to claim coverage, and do not modify a signed compiler graph
to substitute the independent catalog.

A future authenticated mapping needs signed occurrence/provenance sidecars and
reviewed links among physical source sites, resolved declaration/import binding
instances and proof roles. Current function labels, raw callee names, unspanned
clauses and character offsets are insufficient. The current tool supplies no
such mapping or alias proof credit. Module/import/method obligations remain open
under issue4953 even when their lexical sites appear in an inventory.

## Trust and operation

Review the complete catalog and source snapshot before selecting the digest.
Computing a digest from an untrusted catalog and immediately trusting that value
would not establish an independent trust anchor. A representative invocation is:

```sh
python3 agent-scripts/source-contract-inventory.py \
  --source-root /path/to/stable-source \
  --graph /path/to/reviewed-occurrence-catalog.json \
  --graph-sha256 "$TRUSTED_CATALOG_SHA" \
  --grammar resilient/tests/fixtures/source_inventory/grammar.json
```

The implementation pins the reviewed grammar bytes. It checks source byte counts
and hashes, confined regular files, every catalog import occurrence and graph
reachability. Stable local snapshots are required; this is not an atomic snapshot
or an adversarial concurrent-filesystem guarantee. No compiler, solver, package
resolver or network process is invoked.

Version1 bounds are32 files,128KiB per file,1MiB total source,65536 total tokens,
depth32,256 declarations and1024 clauses, with a20-second POSIX main-thread
operation deadline. Errors are nonzero JSON results without partial inventory
arrays. `inventoried` is a lexical outcome only. Unsupported proof bodies,
unresolved references, clause-only results, infeasible entry conditions and
explicitly unselected sites still need distinct downstream dispositions; none
may silently become implementation proof coverage.

The source-authored16-case requirements,20 boundary controls and15 token-kind
controls have separate frozen histories. The first16-case candidate had named
passing outcomes but suppressed raw `println!` reports. Output-only harness v3
will retain direct stdout reports under a separately recorded source version;
old runs must not be relabeled as having retained those reports. The quoted-token
source-review defect remains a merge blocker until actual hosted baseline and
corrected-control evidence are retained. No full-language completeness or human
independent assessment is claimed.
