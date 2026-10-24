# Tool-operation failure analysis

This analysis concerns the scoped host certificate/replay operation. The possible
effect is false confidence in a contract result or inability to reproduce a
verification result. A consuming project must determine whether that effect can
propagate into its safety decisions and whether independent verification detects
it. No tool-impact, detection, confidence, assurance or safety level is assigned
without that project context.

| Failure mode | Observed detection/control | Remaining limit and required use restriction |
| --- | --- | --- |
| Implementation return proof omitted from ordinary bundle | QHOST-001 exercises straight, constant and branch-specific export and consumer replay, with a false-postcondition control. Historical #4927/#4934 records preserve the defect and repair. | Selected examples do not prove complete source-obligation enumeration. Review accepted operational requirements and all applicable obligations. |
| Signed manifest entry removed/reordered or label changed | QHOST-002 checks strict signed enumeration, missing/wrong signatures and compiler-produced manifest signing. | Use the separately tested `--source-root` mode for source-content identity; a signed label alone is insufficient. Accepted requirements and source-to-query correctness still require separate review. |
| Entry/import content or frozen graph is substituted | QHOST-008/QHOST-009 check signed parsed-buffer hashes, relocation, entry/import tamper, missing/unsigned/malformed inventories, complete transitive graph/reachability, duplicate/traversal/symlink rejection and cache equivalence. | Requires controlled stable snapshots and a trusted producer/key; package resolution configuration and atomic filesystem access are not claimed. |
| Solver absent or no obligations to replay | QHOST-006 checks explicit rejection; hash-only mode is separately tested. | Hash-only success cannot be promoted to solver proof. Require the intended verification mode and reject unresolved obligations. |
| Solver exits unsuccessfully or returns diagnostic/malformed/SAT/unknown output | QHOST-007 checks process status and strict clean-UNSAT response acceptance. | A syntactically accepted result covers only the emitted formula and selected solver configuration. Source-to-query correctness remains outside replay validation. |
| Replay hangs or floods output | QHOST-005 checks time/output budget rejection and an in-budget passing control. | Bounds are host-process safeguards; they are not embedded timing or WCET evidence. Timeout means unresolved, never proof. |
| Loop example receives unjustified proof | QHOST-003/QHOST-004 cover listed positive/negative cases and conservative guards at the recorded baseline. | No general loop semantic completeness; retain the exact baseline-specific restrictions in scope.md and revalidate semantic changes. |
| Mathematical proof is applied to overflowing machine arithmetic | No machine-width correspondence is established by these suites. | Exclude that credit and independently verify arithmetic semantics on the consuming backend/target. |
| Wrong compiler, solver or source is substituted | Binary/source/environment identity, declared input hashes, complete archive checksums and raw hosted outcome records permit substitution checks. | Hashes establish identity/integrity, not correctness or authentic regulatory approval. Verify against the accepted baseline before use. |
| Exported archive omits a checksummed record | Initial run40054083771 exposed hidden-lock omission; correction #4945 enables complete generated-directory retention; the replacement run40055340145 downloaded inventory, including `.tests.lock`, was verified in full. | Accept only an artifact whose downloaded checksum inventory is complete. Retain the initial disposition as a process anomaly. |
| Runtime scheduling interference invalidates runtime confidence | Historical and refreshed serial/eight-thread and diagnostic records retain both observed failures and successes under #4907. | This dossier grants no deployed-runtime/concurrency credit; intermittent failures are not erased by later passes. |

For ISO 26262, use these failure modes as inputs to an intended-use-specific
impact/detection analysis and qualification-method decision. For DO-330, use them
when determining which tool outputs are relied upon and which verification is
replaced or reduced. For IEC 62304, link them to the manufacturer's tool-use
validation and product hazard/risk controls. For IEC 61508, use them in the
support-tool failure-impact/confidence assessment. Those decisions and their
independent/project acceptance remain pending rather than inferred from tests.
