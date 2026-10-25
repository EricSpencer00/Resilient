# Requested implementation-coverage audit

`agent-scripts/audit-proof-coverage.py` enforces an independently supplied expected
policy after it invokes authenticated source-bound replay itself. Its success
means **only that this supplied policy is covered**. It does not discover all
source obligations or prove compiler translation correct. A policy that omits a
function, selects the wrong source meaning, or incorrectly marks a site
`unselected` can omit the very property the consuming project needs. Independent
source/requirements review of the policy is therefore a required trust boundary.

The caller explicitly supplies its trusted public key and trusted policy digest.
Do not compute the expected digest from an untrusted policy and treat that as
approval. Pin the accepted policy and reviewer record through the consuming
project's configuration process. Nothing here assigns a safety classification,
regulatory independence or external acceptance.

## Public command and binding

```sh
python3 agent-scripts/audit-proof-coverage.py \
  --rz /controlled/rz --cert-dir /controlled/certs \
  --source-root /controlled/snapshot --pubkey /controlled/accepted-public.pem \
  --policy /controlled/accepted-policy.json --policy-sha256 TRUSTED_POLICY_SHA256 \
  --profile REVIEWED_MODEL_PROFILE --timeout-seconds 30
```

All seven identity/input options are mandatory. The auditor hashes the actual
compiler executable, checks its verbose build profile, compares the exact signed
source graph and checks the trusted policy bytes/profile and configured solver executable digest. It then invokes that
same executable with `verify-all --source-root ... --pubkey ... --z3`. A prior
caller replay is never assumed. Source graph canonicalization is UTF-8 JSON with
sorted object keys, compact separators and unescaped Unicode. The public key is
a caller trust decision, not a key discovered from an untrusted bundle.

The policy's model profile is an independently reviewed claim pinned to exact
compiler bytes and source/formula checkpoints. It is **not** compiler
self-attestation that every construct has those semantics. Build profile and
model profile are distinct. Changing arithmetic metadata or query theory needs
a newly reviewed policy and baseline; this audit does not manufacture arithmetic
correspondence credit.

One JSON report is written to stdout; success exits0, rejection exits1. Stages
are `binding`, `authentication` and `coverage`. Successful reports retain the
policy/compiler/source-graph/solver digests, explicit site descriptors, raw-query
digests, antecedent feasibility and excluded sites. Error codes distinguish
missing/duplicate/extra obligations, wrong proof basis, source/query drift,
checkpoint mismatch and unresolved/infeasible sites.

## Policy structure

Version1 uses schema `resilient-expected-implementation-coverage`, and contains:

- `compiler`: exact `sha256` and actual verbose `build_profile`.
- `solver`: exact `sha256` of the configured solver executable.
- `profile`: accepted model identifier, matched against the explicit CLI option.
- `source_graph_sha256`: canonical signed `source_binding` graph digest.
- `sites`: independently enumerated source-relative `source`, qualified
  `declaration`, zero-based `clause` and stable `path` descriptors. All four form
  site identity; names alone do not distinguish imports/methods/overloads.
- Each `required` site has an exact emitted `obligation` reference (`fn`, `kind`,
  `idx`) and `commands`: the independently reviewed complete SMT command tree.
  `feasibility_commands` independently specifies the source entry/path domain;
  review it together with the full query and source at this same site. Both
  checkpoint trees are bound by the trusted policy digest.
  Implementation sites require `kind: ensures_implementation`. One emitted
  obligation cannot satisfy multiple expected paths.

`unsupported`, `unresolved`, `infeasible` and other non-required statuses reject
requested coverage with their original status retained in the report. Infeasible
is not a claim that a legitimate empty-precondition contract is false. Explicit
`unselected` sites are reported separately and grant no credit; they do not prove
that omitted declarations are actually unselected by import resolution. Imports,
method qualification and selection must be reviewed against the resolved program.
At least one required site is necessary.

All requested implementation references must occur exactly once. Unexpected
implementation references fail; auxiliary nonimplementation certificates are
counted separately and cannot fill implementation sites. A clause-only proof at
the expected function/index cannot substitute for implementation evidence.

## Independently authored semantic checkpoints

The new CLI fixtures in `resilient/tests/proof_coverage_audit.rs` explicitly review:

```rz
fn covered(int x) -> int ensures result == x {
    if x >= 0 { return x; } else { return x; }
}
```

The then path assumes `x >= 0`; the else path assumes `not(x >= 0)`. Both substitute
returned `x` into `result == x`, yielding the negated goal `not(x == x)`. The
independent command checkpoints declare `x: Int`, preserve one exact path guard,
assert the negated goal and end in one `check-sat`. These equality-only checkpoints
avoid overflow dependence; they do not establish general mathematical/machine
integer equivalence. They are authored from source meaning rather than extracted
as the expected answer from the query under test.

