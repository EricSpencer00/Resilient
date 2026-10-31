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

This is conservative value provenance, not a complete higher-order analysis:
callable parameters and values returned by arbitrary function calls are not fully
resolved. It does **not** prove that the function implements the TLA+ action.

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
