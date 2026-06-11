---
title: "2. Variables and types"
parent: Tutorial
nav_order: 2
permalink: /tutorial/02-variables-and-types
---

# 2. Variables and types
{: .no_toc }

`let`, primitives, optional annotations, and `--typecheck`.
{: .fs-5 .fw-300 }

---

## `let` introduces a local binding

```resilient
fn main() {
    let greeting = "hello";
    let count = 3;
    println(greeting);
    println(count);
}
main();
```

`let <name> = <expr>;` is the one-and-only binding form. The
expression on the right is evaluated eagerly; there's no lazy
`let` today.

## Primitive types

| Type     | Literal example              | Notes |
| -------- | ---------------------------- | ----- |
| `int`    | `42`, `-3`, `0`              | 64-bit signed |
| `float`  | `3.14`, `-0.5`               | IEEE-754 binary64 (`f64`) |
| `f32`    | `3.14 as f32`, `as_f32(1.5)` | IEEE-754 binary32 — use on embedded FPU targets |
| `bool`   | `true`, `false`              | |
| `string` | `"hi"`                       | |

`float` and `f32` are **distinct types**. Mixing them in arithmetic
without an explicit cast is a type error — the checker suggests
`as f32` or `as f64`. On Cortex-M4F, `f32` maps to the hardware
single-precision FPU; `float` is software-emulated and slower.

You can spell the type explicitly when clarity helps:

```resilient
fn main() {
    let name: string = "Resilient";
    let version: int = 1;
    let enabled: bool = true;
    println(name);
    println(version);
    println(enabled);
}
main();
```

## Turning on static type checking

By default `rz` runs the type checker in **soft mode**: mismatches
print to stderr but the program still executes. Use `--typecheck`
(or `-t`) for **strict mode** — any type error aborts before run:

```bash
rz --typecheck hello.rz
```

When every binding and call is consistent, you get a green
confirmation:

```
Running type checker...
Type check passed
```

Mismatches get caught before the program runs. Save this
broken version:

```resilient
fn main() {
    let good: int = 1;
    let bad: int = "oops";
    println(good);
    println(bad);
}
main();
```

Running with `-t`:

```
Type error: file.rz:3:5: let bad: int — value has type string
Error: Type check failed
```

The checker reports the first mismatch and refuses to run the
program. Without `-t`, soft mode prints the same diagnostic but
still runs — you get the runtime failure on the crashing line
instead of an early exit.

## Mutation and shadowing

Bindings are mutable:

```resilient
fn main() {
    let n = 0;
    n = n + 1;
    n = n + 1;
    println(n);
}
main();
```

Prints `2`. If you `let` the same name twice, the second
shadows the first — the old binding is unreachable from that
point forward. (Shadowing is common in match-arm bodies.)

## What you learned

- `let` for locals; annotations are optional and never
  required.
- Primitives: `int`, `float` (f64), `f32`, `bool`, `string`.
- `--typecheck` surfaces mismatches at compile time instead of
  on the crashing line.
- Bindings are mutable; reassignment uses `=`.

## What's next

→ [3. Functions and contracts]({{ site.baseurl }}/tutorial/03-functions-and-contracts)
