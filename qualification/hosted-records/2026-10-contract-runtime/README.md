# Retained contract and runtime qualification records

These immutable records preserve hosted executions beyond artifact expiration.
They describe the tested revisions, not every later commit or target platform.

- Contract replay: [Actions run 40051711639](https://github.com/EricSpencer00/Resilient/actions/runs/40051711639). Three unchanged regressions fail on the test-only baseline and pass on the repaired source; the false-postcondition control passes in both. All three attestations verified. Z3 executable and linked-library hashes match between phases.
- Runtime: [Actions run 40050887015](https://github.com/EricSpencer00/Resilient/actions/runs/40050887015). Serial and eight-thread host runs pass, with 360 unit tests each plus integration tests and doctests. Six pre-existing doctests are ignored. This does not eliminate the intermittent failures tracked in #4907.

`sources.json` names each clean tested commit and tree. Apply its full-index
patch to the recorded base to reconstruct the tested tree; CI checks that tree
identity. The patches preserve source even if temporary PR merge refs disappear.
`SHA256SUMS` covers every retained file except itself. Evidence validation checks
bundle integrity, paired attestations, solver identities and source reconstruction.

These are tool-use evidence inputs for the ISO 26262, DO-178C/DO-330, IEC 62304
and IEC 61508 preparation packages. They do not establish assessment acceptance,
complete source-obligation coverage, generated-code correctness or target hardware
behavior. Contract proofs use the existing mathematical integer/body models.
