---
title: "3. Functions and contracts"
parent: Tutorial
nav_order: 3
permalink: /tutorial/03-functions-and-contracts
---

# 3. Functions and contracts
{: .no_toc }

`fn` + `requires` + `ensures` + `--audit`.
{: .fs-5 .fw-300 }

---

## Functions

Declare a function with `fn`, typed parameters, and an optional
return type:

```resilient
fn add(int a, int b) -> int {
    return a + b;
}

fn main() {
    println(add(2, 3));
}
main();
```

- Parameter syntax is `<type> <name>`, comma-separated.
- `-> int` is the return type; without it the checker infers
  `Void`.
- `return <expr>;` is the only way to exit with a value. There
  is no trailing-expression implicit return (that's a noise-
  reducing policy choice, not a limitation).

## Contracts: `requires` and `ensures`

Resilient lets you attach **pre-conditions** (`requires`) and
**post-conditions** (`ensures`) to a function. They're Boolean
expressions evaluated at call time; they aren't comments, and
they aren't conditional logic — they're invariants the
language checks for you.

```resilient
fn safe_div(int n, int d) -> int
    requires d != 0
    ensures result >= 0 || result < 0
{
    return n / d;
}

fn main() {
    println(safe_div(10, 2));
}
main();
```

- `requires d != 0` — before the body runs, `d` must not be
  zero. If it is, the runtime aborts with a contract
  violation.
- `ensures result >= 0 || result < 0` — after the body runs,
  the special `result` binding must satisfy the clause. This
  one's a tautology on purpose, to show the shape; real
  post-conditions encode something about the output.

## `--audit` tells you what got discharged statically

Many `requires` clauses can be proved at compile time. A
trivial one — `requires 1 + 0 == 1` — is a tautology. A less
trivial one — `requires x + 0 == x` for any `x` — still folds
away with a bit of algebra.

The `--audit` flag dumps a table of how many contract sites
the compiler discharged statically versus the number it
deferred to runtime:

```bash
resilient --typecheck --audit your_file.rz
```

Try it on this program:

```resilient
fn double(int x) -> int
    requires x >= 0
    ensures result >= 0
{
    return x * 2;
}

fn main() {
    println(double(21));
}
main();
```

The audit reports:

```
--- Verification Audit ---
  contract decls (tautologies discharged): 0
  contracted call sites visited:           1
  call-site requires discharged statically: 1 / 1
  call-site requires left for runtime:      0 / 1
  static coverage:                          100%
```

Because we passed `double(21)` — a non-negative literal — the
compiler folds the `requires x >= 0` clause at compile time
and strips the runtime check. 100% static coverage.

If you change the call to `double(some_variable)` where
`some_variable` has an unknown value, the discharger can't
prove non-negativity; the runtime check stays in and static
coverage drops to 0%.

## What you learned

- `fn <name>(<type> <arg>, …) -> <type> { … }` — typed
  parameters, optional return type, explicit `return`.
- `requires` / `ensures` — compiler-checked boolean
  invariants, not inline assertions.
- `--audit` shows the static-vs-runtime coverage per
  contracted call site.

## What's next

→ [4. Live blocks]({{ site.baseurl }}/tutorial/04-live-blocks)
