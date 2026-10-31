# Qualification evidence intake

This package prepares evidence for review of a **specific Resilient version and
intended use**. It is not a qualification kit accepted by an assessor, a compliance
checklist, or a certificate. The starter deliberately reports gaps. Python 3.10+
is required; no additional packages are needed.

1. Copy `package-template.json` into a private assessment directory. Select only
   the applicable `standards` identifiers; use separate packages for materially
   different targets, backends, versions or intended uses.
2. Record the full source revision, compiler binary SHA-256, target, backend,
   build flags and solver version (`none` when no solver is used). In the baseline
   evidence, include dependency locks, build environment, runtime, linker,
   optimization settings, solver configuration and reproducible build commands.
3. Assign each applicable control an owner. Place the actual evidence files
   beneath the package directory and record their relative paths and SHA-256
   digests in `artifacts`. Include the review records themselves as artifacts.
4. Have a separate reviewer evaluate evidence against the project's accepted
   objectives. Record their identity and review reference; change `status` to
   `reviewed` only after review. Independence here is a project intake rule;
   differing strings alone do not establish regulatory independence.
5. Add one traceability record per requirement, naming source, test, result and
   the reviewed `evidence_control` containing the test report. Put multiple tests
   in the referenced report. Resolve open issues or revise the scope with a
   documented safety rationale before removing them from `open_issues`.
6. Run from the repository root:

   ```sh
   python3 scripts/check_qualification.py /path/to/assessment/package.json
   python3 -m unittest discover -s scripts -p test_check_qualification.py -v
   ```

Exit 1 means gaps or invalid input. Exit 0 means **intake-complete**: listed files
exist, hashes match, required records are populated and no open issues are
listed. Both outcomes contain `"certified": false`. The checker cannot establish
truth, technical adequacy, completeness of the requirement set, authentic review,
correct source-to-proof translation, or assessor acceptance. It is itself an
unqualified support tool; review its output before relying on it.

## Evidence controls

These are Resilient preparation categories, not an exhaustive reproduction of
standards clauses. Tailor them using the applicable licensed standards and the
agreed certification or assessment basis.

| Control | Evidence to supply |
| --- | --- |
| intended-use | Operations used, tool inputs/outputs, supported language subset, excluded features, safety credit sought, users and operating environment |
| baseline | Versioned toolchain inventory, binary hashes, build/reproduction procedure and supported target/backend matrix |
| requirements | Unique tool operational requirements; acceptance criteria; source and design traceability; complete requirements inventory |
| validation | Positive and negative tests, expected/actual outcomes, raw logs, coverage and target results; unsupported constructs, timeout/unknown, invalid certificates and fault injection |
| anomalies | Known defects, safety impact, mitigations, restrictions and release disposition, including accepted residual anomalies |
| change-control | Review, configuration identification, release approval, impact analysis and revalidation procedure |
| independent-review | Review scope, reviewer competence/independence, findings, resolutions and signed approval records |
| tool-confidence | ISO 26262 intended-use-specific TI/TD rationale, derived TCL, workflow error-detection measures and their validation |
| qualification-method | ISO 26262 qualification need, selected methods, applicability, validation results and report |
| safety-plan | ISO 26262 safety responsibilities, ASIL and item context, confirmation measures and assessment entry criteria |
| tool-qualification | DO-178C/DO-330 tool criteria, proposed TQL where applicable, operational requirements, qualification plan and results |
| formal-methods-credit | DO-333 objectives and credit sought, formalization review, assumptions, soundness limits and complementary verification |
| certification-plan | Applicant's agreed DO-178C planning data, objective matrix, liaison and approval records |
| tool-validation | Medical-device manufacturer's intended-use validation under its applicable QMS; tool failures, detection and change impact |
| risk-management | Medical software hazards, risk controls, verification and residual-risk review linked to ISO 14971 process |
| software-classification | IEC 62304 classification rationale, applicable lifecycle activities, delivered software/SOUP inventory and maintenance responsibilities |
| support-tool-assessment | IEC 61508 support-tool classification, failure impact, tool confidence measures and evidence |
| sil-methods | IEC 61508 target SIL, justified methods combination, evidence and independence requirements |
| functional-safety-plan | IEC 61508 lifecycle ownership, verification/validation, assessment and change management |

## Specification-generation research

Treat generated natural-language requirements, TLA+ models and Resilient
contracts as candidate specifications requiring review. Retain the original
requirement, generation inputs/configuration, candidate, reviewer corrections,
accepted specification and validation results. Check vacuous/inconsistent
assumptions and deliberately wrong implementations. Record model-checker bounds
and liveness/fairness assumptions separately from SMT results.

A passing TLA+ model does not prove a Resilient implementation refines it. Such
credit needs an explicit refinement relation and validated translation or an
independent implementation check. Benchmark performance and an informal research
conversation are not qualification evidence or an industry endorsement.

## Executed validation records

The [saved-query replay record](certificate-replay/README.md) contains a real
red/green regression run, operational requirements, failure analysis and actual
Z3 observations for a pinned Linux baseline. Its scope and remaining gaps are
explicit; it is an input to review, not an approved qualification package.

The [host certificate/replay dossier](host-cli-2026-10/README.md) binds 33 actual
hosted test outcomes to a retained debug compiler, exact source/environment,
seven operational requirements, failure analysis and four populated preparation
profiles. Independent/project acceptance remains pending. Its source-bound
restrictions and original incomplete-archive disposition are retained.

The [source-input binding record](hosted-records/2026-10-source-binding/README.md)
retains 24 new CLI cases on baseline and candidate, with 23 verified red-green
attestations and a passing legacy control. The binary-baseline workflow now
includes both source-binding suites with the seven earlier suites and records
raw test output, compiler version and resolved dynamic-library identities.

The [source-bound 1.2 host dossier](host-cli-source-binding-2026-10/README.md) records the retained debug binary and all 57 passes across nine suites from hosted run40060158827. It preserves the earlier baseline and leaves project classification, independent review and external acceptance pending.

The [controlled-requirements pipeline](requirements-pipeline/README.md) generates a
bounded finite-state TLA+ model and executable transition function, checks the
actual model transition relation, and compares every table cell with interpreter
and both VM outputs. It retains per-requirement observations and rejects altered
bundles, contradictory properties, wrong implementations and incomplete runs.
Its finite-instance evidence does not establish general compiler refinement or
replace requirements review.