Fixture processes sign these reviewed queries/manifests with generated ephemeral
test keys. Missing logical paths, duplicate logical identities, extra paths,
clause-only substitution, wrong guards and duplicate guard assertions retain valid
cryptography and valid UNSAT replay. Tests independently invoke strict replay
before the relevant coverage mutations are audited. Thus a generic bad-signature
or solver failure cannot satisfy their coverage-stage assertions. Distinct valid
query filenames in the duplicate/wrong-guard controls prevent a generic
filename-duplicate check from standing in for logical coverage enforcement.

A separate actual-compiler control uses an identity function beside:

```rz
fn uncovered(int x) -> int ensures result == x {
    let y = x + 1;
    return y;
}
```

At x=0, the return is1 rather than0. Current body modeling does not cover this
leading-let shape. The observed legacy signed/source-bound replay accepts its
neighbor's nonempty proof subset and emits no implementation obligation for this
function. That is consistent with replay's existing limited purpose; it does not
make the neighbor's formula invalid. The requested coverage policy must expose
this unsupported site instead of representing the subset as full requested credit.

The SMT reader preserves command order, declaration/assertion multiplicity,
quoted atoms and complete session shape. Only whitespace/comments are ignored;
it does not simplify equivalent formulas or erase duplicate guards. Accepted
sessions contain only `set-logic`, `declare-const`, zero-arity `declare-fun`, `assert` and one final
`check-sat`. After full-query checkpoint comparison, the auditor checks SAT of the separate
independently authored `feasibility_commands`. It never guesses the antecedent by
removing the last assertion: a query may encode requires/path conditions inside
one negated implication. Full-query and feasibility checkpoints must share exact
theory/symbol declarations, and their reviewed source/path relation is an
explicit trust boundary, not an equivalence automatically proved by this tool.
An arbitrary unrelated SAT control is not an adequate source-domain checkpoint.
UNSAT feasibility is infeasible and UNKNOWN/error outcomes unresolved; neither
grants reachable-site credit or declares an empty-precondition contract false.
The new embedded-entry control binds the source requirements x>0 and x<0 to a
valid signed `not((x>0 and x<0) => x==x)` query and a separately reviewed
contradictory entry-domain checkpoint. It must reject reachable credit even
though stripping the query's only assertion would leave an empty SAT session.

