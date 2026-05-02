---
title: Syntax Reference
parent: Language Reference
nav_order: 1
permalink: /syntax
---

# Syntax Reference
{: .no_toc }

The full Resilient grammar in one page.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Functions

Parameters carry explicit types; zero-parameter functions use
empty parentheses.

```rust
fn main() {
    println("Hello, world!");
}

fn add(int a, int b) {
    return a + b;
}
```

Functions can be defined in any order — forward references work:

```rust
fn caller() { return callee(); }
fn callee() { return 42; }
```

### Return types

A `-> TYPE` annotation is optional. When omitted the return type
is inferred from the body. Writing it out is still checked
against the body:

```rust
fn square(int x) -> int { return x * x; }
fn square(int x)        { return x * x; }  // identical, inferred

fn log_once(string msg) { println(msg); }  // void, inferred
```

Parameter types are always required — inferring them from
call-site usage produces errors at callers instead of the
definition, which is worse DX.

### Generic functions

Type parameters are declared with `fn<T, U> name(...)`:

```rust
fn identity<T>(T x) -> T { return x; }
fn swap<A, B>(A a, B b) { return [b, a]; }
```

Type parameters are currently parse-and-AST only; the
typechecker uses the HM scaffolding from RES-122 and
monomorphizes at the call site. Constraints (`fn<T: Trait>`)
are a future extension.

### Contracts

`requires` clauses are checked on entry; `ensures` clauses on
exit. Inside `ensures`, the special identifier `result` refers
to the return value.

```rust
fn safe_div(int a, int b)
    requires b != 0
    ensures  result * b == a
{
    return a / b;
}
```

