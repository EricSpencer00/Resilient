# Host certificate/replay qualification preparation

This is a concrete technical preparation dossier for the restricted operation in
[scope.md](scope.md), with four standard-specific intake profiles in
[standard-use.md](standard-use.md). It does not establish qualification,
regulatory independence, product safety classification or assessor acceptance.

The observed baseline is source `6ad36dfb4b3a9c5d8c2ff396c3d35335e649e3a3`, tree
`3f65cf36295003eaa2bc3fc0c130400e9aac38ee`, host debug `z3` executable SHA-256
`314632ab21416bfa826be8af6852744865cfef673375ac073b912d90887a4358`.
[Hosted run40055340145](https://github.com/EricSpencer00/Resilient/actions/runs/40055340145)
passed all **33 tests across seven suites**, with zero failures or ignored cases.
The compiler and full environment identity are in [binary-identity.json](binary-identity.json).
The raw hosted job log and evidence manifest both record every named outcome.
This record applies to this source/binary; it is not a statement that later main
or a release/embedded build passed these tests.

| Operational requirement | Suite | Actual passes |
| --- | --- | ---: |
| QHOST-001: contract proof export/replay | contract_replay_roundtrip | 4 |
| QHOST-002: signed manifest enumeration | manifest_authentication | 8 |
| QHOST-003: selected loop round trips | loop_certificate_roundtrip | 3 |
| QHOST-004: conservative iteration guards | finite_loop_certificate_guards | 4 |
| QHOST-005: replay resource budgets | solver_replay_bounds | 4 |
| QHOST-006: explicit replay prerequisites | solver_replay_required | 4 |
| QHOST-007: strict solver responses | solver_replay_conformance | 6 |

[requirements.json](requirements.json) enumerates every included test;
[observations.json](observations.json) binds those requirements to observed pass
outcomes. [tests/manifest.json](tests/manifest.json) retains the exact command,
clean source revision, tool versions, eight declared input hashes and parsed
outcomes. [hosted-job.log](hosted-job.log) is the complete Actions job output,
including the seven test summaries. The collector deliberately does not save
raw output, so this log is retained separately.

The downloaded Actions artifact's **entire** `SHA256SUMS` inventory was verified,
including `rz`, `.tests.lock`, binary identity and all bundle files. Its original
inventory is retained as [hosted-artifact-SHA256SUMS](hosted-artifact-SHA256SUMS).
The executable remains in artifact12544164753, identified by
[artifacts.json](artifacts.json); download and archive it before the recorded
2027-01-22 expiration. This repository retains evidence/identity, not the 59 MB
compiler binary. Verify this dossier's separate `SHA256SUMS` from its directory;
the original hosted inventory requires the downloaded executable and lock too.

The earlier run40054083771 passed its executed tests but omitted a checksummed
hidden file during artifact upload. It is **not accepted as a complete retained
baseline**; [prior-run-disposition.json](prior-run-disposition.json) preserves the
observed defect. #4945 corrected retention and the replacement download was
verified without synthesizing missing evidence.

[failure-analysis.md](failure-analysis.md) maps concrete tool failure modes to
observed controls and limits. [anomalies.md](anomalies.md) preserves unresolved
restrictions. [reproduction.md](reproduction.md) gives hosted repeat commands,
identity checks and change triggers; [change-control.md](change-control.md)
defines the review/revalidation procedure.

[package.json](package.json) is the populated four-profile intake. Its evidence
is prepared and hashed, with project classification, independent review and
external acceptance explicitly pending. The inventory checker is expected to
report `gaps`, always `certified: false`; its result does not change the observed
33 test passes or grant qualification credit. Tailor the project-specific
controls before seeking acceptance.
