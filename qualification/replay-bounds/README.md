# Solver replay resource limits

This validation scope covers `verify-all --z3` on Linux with the `z3` feature.
It supplements the [saved-query replay record](../certificate-replay/README.md).
It does not establish a tool classification or assessor acceptance under
ISO 26262, DO-330, IEC 62304 or IEC 61508.

| Requirement | Public CLI regression |
| --- | --- |
| Reject a response received after the configured per-invocation deadline. | `replay_rejects_response_after_deadline` |
| Reject stdout exceeding 64 KiB even when it contains only UNSAT and whitespace. | `replay_rejects_oversized_whitespace_after_unsat` |
| Reject a timeout outside 1–300000 milliseconds or a malformed value. | `replay_rejects_invalid_timeout_configuration` |
| Continue accepting one clean UNSAT response within budget. | `replay_accepts_clean_response_within_budget` |

The implementation is `resilient/src/solver_replay.rs`; the frozen test input
is `resilient/tests/solver_replay_bounds.rs`. Baseline commit
`b272c109ef1ca61104eeff7f025362a08e053c4b` produced three failures and one passing
control in [GitHub Actions](https://github.com/EricSpencer00/Resilient/actions/runs/40045651817/job/120726904466).
All builds and test execution for this change use GitHub-hosted runners.

The `Replay qualification` workflow reruns each case separately against that
baseline and the candidate revision. It retains checksummed source-bound
records for 90 days and verifies three red-green attestations with matching
test inputs, commands and toolchain versions. A build failure without the
named regression outcome cannot satisfy an attestation. Archive the workflow
artifacts before retention expires for a project qualification dossier.

Passing CI and its artifacts establish only the tested behavior on the
recorded revisions. The deadline includes output collection but depends on
ordinary operating-system scheduling and process termination. Capture limits
apply separately to stdout and stderr. Cleanup targets the direct solver
child; arbitrary descendants and inherited pipe handles are outside this
scope. The delayed-response test uses a finite 300 ms fixture and an independent
five-second harness watchdog; it does not simulate every operating-system hang.

This record does not cover source-to-query correctness, all tool operational
requirements, real-time execution guarantees, or a system safety case.