With `--features z3`, the verifier tries to discharge each
clause at compile time. What it can't prove becomes a runtime
check. See [Philosophy → Verifiability](philosophy#2-verifiability)
for the certificate story.

---

## Variables

```rust
let x = 42;
let name = "Resilient";
x = x + 1;        // reassignment requires the name to be declared
```

### Static variables

`static let` bindings persist across function calls. They're
the MVP stand-in for global state:

```rust
fn tick() {
    static let n = 0;
    n = n + 1;
    return n;
}
// tick() → 1, then 2, then 3
```

---

## Live blocks

The headline feature. Code inside a live block re-executes
when a recoverable error fires inside, restoring the block's
local state to its last-known-good snapshot.

```rust
live {
    let sensor_value = read_sensor();
    assert(is_valid_reading(sensor_value), "Invalid reading");
    process_data(sensor_value, threshold);
}
```

Optional `invariants:` clauses are checked after every
iteration; a failed invariant triggers the same retry path
as a body-level error.

---

## Assertions

Assertions halt with a diagnostic. For comparison conditions
both operand values appear in the error message:

```rust
assert(fuel >= 0, "Fuel must be non-negative");
// ASSERTION ERROR: Fuel must be non-negative
//   - condition -5 >= 0 was false
```

### Runtime assumptions

`assume(expr)` is like `assert` but communicates intent to the
verifier rather than enforcing a check on every execution path.
At runtime it traps if `expr` is false, just like `assert`.
When built with `--features z3`, the SMT verifier treats the
expression as an **axiom** — it is assumed to hold rather than
proved (verifier integration planned in RES-235).

```rust
assume(x > 0);               // asserts x is positive; verifier treats as axiom
assume(x > 0, "must be positive");  // optional message shown on trap
```

---

## Data types

| Type     | Notes                                                              |
|----------|--------------------------------------------------------------------|
| `int`    | 64-bit signed. Decimal (`42`), hex (`0xFF`), binary (`0b1010`). Underscore separators allowed: `0xDEAD_BEEF`. |
| `float`  | 64-bit IEEE-754                                                    |
| `string` | UTF-8 text; `len(s)` returns scalar count                          |
| `bytes`  | Raw byte sequence; `b"\x00\x01abc"` literal                        |
| `bool`   | `true` / `false`                                                   |

### Numeric coercion

Resilient does not implicitly coerce between numeric types.
Mixing `int` and `float` in arithmetic or comparisons is a
type error. Use the explicit converters:

```rust
let a = 1 + 2.0;              // ERROR: Cannot apply '+' to int and float
let b = to_float(1) + 2.0;    // ok → float 3.0
let c = 1 + to_int(2.0);      // ok → int 3
```

| Signature | Semantics |
|---|---|
| `to_float(int) -> float` | Exact widening (for `abs(x) < 2^53`) |
| `to_int(float) -> int` | Truncate toward zero; `NaN` / `±∞` / out-of-range are runtime errors |

---

## Operators

| Category   | Operators                                  |
|------------|--------------------------------------------|
| Arithmetic | `+`  `-`  `*`  `/`  `%`                    |
| Comparison | `==`  `!=`  `<`  `>`  `<=`  `>=`           |
| Logical    | `&&`  `\|\|`  `!` (prefix)                 |
| Bitwise    | `&`  `\|`  `^`  `<<`  `>>`                 |
| Prefix     | `!x` (logical-not), `-x` (negate)          |
| String     | `+` (concat); int/float/bool coerce when concatenated |

String comparison is lexicographic (`"apple" < "banana"`).

---

## Control flow

```rust
if condition {
    ...
} else {
    ...
}

while condition {
    ...
}
```

Parentheses around conditions are optional. `while` has a
built-in 1,000,000-iteration runaway guard so an infinite
loop terminates with a clean error rather than hanging the
process.

---

## Match expressions

`match` picks the first arm whose pattern matches the scrutinee.

```rust
match n {
    0 => "zero",
    x => "non-zero: " + x,
}
```

### Arm guards

An optional `if <bool-expr>` after the pattern gates the arm
on a runtime condition:

```rust
match n {
    x if x < 0 => "negative",
    0          => "zero",
    x if x > 100 => "big",
    _          => "small-positive",
}
```

A guarded arm does not count toward exhaustiveness — a match
with only guarded arms still needs an unguarded catch-all.

### Or-patterns

Alternatives can share an arm — `<p1> | <p2> | ...`:

```rust
match d {
    0 | 6             => "weekend",
    1 | 2 | 3 | 4 | 5 => "weekday",
    _                 => "invalid",
}
```

### `default` keyword

`default` is a reserved alias for `_`:

```rust
match n {
    0       => "zero",
    default => "other",   // same as `_ => "other"`
}
```

### Bind patterns (`name @ inner`)

A `name @ inner` pattern simultaneously binds the matched value to
`name` and tests it against `inner`. This is useful when the arm body
needs both the whole value and the confirmation that it matched:

```rust
match n {
    val @ 1..=10 => println("in range: " + val),
    _            => println("out of range"),
}
```

The `inner` pattern can be a literal, a range, a wildcard, or any
other valid sub-pattern.

---

## Type aliases

`type <Name> = <Target>;` at top level declares a structural
alias:

```rust
type Meters = int;
fn step(Meters m) -> Meters { return m + 1; }
```

Aliases are structural, not nominal — `Meters` unifies with
`int` at every use site. For a nominal type that doesn't flow
into `int` parameters, wrap in a one-field struct instead.

---

## Structs

```rust
struct Point {
    int x,
    int y,
}
```

### Struct literals

```rust
let p = new Point { x: 3, y: 4 };

// Shorthand: when the value expression is just the field name,
// the `: name` is optional.
let p = new Point { x, y };

// Mix explicit and shorthand in any order:
let q = new Point { x, y: y + 1 };
```

### Destructuring

`let <StructName> { ... } = expr;` pulls fields into locals:

```rust
let Point { x, y } = p;          // bind both fields

let Point { x: a, y: b } = p;    // rename fields

struct Foo { int a, int b, int c, }
let f = new Foo { a: 1, b: 2, c: 3 };
let Foo { a, .. } = f;            // `..` ignores remaining fields
```

Without `..`, every declared field must appear in the pattern.

---

## Arrays

Dynamic arrays are constructed with bracket literals and indexed
with `arr[i]`. Indexing is bounds-checked — an out-of-range
access raises [E0009](errors/E0009).

```rust
let arr = [1, 2, 3];
let first = arr[0];    // → 1
arr[1] = 42;           // in-place update
println(arr[1]);       // → 42
```

### Fixed-size array type

Use `[T; N]` as a parameter or return type to declare a
fixed-length array of element type `T` and length `N`:

```rust
fn sum(int a, [int; 3] v) -> int {
    return v[0] + v[1] + v[2];
}
```

`[T; N]` is a type annotation only — the length is carried in
the type, not at runtime. The interpreter and VM accept both
dynamic and fixed-size arrays; the JIT uses this annotation for
layout decisions.

---

## Comments

```rust
// line comment

/* block comment, can
   span multiple lines */
```

---

## Built-in functions

| Name           | Signature              | Notes                              |
|----------------|------------------------|------------------------------------|
| `println(x)`   | any → void             | prints, trailing newline           |
| `print(x)`     | any → void             | no trailing newline; flushed       |
| `len(s)`       | string → int           | Unicode scalar count               |
| `abs(x)`       | number → number        | int or float                       |
| `min(a, b)`    | two numbers → number   | int↔float coercion                 |
| `max(a, b)`    | two numbers → number   | int↔float coercion                 |
| `sqrt(x)`      | number → float         |                                    |
| `pow(a, b)`    | two numbers → float    | `a^b`                              |
| `floor(x)`     | number → float         | toward -∞                          |
| `ceil(x)`      | number → float         | toward +∞                          |
| `to_float(x)`  | int → float            | Exact widening                     |
| `to_int(x)`    | float → int            | Truncate; errors on NaN / overflow |

---

## Diagnostics

All errors carry `<file>:<line>:<col>:` prefixes — editor-clickable
in any tool that recognizes the format (most do). Neither the
parser nor the lexer panic on any input — every error surfaces
as a recoverable diagnostic. A program that fails to parse or
evaluate exits non-zero so CI and shell pipelines can branch on
success.

For a full index of compiler error codes see the [Error Reference](errors/).

---

## Compiling and running

```bash
# Run the interpreter
rz resilient/examples/hello.rz

# With static type checking
rz --typecheck resilient/examples/hello.rz

# With the verification audit
rz --audit resilient/examples/hello.rz

# Bytecode VM
rz --vm resilient/examples/hello.rz

# Cranelift JIT (requires --features jit at build time)
rz --jit resilient/examples/hello.rz

# Interactive REPL
rz
```

---

## File extensions

Resilient source files use the `.rz` extension. The language is
unrelated to Rust — no `unsafe`, no lifetimes, no ownership/borrow
checker — and carries its own extension for clarity.
