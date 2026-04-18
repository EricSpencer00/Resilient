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

| Type     | Literal example   |
| -------- | ----------------- |
| `int`    | `42`, `-3`, `0`   |
| `float`  | `3.14`, `-0.5`    |
| `bool`   | `true`, `false`   |
| `string` | `"hi"`            |

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

By default the interpreter runs whatever parses, deferring
type errors to runtime. The `--typecheck` (or `-t`) flag
enables the static checker:

```bash
resilient --typecheck hello.rs
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
Type error: file.rs:3:5: let bad: int — value has type string
Error: Type check failed
```

The checker reports the first mismatch and refuses to run the
program. Without `-t`, the same program would crash at runtime
when `"oops"` hit the `println(bad)` site — same outcome,
later signal.

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
- Four primitives: `int`, `float`, `bool`, `string`.
- `--typecheck` surfaces mismatches at compile time instead of
  on the crashing line.
- Bindings are mutable; reassignment uses `=`.

## What's next

→ [3. Functions and contracts]({{ site.baseurl }}/tutorial/03-functions-and-contracts)
