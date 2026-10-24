---
title: Qualification preparation
parent: Standards
nav_order: 5
permalink: /qualification-readiness
---

# Prepare a qualification evidence package

Resilient provides an evidence-intake template and checker for ISO 26262,
DO-178C/DO-330, IEC 62304 and IEC 61508 preparation. No qualification or
certification is granted by these materials.

Start with the [package instructions](https://github.com/EricSpencer00/Resilient/tree/main/qualification)
and [template](https://github.com/EricSpencer00/Resilient/blob/main/qualification/package-template.json).
Keep completed assessment records in the project's controlled evidence repository.

## Review sequence

1. Agree intended use, applicable standard editions, safety classification,
   toolchain boundaries and acceptance criteria with the responsible safety lead.
2. Freeze the source and binary baseline, dependencies, target, backend, build
   flags and solver configuration. Define supported features and exclusions.
3. Evaluate tool failure modes and detection measures. Establish the applicable
   qualification or validation approach for that use.
4. Trace requirements to implementation, positive/negative tests and raw results.
   Include unsupported constructs, unknown/timeouts, fault injection, known
   anomalies, target behavior and proof-translation checks.
5. Obtain independent review, resolve findings, and run the intake checker.
6. Submit the reviewed package through the project's assessment process. Record
   external acceptance, restrictions and change-triggered revalidation separately.

## What the checker establishes

It rejects missing controls, unreviewed evidence, unresolved issues, broken
trace references, missing files and mismatched hashes. `intake-complete` means
that the listed inventory passes these checks. It does not establish technical
adequacy, regulatory compliance or authentic assessor approval.

The supplied template starts with open gaps. A green synthetic test fixture
is a software test, never evidence that Resilient is qualified.

## Generated specifications

Generated specifications need a requirement source, review history, explicit
assumptions and validation against both correct and deliberately incorrect
implementations. A TLA+ model-checking result and a Resilient contract proof cover
different models. Claiming a relationship requires a reviewed refinement argument
and evidence for any translation used between them.

## Available validation evidence

The [saved-query replay validation record](https://github.com/EricSpencer00/Resilient/tree/main/qualification/certificate-replay)
contains executed positive and negative tests, real Z3 results, source and binary
identities, requirement traceability and failure analysis. It covers the saved-query
replay operation on a pinned Linux baseline. Program-level proof completeness,
other release targets and independent assessment remain outside that record.

The [replay resource-limit record](https://github.com/EricSpencer00/Resilient/tree/main/qualification/replay-bounds/hosted-record)
retains the hosted baseline and repaired runs, three verified red-green
attestations, and a source reconstruction patch. It covers timeout enforcement,
output limits and configuration rejection, with a passing-response control.
