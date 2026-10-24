# Source correspondence fixtures

`expectations.json` records manually authored source path domains and return
substitutions, frozen before this suite's query capture. These are trusted
fixture specifications, not a compiler-generated completeness inventory.
`source-review.json` records the pre-capture technical review. Its reviewer
model was not independently established; no human or organizational
independence is claimed.

The five positive capture cases check source hashes, expected model-site
counts, authenticated source-bound replay and the requested query-profile
marker. They retain public source, signed manifests, queries and executable
identities in existing CI logs. Those observations alone do not establish
source-to-query correspondence. Exact guard, ordered first-match exclusion and
return-substitution review remains required; whole-query UNSAT equivalence
cannot detect an omitted guard.

`negative-expectations.json` preserves the original source-authored controls.
Its **try-handler hard-refutation expectation is invalid** and must not be
used as an acceptance criterion. `try-disposition-review.json` supplies the
effective disposition without changing the frozen original bytes. The pure
try body returns `x` without throwing; the handler's modeled return of zero
is not a demonstrated feasible runtime path. UNSAT over conservative
alternatives may justify a proof; SAT in an overapproximation does not alone
justify rejecting the source program. That case records output only. The
other four controls require a specific implementation-contract refutation,
not a generic parser/typechecker error.

The try/catch frontend and later precision checks are separate evidence
stages. A model-only proof cannot bypass a public CLI frontend failure or earn
qualification credit. These fixture bytes have not yet been executed in this
lane. The 256 KiB capture bound applies to the emitted JSON record, not to
subprocess execution or output. Executable hashes and version text do not
establish complete dynamic linkage or whole-environment identity.
