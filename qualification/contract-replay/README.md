# Implementation postcondition replay evidence

The `ensures_implementation` records in `--emit-certificate` bundles retain
independent SMT queries for the existing symbolic body model. Each record names
the function and declared ensures-clause index; its index identifies the modeled
return path. Every modeled path must prove before any record for that clause is
exported. Constant and cached verdicts are regenerated with actual solver input.

The hosted contract replay workflow compares the test-only baseline
`c9bf76695638eeccd0dd6cc3b0f8aac42e1d3694` against the candidate using identical
CLI tests and pinned Rust 1.99.1. Three regressions cover straight returns,
branch-specific returns and constant returns; a false postcondition remains a
rejected control. The tests require signed-manifest verification, actual Z3
replay, exact path counts and one solver session per query. Source-bound evidence
and independently verified red/green attestations are retained for 90 days.

These queries use the current translator's mathematical integer model and the
existing symbolic evaluator's supported pure bodies. They do not establish
machine-integer overflow behavior, compiler/code-generation correctness, or
coverage of unsupported bodies. Clauses without `result`, unsupported models,
unknown verdicts and missing certificates are not represented as implementation
proofs. Consumers still need a separately reviewed inventory of required source
obligations; authenticating an exported enumeration does not establish that it
is complete. Qualification acceptance remains scoped to the assessed tool use.
