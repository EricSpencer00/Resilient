# Finite controller assurance profile

## Intended use and boundary

This profile supplies development-time evidence for a reviewed, synchronous,
finite controller with one state variable and one event per step. The generated
function is a pure lookup table encoded with integer comparisons. The profile
covers the host interpreter and the match/direct bytecode dispatch engines. It
does not qualify a compiler release, a medical device, an automotive item, an
avionics application or an embedded target.

The operational interface is the grammar and CLI in [README.md](README.md).
The full requirements input, integer encoding, initial state, total transition
relation, properties and fairness premise are controlled artifacts. A change to
any of these, the generator/checker, compiler, TLC jar or execution environment
requires a new evidence run and impact review. Old records remain historical.

## Requirements and verification trace

| ID | Acceptance criterion | Executed verification entry point in `tests/test_cli.py` |
| --- | --- | --- |
| RP-01 | Consume all nonblank input; reject unknown sentences, symbols, duplicate IDs/pairs, partial tables and unreachable states. | `test_rejects_ambiguous_partial_unknown_and_unreachable_requirements` |
| RP-02 | Identical inputs and generator produce identical artifact bytes; the fixture table matches a separately written oracle. | `test_deterministic_outputs_and_independent_transition_oracle` |
| RP-03 | Reject changed artifacts even when their manifest hashes are changed to match. | `test_bundle_tampering_is_not_accepted`, `test_rehashed_tampering_and_stale_evidence_are_rejected` |
| RP-04 | Compare every declared pair with actual interpreter, match VM and direct VM output; retain requirement-level results. | `test_every_transition_executes_under_interpreter_and_both_vm_dispatches` |
| RP-05 | Detect a wrong implementation and every single-cell mutation of the reference controller. | `test_wrong_implementation_fails_actual_output_comparison`, `test_each_transition_mutation_is_detected` |
| RP-06 | Require actual TLC completion, satisfied safety/liveness properties and an observed transition relation equal to the declared table. | `test_real_model_and_runtime_agree_and_contradictory_property_fails` |
| RP-07 | Missing tools and timed-out executions never pass; preserve partial output and failure status. | `test_missing_compiler_cannot_pass`, `test_timeout_is_failure_with_retained_output` |
| RP-08 | Refuse to overwrite an existing evidence directory, including a prior report. | `test_rehashed_tampering_and_stale_evidence_are_rejected` |
| RP-09 | Check original requirements against IR, pure generated code and TLA/configuration with a separate implementation; reject consistent generator faults. | `tests/test_independent.py`: all four tests, including five translation faults and seven artifact mutations. |

The Cargo integration test invokes these public CLI tests with the actual built
compiler. Its run alone does not satisfy RP-06 when TLC is unavailable. The
standalone command with `TLC_JAR` exercises the full set. Record the test summary,
including skips, together with the retained workspaces; a green subset must not
be relabeled as the full campaign.

## Failure analysis

| Failure | Detection or restriction | Residual obligation |
| --- | --- | --- |
| Requirements encode the wrong intent or omit a hazard | Strict grammar removes syntactic ambiguity; properties are separate sentences. | Human requirements/hazard review and completeness argument. |
| Generator and checker share an incorrect translation | Separate source/artifact checker, independent frozen fixture oracle, actual TLC relation, separate runtime executions and single-cell mutations. | Review grammar semantics and translation; additional application-specific adversarial cases. This is not a universal translation proof. |
| Model passes while code implements a different table | Exhaustive state/event observations for the generated pure function; exact TLC relation comparison. | Any candidate with hidden state/effects needs additional semantics and sequence coverage. |
| Liveness depends on an unjustified scheduler | Explicit weak fairness; reject different simultaneous fairness premises. | Establish the real scheduler assumption before deployment. |
| Evidence is stale, altered or incomplete | Fresh directories, regenerated bundle comparison, tool/content hashes, timeout/nonzero/status checks, raw logs. | Controlled storage and authentic review; hashes do not authenticate authors. |
| A host result is applied to another target or backend | Report and profile identify the host-only boundary. | Target-specific verification, timing/resource analysis and tool-use assessment. |
| Checker, TLC, interpreter and VM share or contain defects | Negative controls and two checking mechanisms provide limited diversity. | Trusted-base assessment and independent validation. None of these tools is qualified by this campaign. |

## Standards preparation crosswalk

These are evidence categories, not normative clause claims or an exhaustive
objective matrix. The applicable licensed editions and project assessment basis
must be reviewed before assigning compliance credit.

| Preparation profile | This evidence can contribute | Still requires project-specific assessment |
| --- | --- | --- |
| [ISO 26262-8:2018](https://www.iso.org/standard/68390.html) | Requirements trace, configuration identities, verification and tool-error detection observations. | Intended item/use, ASIL context, TI/TD/TCL rationale, qualification approach, confirmation measures and independent acceptance. |
| [IEC 61508-3:2010](https://webstore.iec.ch/en/publication/5517) | Software/tool behavior requirements, verification records, explicit limitations and failure analysis. | Lifecycle/system context, SIL, support-tool assessment, methods/competence/independence and functional safety assessment. |
| [DO-178C/DO-330 and formal-methods supplement](https://www.faa.gov/airports/resources/advisory_circulars/index.cfm/go/document.information/documentNumber/20-115D) | Operational requirements, model assumptions, finite verification evidence and negative controls. | Applicant's software level, tool criteria/TQL, approved plans, requested DO-333 credit and authority acceptance. |
| [IEC 62304](https://committee.iso.org/standard/38421.html) | Development-tool validation inputs and traceable verification observations. | Manufacturer/QMS context, software classification, device hazard/risk controls, lifecycle and maintenance acceptance. |

Technical preparation is maintained in the repository. Requirements acceptance,
independent technical review, licensed-standard objective tailoring and any
external assessment remain unapproved. Populate authentic review records through
[the qualification intake process](../README.md); do not manufacture reviewer
identities or set controls to `reviewed` merely because this test suite passes.
