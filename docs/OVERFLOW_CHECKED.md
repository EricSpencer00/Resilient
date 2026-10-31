# Machine-integer contract checks

`#[overflow_checked]` enrolls a named function's contracts in the additional
64-bit wraparound-aware check when the compiler is built with Z3:

```resilient
#[overflow_checked]
fn identity(int x) ensures x + 0 == x { return x; }
```

An unbounded-integer tautology such as `x + 1 > x` is false for signed 64-bit
wrapping arithmetic at `i64::MAX`. An enrolled function with that contract is
rejected when the machine-integer check finds the counterexample.

Enrollment belongs to the declaration. Import aliases, selective imports and
reexports preserve it; unrelated local, nested or module functions do not inherit
it merely by sharing a name. Public in-memory verification invocations keep this
metadata separate. Loop-bound and overflow-check annotations can coexist.

Use strict type checking to stop execution on verification errors. Without the
Z3 feature, this additional check does not run. The attribute does not establish
whole-program correctness, termination, timing safety, general TLA+ refinement,
or certification; unsupported obligations still require separate evidence.
