# Source-bound host qualification preparation

This populated dossier records the restricted host operation in [scope.md](scope.md)
and four standard-specific intake profiles in [standard-use.md](standard-use.md).
It does not establish qualification, regulatory independence, product safety
classification or assessor acceptance.

[Hosted run40060158827](https://github.com/EricSpencer00/Resilient/actions/runs/40060158827)
passed **57 named tests across nine suites**, with zero failures or ignored cases.
The exact source is `be7d1302d66bb9061e497ffc6df6d9b57660d298`, tree `dc5f656a4b3dd158424320785325f27b156da042`.
The retained Linux x86_64 debug `rz 1.2.0` executable SHA-256 is
`7bdca0c29ce7c3706bc0e89a6dc2bcc10e25308edb6b89b6ad029f71640691a9`. Its embedded commit is `be7d1302d66b`;
[binary-identity.json](binary-identity.json) records the build, linked-library hashes,
solver, compiler tools and runner image. This record applies to that binary;
the optimized 1.2 release has a separate source/build identity and receives no
qualification credit from these observations.

| Requirement | Suite | Observed passes |
| --- | --- | ---: |
| QHOST-001 | contract_replay_roundtrip | 4 |
| QHOST-002 | manifest_authentication | 8 |
| QHOST-003 | loop_certificate_roundtrip | 3 |
| QHOST-004 | finite_loop_certificate_guards | 4 |
| QHOST-005 | solver_replay_bounds | 4 |
| QHOST-006 | solver_replay_required | 4 |
| QHOST-007 | solver_replay_conformance | 6 |
| QHOST-008 | source_binding | 14 |
| QHOST-009 | source_binding_graph | 10 |

[requirements.json](requirements.json) names all 57 included tests.
[observations.json](observations.json) binds them to the pass outcomes.
[tests/manifest.json](tests/manifest.json) retains the exact command, clean source,
ten declared input hashes and parsed outcomes. Its additional `command` outcome
records command success; it is not a 58th test. [test-output.log](test-output.log)
is the explicit captured test/build stdout; [hosted-job.log](hosted-job.log) retains
the complete hosted job output. All nine zero-failure/zero-ignore summaries and
all 57 named passes were checked against the raw output.

The downloaded artifact's complete checksum inventory was verified, including
`rz`, `.tests.lock`, raw test output, identity and all evidence bundle files.
[hosted-artifact-SHA256SUMS](hosted-artifact-SHA256SUMS) is that original inventory;
this dossier's separate `SHA256SUMS` covers its committed files. Artifact12547182194
expires `2027-01-22T03:53:43Z`. The hosted packaging workflow preserves the actual
executable together with this dossier for the separately labeled permanent
`qualification-baseline-linux-x86_64-debug.tar.gz` release attachment. Publication
is a distinct step; issue #4954 tracks a verified retention receipt. This repository
contains identity/evidence rather than the executable itself.

The earlier [33-case dossier](../host-cli-2026-10/README.md) remains unchanged.
[Retained paired records](../hosted-records/2026-10-source-binding/README.md) preserve
23 source-binding baseline failures plus the legacy compatibility pass, followed
by all 24 candidate passes and 23 verified attestations. The integrated binary
above revalidated all 24 source-binding cases together with the existing 33 cases.
[prior-run-disposition.json](prior-run-disposition.json) also preserves the initial
hidden-file retention defect; no incomplete artifact is accepted as complete.

[failure-analysis.md](failure-analysis.md), [anomalies.md](anomalies.md),
[change-control.md](change-control.md) and [reproduction.md](reproduction.md)
state concrete failure controls, remaining restrictions and repeat commands.
Source identity requires `verify-all --source-root snapshot/ --pubkey public.pem --z3`;
legacy program labels alone do not authenticate source bytes. Source-to-query
translation, machine-integer correspondence, finite-loop completeness and runtime
concurrency remain outside the observed credit and tracked by #4952/#4953/#4904/#4907.

[package.json](package.json) contains hashed evidence and populated traceability.
Independent review, project classification/tailoring and external acceptance remain
pending. The inventory checker deliberately reports `gaps` and `certified: false`;
these unresolved acceptance steps do not alter the recorded test outcomes.
