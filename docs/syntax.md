---
title: Syntax Reference
nav_order: 4
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

## Assertions

Assertions halt with a diagnostic. For comparison conditions
both operand values appear in the error message:

```rust
assert(fuel >= 0, "Fuel must be non-negative");
// ASSERTION ERROR: Fuel must be non-negative
//   - condition -5 >= 0 was false
```

## Data types

| Type     | Notes                                                              |
|----------|--------------------------------------------------------------------|
| `int`    | 64-bit signed. Decimal (`42`), hex (`0xFF`), binary (`0b1010`). Underscore separators allowed: `0xDEAD_BEEF`. |
| `float`  | 64-bit IEEE-754                                                    |
| `string` | UTF-8 text; `len(s)` returns scalar count                          |
| `bool`   | `true` / `false`                                                   |

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

## Comments

```rust
// line comment

/* block comment, can
   span multiple lines */
```

## Built-in functions

| Name         | Signature              | Notes                         |
|--------------|------------------------|-------------------------------|
| `println(x)` | any → void             | prints, trailing newline      |
| `print(x)`   | any → void             | no trailing newline; flushed  |
| `len(s)`     | string → int           | Unicode scalar count          |
| `abs(x)`     | number → number        | int or float                  |
| `min(a, b)`  | two numbers → number   | int↔float coercion            |
| `max(a, b)`  | two numbers → number   | int↔float coercion            |
| `sqrt(x)`    | number → float         |                               |
| `pow(a, b)`  | two numbers → float    | `a^b`                         |
| `floor(x)`   | number → float         | toward -∞                     |
| `ceil(x)`    | number → float         | toward +∞                     |

## Diagnostics

All errors carry `<file>:<line>:<col>:` prefixes — editor-clickable
in any tool that recognizes the format (most do). Neither the
parser nor the lexer panic on any input — every error surfaces
as a recoverable diagnostic. A program that fails to parse or
evaluate exits non-zero so CI and shell pipelines can branch on
success.

## Compiling and running

```bash
# Run the interpreter
resilient examples/hello.rs

# With static type checking
resilient --typecheck examples/hello.rs

# With the verification audit
resilient --audit examples/hello.rs

# Bytecode VM
resilient --vm examples/hello.rs

# Cranelift JIT (requires --features jit at build time)
resilient --jit examples/hello.rs

# Interactive REPL
resilient
```

## File extensions

The repo uses `.rs` for source files so editors with Rust
syntax highlighting give you free coloring. The language is
otherwise unrelated to Rust — no `unsafe`, no lifetimes, no
ownership/borrow checker. A future ticket may switch to a
distinct extension (`.res`?) for clarity.
