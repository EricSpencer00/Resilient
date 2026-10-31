# Loop bounds

Once a function is enrolled (via either `@require_contracts` variant),
any `while` loop in its body must carry a `#[loop_bound(N)]` attribute
naming its maximum iteration count:

```resilient
@require_contracts(strict)

#[loop_bound(100)]
fn count_up(int n) -> int requires n >= 0 requires n <= 100 ensures result >= 0 {
    let i = 0;
    while (i < n) {
        i = i + 1;
    }
    return i;
}
```

Omit `#[loop_bound(N)]` on an enrolled function with a `while` loop and
the compiler reports:

```
error[loop_bound]: function `count_up` enrolled in contract verification: contains
a while-loop and requires #[loop_bound(N)] (RES-3780 Tier 2)
```

With `--features z3`, the compiler additionally tries to *prove* (or
refute) the declared bound for loops that follow a simple
monotonic-counter shape (`while (i < n) { i = i + step; }`-style). A
bound the prover can't statically match falls back to a runtime check
instead of a hard compile error — you get a warning, not a failure.

The interpreter and bytecode engines enforce resolved `#[loop_bound(N)]`
attributes even without contract enrolment or Z3. Each activation of each
`while` loop has its own budget. The condition is evaluated normally; if it
would enter body iteration `N + 1`, execution fails with `loop bound exceeded`
before that body's side effects. `continue` consumes an iteration; a normal
exit or `break` after exactly `N` iterations succeeds. CLI JIT execution falls
back to the guarded bytecode path before running a bounded function.

Static estimates require integer bounds, stable precondition variables and a
single unconditional positive counter update whose arithmetic cannot overflow.
Inclusive comparisons account for the extra iteration. Conditional updates,
overwritten counters and other unsupported shapes emit a warning and retain
the runtime guard. Generated guards are installed after source verification;
they do not establish source invariants.

This is a per-loop body budget, not a whole-program termination or timing
proof. Recursion, condition evaluation and repeated loop activations need
separate analysis. Contract enrolment requires the annotation regardless of
whether `@ai_generated` is attached.


Bounds belong to parsed declarations, including their lexical parent functions
and source positions. Imports carry the metadata of selected declarations through
namespace aliases and reexports. Private, unselected, or conditionally excluded
declarations cannot give a bound to an unrelated function with the same name.
Each imported source is collected in a separate scope; explicit inner-function
bounds retain their ownership through cloning and generic specialization.

The argument must be one integer literal from `1` through `4294967295`.
Zero, negative numbers, floats, expressions, missing arguments, and extra
arguments are parser errors. The attribute applies to named function
declarations. Repeating an identical bound is allowed; conflicting bounds on
one declaration are rejected. These errors stop execution even without strict
type checking, including when the declaration is imported.
