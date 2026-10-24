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
source graph and checks the trusted policy bytes/profile. It then invokes that
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
- `profile`: accepted model identifier, matched against the explicit CLI option.
- `source_graph_sha256`: canonical signed `source_binding` graph digest.
- `sites`: independently enumerated source-relative `source`, qualified
  `declaration`, zero-based `clause` and stable `path` descriptors. All four form
  site identity; names alone do not distinguish imports/methods/overloads.
- Each `required` site has an exact emitted `obligation` reference (`fn`, `kind`,
  `idx`) and `commands`: the independently reviewed complete SMT command tree.
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
sessions contain only `set-logic`, `declare-const`, `assert` and one final
`check-sat`. After checkpoint comparison, the auditor separately checks SAT of
the antecedent with the final negated goal removed. UNSAT antecedents are
infeasible and UNKNOWN/error outcomes unresolved; neither grants reachable-site
credit. This is formula-domain feasibility, not independently established source
reachability.

## Execution limits and evidence

The supported auditor environment is a POSIX host with Python3 and the configured
Z3 executable; the qualification scope remains the separately identified host
baseline. It bounds the entire audit to0.1..60seconds plus bounded process cleanup,
subprocess output to512KiB, policy/manifest documents to2MiB, required sites and
manifest obligations to512, each query to1MiB/100,000tokens/64nesting levels and
all reviewed query bytes to16MiB. Subprocesses use argument vectors, bounded
nonblocking input/output and a new process group; timeouts/output overflow kill
that group including ordinary solver descendants. This is not containment of a
malicious trusted executable that deliberately escapes its process group.

Use a controlled stable filesystem snapshot. Compiler/manifest/query drift checks
are performed, but ordinary filesystem check/open operations are not atomic
against adversarial concurrent mutation. This is not a filesystem isolation claim.

The paired GitHub-hosted workflow is
`.github/workflows/proof-coverage-qualification.yml`. It pins baseline
`9f1d870acc5d5ba963a7e27d522e60aa70f7213e`, overlays the frozen NEW test file,
retains public source/policy/raw signed SMT and raw stdout, and verifies paired
per-test evidence. Ephemeral private signing keys are never retained. Historical
run40063652331 retains the original fixture version and an explicitly invalid
zero-test helper record; the corrected version has distinct filenames and only
attributed tests. These versions must not be represented as identical overlays.

Issue #4953 remains open: this small policy/checkpoint subset does not discharge
all branch/nested-return/shadowing/import/loop/call semantics, complete source
obligation enumeration, machine arithmetic, compiler lowering or external
assessment. The accepted57-case dossier remains at its exact earlier binary;
this audit adds no results to that record and qualifies no optimized release.
