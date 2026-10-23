# Saved-query replay validation record

This record contains executed validation evidence for `rz verify-all --z3` on
Linux x86_64. It supports review of one tool operation: checking the integrity of
listed certificate files and replaying their SMT queries. It does not establish
that those queries cover the program's requirements or faithfully translate its
semantics. No safety classification, qualification level, independent approval,
or certification is assigned by this record.

## Controlled baseline

- Production baseline before repair: `532bf7033ab2efe3ba66d2b3a79398de7b14f80d`.
- Test-only red revision: see `evidence/red/manifest.json`; production code is
  unchanged from the baseline.
- Validated source revision: `821b5513382d5d471fc787c56308cf4a90388d7f`.
  Later commits in this package add records and documentation only.
- Build: `cargo test --locked --manifest-path resilient/Cargo.toml --features z3`;
  development/test profile, Linux x86_64, lockfile and toolchain identities in
  each evidence manifest. This is not validation of an optimized release binary.
- Actual CLI binary and Z3 executable SHA-256 identities, Z3 version and observed
  query results: [`evidence/real-results.json`](evidence/real-results.json).
- The real-solver evidence additionally hashes the executed `rz` binary, the
  validation procedure and all four input queries. The binary is not distributed
  in this package. Rebuilds must record their own binary identity.

The manifests record clean source revisions. [`source-identity.json`](source-identity.json)
and [`validated-source.patch`](validated-source.patch) preserve the full tested
source-tree delta against the upstream baseline, including tests and procedures,
even if the pre-merge commit IDs become unavailable. Apply the patch with
`git apply --unidiff-zero --index` to that clean baseline and compare `git write-tree` with the
recorded tree ID before reproducing.

A fleet rebase or squash merge can
change commit IDs: compare the recorded input hashes and source diff before
transferring any result to a new revision. A matching subset of input hashes does
not establish equivalence of the whole compiler. Revalidate a changed baseline.

## Operational requirements and results

[`requirements.json`](requirements.json) defines nine operational requirements
with concrete acceptance criteria and test identifiers.
[`traceability.json`](traceability.json) links each to the implementation and
recorded passing outcome, including its evidence run ID. `solver_replay_required`
and `solver_replay_conformance` prefixes identify Rust integration-test binaries;
their manifest outcome names omit those prefixes. `verify_all_smoke` is a module
within the `it` test binary. `real_solver` names come from the Python procedure.

| Requirements | Evidence bundle | Observed result |
| --- | --- | --- |
| QREP-001–003 | `evidence/required` | 4 passed |
| QREP-004–005 | `evidence/responses` | 6 passed, including multi-input fault cases |
| QREP-006 | `evidence/real` and `real-results.json` | 4 passed with actual Z3 |
| QREP-007–009 | `evidence/integrity` | 8 passed; includes 2 additional emitter/key checks |

The test-first run in `evidence/red` passed the hash-only compatibility control
and failed three new negative tests: an unavailable absolute solver path, a
solver missing from PATH, and an empty obligation list all incorrectly returned
success. The repaired implementation rejects each condition. The evidence
verifier correctly accepts this bundle as a record of a **failed** run; bundle
integrity and test success are separate facts.

The actual solver cases retain expected acceptance, process exit code, stdout,
stderr and query hash. The Rust bundles retain named outcomes and command status;
the repository evidence runner deliberately does not retain raw logs. These are
validation records, not independently signed or externally witnessed results.

## Broader regression observations

The complete compiler suites pass in both configurations:
`evidence/compiler-z3` and `evidence/compiler-default`. These run all compiler
tests without a test-name filter; ignored tests retain their existing disposition.
Formatting and compiler lint checks also passed on the NUC.

