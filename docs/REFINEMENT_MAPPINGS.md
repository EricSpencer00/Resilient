# TLA+ mapping validation

A top-level function can name an action in a TLA+ specification:

```resilient
@refines(spec = "controller.tla", action = "Step")
fn step(int state) { return state; }
```

The compiler checks that the specification exists, that it defines the named
action, and that extern functions found by its call analysis declare both
`requires` and `ensures`. The analysis follows direct calls, identifier aliases,
aggregate elements and conditional callable values, including bindings,
destructuring and aggregate writes. It retains every possible target across
branches and aggregate elements, so even a constant index into a mixed aggregate
can require contracts on all its callable elements. Taking an unused reference
alone does not impose an extern-contract obligation.

For each mapped root, a fixed-point analysis also propagates known named callable
values into reachable helpers' parameters and out of their returns. Anonymous
functions have distinct identities even when imported declarations share source
positions. Their captured values are tracked separately from parameters and
locals; creating an unused closure does not execute its body. Passing a closure
to an unmodeled consumer conservatively enrolls its body, including callbacks to
builtin collection methods. The analysis follows
recursive forwarding, conditional returns, and returned values stored in local
aggregates. Unrelated non-refined callers do not supply parameter targets to
that root's analysis. A helper's possible targets are joined across its reachable
calls; the analysis does not distinguish individual invocations.

Global initialization is analyzed before each mapped root. Callable values from
initializer factories, forwarded arguments, collection operations and closures
are retained in global bindings and captures. Initializer-only calls do not become
calls of the mapped function; helper parameter and return summaries are recomputed
for that root so unrelated startup arguments do not contaminate its obligations.
Global values and captures still conservatively join their possible targets.

Collection callback flow covers `map`, `filter`, `reduce`, `flat_map`,
`for_each`, `find`, `any`, and `all`. Known named or anonymous callbacks are
checked, element/accumulator targets flow into their parameters, and possible
callable results flow out of transformations and reductions. Element-preserving
chains and builtin `unwrap` retain this provenance. Both reduction forms are
supported. Lexical array-shape facts distinguish callable elements from the
callback itself, preserving ignored-element cases. Unknown receiver shapes retain
conservative field-call analysis. This analysis joins possible elements, accumulators and return values;
it does not prove collection sizes or predicate outcomes.

This is conservative value provenance, not complete higher-order verification.
Unknown caller-supplied targets and interprocedural mutation of aliased
aggregates are not fully resolved. Closure targets are joined across possible
captures and invocations, so this analysis does not distinguish individual
closure instances or prove which branch executes. It does **not** prove that the
function implements the TLA+ action.

The mapping belongs to the annotated declaration. Import aliases and reexports
preserve that ownership. Private, unselected and cfg-excluded functions do not
transfer their mappings to unrelated functions with the same name. Nested and
inline-module mappings are outside the supported top-level mapping profile and
are rejected instead of being attached to another function.

A relative `spec` path is resolved against the directory containing the annotated
source file, including through reexports. A same-named specification beside the
importing file cannot substitute for the declaration's specification.

Fileless verification through `run_program_with_options` preserves mapping ownership and extern-contract
checks but defers specification-file lookup because it has no source directory.
Use file-backed checking when specification existence and action lookup are
required evidence. Neither path establishes general semantic refinement or
certification.
