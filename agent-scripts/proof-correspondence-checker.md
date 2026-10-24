# Structural source checkpoints

`proof-correspondence-checker.py` authenticates a signed saved-query bundle with
an explicitly trusted compiler, public key and solver, then compares each
requested implementation query with independently supplied source checkpoints.
Its only current profile is `legacy-lia-identity-v1`: one imported-free
`program.rz`, top-level `program::identity`, ensures clause0 and identity return.
Four reviewed source fixtures cover straight return, if/else,
early-return/fallthrough and ordered guarded scalar match with wildcard.

Supply `--rz`, `--compiler-sha256`, `--solver`, `--solver-sha256`, `--cert-dir`,
`--source-root`, `--pubkey`, `--descriptor`, `--descriptor-sha256`,
`--profile legacy-lia-identity-v1` and `--build-profile debug` (or the actual
trusted build profile). Hash arguments are caller trust anchors; reading hashes
from the same untrusted bundle would not establish trust. The checker invokes
`rz verify-all --source-root ... --pubkey ... --z3` itself with the pinned solver.
It emits JSON and exits0 only for `code: matched`; other outcomes are nonzero.
`authenticated` distinguishes replay success from subsequent structural failure.

Comparison expands lexical parallel lets while preserving free-variable scope.
It preserves all remaining command structure, assertion and guard order, and
multiplicity. It does not simplify algebra or use solver equivalence to accept a
checkpoint. Quoted identifiers, quantifiers and non-reviewed profiles fail closed.
The default whole-audit deadline is20seconds (POSIX main thread; configurable
0.1..60), with bounded regular-file reads, output and process-group cleanup.
Inputs must remain a controlled stable snapshot; the checker makes no atomic
filesystem snapshot guarantee.

The caller must review every descriptor source hash, declaration/clause/path
label, source domain, returned expression and exact command checkpoint. Labels
are trusted manual mapping, not automatically enumerated declarations. Missing
sites in a caller-supplied descriptor remain an explicit trust boundary. A match
establishes only this supplied policy's authenticated structural checkpoints;
it does not prove source inventory completeness, runtime reachability, general
translation correctness, BV semantics or qualification acceptance. Dynamic
libraries and the whole runtime environment require separate retained evidence.
Issue4953 remains open for the broader source-to-SMT requirements.