The solver path is resolved once from `Z3_BINARY` (or PATH's `z3`), matched to the
trusted policy digest and pinned as the same absolute `Z3_BINARY` for rz replay
and feasibility evaluation. Drift is checked afterward. This binds executable
identity, not the whole runtime environment: hosted records separately retain
the solver executable hash/version and libz3.so.4 hash. Complete dynamic linkage and whole-environment identity are not established.

## Execution limits and evidence

The supported auditor environment is a POSIX main-thread process with Python3 and the configured
Z3 executable; the qualification scope remains the separately identified host
baseline. A restored POSIX signal timer and final deadline check bound the audit to0.1..60seconds plus bounded process cleanup,
subprocess output to512KiB, policy/manifest documents to2MiB, required sites and
manifest obligations to512, each query to1MiB/100,000tokens/64nesting levels and
all reviewed query bytes to16MiB. Subprocesses use argument vectors, bounded
nonblocking input/output and a new process group; timeouts/output overflow kill
that group including ordinary solver descendants. This is not containment of a
malicious trusted executable that deliberately escapes its process group.

Inputs are opened nonblocking and must be regular files by descriptor metadata; static FIFOs/devices and oversized inputs reject before replay. A watchdog regression checks policy/query FIFOs, and an oversized-manifest case requires structured JSON rather than a Path-serialization crash.

Use a controlled stable filesystem snapshot. Compiler/manifest/query drift checks
are performed, but ordinary filesystem check/open operations are not atomic
against adversarial concurrent mutation. This is not a filesystem isolation claim.

The paired GitHub-hosted workflow is
`.github/workflows/proof-coverage-qualification.yml`. It pins baseline
`9f1d870acc5d5ba963a7e27d522e60aa70f7213e`, overlays the frozen NEW test file,
retains public source/policy/raw signed SMT, solver/dynamic-library identity and raw stdout, and verifies paired
per-test evidence. Ephemeral private signing keys are never retained. Historical
run40063652331 retains the original fixture version and an explicitly invalid
zero-test helper record; the corrected version has distinct filenames and only
attributed tests. These versions must not be represented as identical overlays.

Issue #4953 remains open: this small policy/checkpoint subset does not discharge
all branch/nested-return/shadowing/import/loop/call semantics, complete source
obligation enumeration, machine arithmetic, compiler lowering or external
assessment. The accepted57-case dossier remains at its exact earlier binary;
this audit adds no results to that record and qualifies no optimized release.


Fixture versions are explicit: original test SHA256
`21c0d0d7881c51ad1d3781a21cc9cafade04d5a217555fb9000e087a3749d375`;
strengthened distinct-filename version
`0585ffe1fe9841b4543f9c30c77642b90ab4a61528a2e1d237f26ae59a307f49`;
solver/feasibility v3
`0716138ad2aebd106a413aeeaccd6eecbb8ecbdac231f66a2a5416f8da3bd9e5`.
The third version adds three cases and explicit expected solver/feasibility
bindings to the policy factory; it preserves all original negative assertions.
The old-helper v3 capture and absent-auditor main baseline establish different
claims and must be reported separately.

## Remaining #4953 source-correspondence requirements

The helper and reviewed equality branch are one narrow improvement. Closing
#4953 requires requirements-based source-to-query evidence for every accepted
body-model family, not a larger count of auditor mutation cases. The next
technical evidence cycle must retain actual compiler queries, independently
review source/SMT checkpoints and negative controls for this inventory:

| Family | Required correspondence checkpoint | Required omission/semantic control |
| --- | --- | --- |
| Single pure return | One return substitution per declared postcondition | Wrong return expression and multiple clauses; no clause-only substitution |
| If/else | Both return substitutions under C and not C | Wrong else result, missing/duplicated branch, swapped guard |
| Early-return/fallthrough | Then return under C; tail return only under not C | Wrong tail, omitted tail and extra unreachable-tail claim |
| First-match scalar match | Ordered effective guards include exclusion of every earlier matching arm; wildcard covers residual domain | Overlapping literals/guards, false guard, missing wildcard and coherently omitted arm |
| Try/catch pure returns | Body plus each handler as explicitly conservative return alternatives | Wrong handler return and omitted handler; no exact exception-reachability claim from unconditional overapproximation |
| Leading let/assignment/shadowing | Explicit unsupported/unresolved site until a reviewed state model exists | Neighboring valid proof cannot hide a changed binding or shadowed parameter |
| Nested control/call/field return | Unsupported status is preserved; no implementation credit fabricated | A valid neighboring proof or tautological clause cannot hide the requested site |
| Imports/aliases/selected declarations | Reviewed resolved declaration identity linked to authenticated source path and import selection | Missing selected import, alias collision, extra unselected import and wrong source/declaration mapping |
| Impl methods | Method-qualified clause/path inventory, distinct from a same-named function | Missing method export beside a valid top-level proof must be visible |
| Vacuity/unresolved solver outcome | Independently reviewed entry/path-domain checkpoint; infeasible and UNKNOWN retained separately | Jointly contradictory requires, unreachable arm, timeout and unavailable/untranslated outcome cannot become reachable coverage |

These are still open acceptance requirements. The policy's source/declaration
labels are manually reviewed descriptors rather than a second independent source
enumerator. Neither this table, a signed inventory nor a solver result proves
that the producer found every obligation. Any later accepted subset must identify
actual compiler/source/solver/library/profile versions, exact expected-site
coverage, source/SMT review findings and authentic reviewer provenance. Existing
held finite/runtime changes and external safety-project acceptance remain separate.


The actual producer branch checkpoints are preserved with exact source/raw SMT
and [root technical review](reviews/40065217486/review.md), including reviewer
provenance and byte hashes. The Sol6.1-low implementation lane authored the exact
command-tree checkpoints after this source/formula inspection; these are reviewed
observations, not a priori query-oracle generation or a human assessment. The
actual-compiler test admits only two independently reviewed checkpoint sets for
this literal source: the prior Int equality sessions and the `vm-i64-wrap-v1`
zero-arity BV sessions. It does not replace actual queries with fixtures. The
caller policy still pins exact compiler/profile/source identities; source or
serialization changes require revalidation/review, not silent normalization.

Version5 frozen test SHA256:
`fb6784849f97b759565deec3ad726d3fb550dc180066891b205e0383c8a35a03`.
There are28 cases, including the original requirements, solver/feasibility and
resource reviews, manually authored BV/arity/guard controls and actual compiler
branch coverage. The watchdog uses Python's standard library rather than GNU
`timeout`; these tests target the auditor's explicit POSIX environment. No
existing main regression assertions are changed.

The seven actual old-helper defect records from hosted run40065786584 are retained under `reviews/old-helper-40065786584`, with original bundle bytes, raw stdout, frozen v5 cases, old helper source and source patch. They are distinct from the 27 absent-auditor requirement failures in the clean-main baseline. The new normal test target verifies their original evidence checksums and named failed outcomes; it does not reclassify these records as corrected-candidate results.