The full-workspace Z3 run failed in the runtime GPIO mock test
`input_read_isolates_pins_within_port` (`evidence/full-z3-gpio-failure`).
A serial rerun passed that section but failed the runtime integration test
`sink_serializes_parallel_writes_and_fails_closed_on_reentry`
(`evidence/full-z3-sink-failure`): the test expects concurrent success while the
sink rejects lock contention. Both are tracked in
[issue #4907](https://github.com/EricSpencer00/Resilient/issues/4907).
No runtime source or existing test was changed for this replay repair. These
failed workspace runs are retained; this record does not establish full-runtime
qualification. Quiet Cargo output records command status rather than a complete
per-test inventory for these broad runs.

## Failure analysis and disposition

| Failure mode | Impact on this operation | Detection/control and disposition |
| --- | --- | --- |
| Requested solver unavailable | Success without executing a requested check | Invocation errors now fail; QREP-001 passes |
| Empty obligation list | Vacuous success presented as solver verification | Explicit replay rejects an empty list; QREP-002 passes |
| UNSAT followed by failure, extra answers or diagnostics | False acceptance of invalid solver execution | Strict process/result checks; QREP-004–006 pass |
| Corrupted certificate or signature | Changed input accepted as original evidence | Hash/signature checks; QREP-007 passes |
| Absent or malformed manifest | Unspecified input scope | Parse/read failure; QREP-008 passes |
| Omitted obligation or incorrect source-to-SMT translation | Valid replay for an incomplete/wrong specification | **Open:** full requirement inventory and translation validation required; do not infer program correctness from replay |
| Nonterminating solver or excessive output | Validation stalls or exhausts resources | **Open:** caller must bound execution; production replay has no internal time/output limit in this baseline |
| Untrusted manifest or substituted solver/key | Successful result on attacker-selected inputs | **Open:** control the manifest, binaries and trust anchors independently; signatures do not authenticate the full requirement inventory |
| Unvalidated platform, release flags or target | Results may not transfer | **Open:** repeat validation for the intended release and environment |

Hash-only mode remains an integrity operation and can accept an empty manifest.
It must not be used as evidence that a solver ran. `--z3` still depends on the
chosen solver's correctness; rerunning Z3 is not an independent proof checker.

## Reproduction

Use a clean checkout of the validated revision, the recorded Rust toolchain,
locked dependencies and an installed Z3. Substitute your controlled paths; save
outputs outside the checkout so generated records do not dirty the source.

```sh
cargo test --locked --manifest-path resilient/Cargo.toml --features z3 --test solver_replay_required
cargo test --locked --manifest-path resilient/Cargo.toml --features z3 --test solver_replay_conformance
cargo test --locked --manifest-path resilient/Cargo.toml --features z3 --test it verify_all_smoke
python3 agent-scripts/e2e-evidence.py run --output /tmp/replay-real \
  --input resilient/target/debug/rz \
  --input qualification/certificate-replay/run-real-solver.py \
  --input qualification/certificate-replay/queries/unsat.smt2 \
  --input qualification/certificate-replay/queries/sat.smt2 \
  --input qualification/certificate-replay/queries/multiple.smt2 \
  --input qualification/certificate-replay/queries/invalid.smt2 -- \
  python3 qualification/certificate-replay/run-real-solver.py \
  --rz resilient/target/debug/rz --solver /usr/bin/z3 \
  --output /tmp/replay-real-results.json
python3 agent-scripts/e2e-evidence.py verify /tmp/replay-real
```

Wrap each Cargo command with `e2e-evidence.py run` and then `verify` as recorded
in the corresponding evidence manifest. The Python procedure gives each real
query 30 seconds; this is a test harness limit, not a compiler guarantee.
Validate each retained bundle with `e2e-evidence.py verify <bundle-directory>`.
The package-level `SHA256SUMS` also covers the requirements, procedures, real
observations and evidence files; it detects accidental changes, not forgery.

## Use in the four assessment packages

These are evidence reuse boundaries, not a claim that standards share identical
objectives. Apply the licensed standards and the project's agreed assessment
basis when assigning credit.

| Assessment | What this record supplies | Project-specific work still required |
| --- | --- | --- |
| ISO 26262 | Tool operational requirements, fault cases, controlled validation results and anomalies | Intended-use TI/TD rationale and derived TCL, selected qualification methods, ASIL/item context, safety plan, independent review and acceptance |
| DO-178C / DO-330 | Bounded operation requirements, test procedures/results and configuration records | Applicant's tool criteria/TQL, qualification plans and objective coverage; DO-333 formal-methods credit and translation assumptions where used |
| IEC 62304 | Intended-operation validation observations and known failure modes | Manufacturer's QMS validation, software classification, lifecycle/risk-management linkage and release approval |
| IEC 61508 | Support-tool behavior, failure analysis and validation observations | Support-tool classification, SIL/methods rationale, lifecycle responsibilities and required assessment/independence |

Review status is **not independently reviewed**. There is no fabricated reviewer,
closed safety anomaly, or accepted qualification package. Supply this record as
an artifact in a project-specific [evidence intake](../README.md), preserve the
open issues, and obtain accountable review before assigning safety credit.

Revalidation is required after changes to replay, certificate/signature handling,
SMT generation, solver/toolchain versions, build flags, platform, dependencies,
requirements or intended use. Record the impact assessment and rerun affected
focused tests plus the applicable full regression suite. Approval and release
of a project-specific qualified baseline remain separate controlled activities.
