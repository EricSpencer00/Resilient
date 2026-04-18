# Resilient Language Syntax Guide

This document describes the syntax of the Resilient language as of
the current ticket set. Language features are added per-ticket —
see `.board/tickets/DONE/` for the full ledger.

## Function Declarations

Functions declare their parameters with types. Zero-parameter
functions use empty parentheses:

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

## Lexical: identifiers

Identifiers match `[A-Za-z_][A-Za-z0-9_]*` — **ASCII only**. Non-
ASCII letters (Cyrillic, Greek, accented Latin, CJK, etc.) are
*rejected* in identifier position with a dedicated diagnostic:

```
1:1: identifier contains non-ASCII character 'ф' — Resilient
identifiers are ASCII-only (see SYNTAX.md)
```

Rationale: this is a safety-critical language. Homoglyph attacks
— two identifiers that render identically in most fonts but have
different code points (Cyrillic `кафа` vs Latin `kafa`, Greek
`Α` vs Latin `A`) — make code review unreliable. Forbidding
non-ASCII in identifiers eliminates the class outright.

String literals, comments, and file contents generally retain
full UTF-8 — the policy tightens *only* identifier scanning. If
a real user asks for a non-ASCII opt-in, we'll revisit under a
new ticket with an explicit flag; we don't build the escape
hatch speculatively (RES-114).

## Variable Declarations

```rust
let x = 42;
let name = "Resilient";
x = x + 1;        // reassignment requires the name to be declared
```

## Static Variables

`static let` bindings persist across function calls. They're the
MVP stand-in for global state:

```rust
fn tick() {
    static let n = 0;
    n = n + 1;
    return n;
}
// tick() → 1, then 2, then 3
```

## Live Blocks

Live blocks re-execute on recoverable error, restoring state from the
last known-good snapshot:

```rust
live {
    let sensor_value = read_sensor();
    assert(is_valid_reading(sensor_value), "Invalid reading");
    process_data(sensor_value, threshold);
}
```

## Assertions

Assertions halt with a diagnostic. For comparison conditions both
operand values appear in the error:

```rust
assert(fuel >= 0, "Fuel must be non-negative");
// ASSERTION ERROR: Fuel must be non-negative
//   - condition -5 >= 0 was false
```

## Data Types

- `int`: 64-bit signed integer. Accepts decimal (`42`), hex (`0xFF`),
  and binary (`0b1010`) literals. Underscore separators allowed:
  `0xDEAD_BEEF`.
- `float`: 64-bit floating point
- `string`: UTF-8 text; `len(s)` returns scalar count
- `bool`: `true` / `false`

## Operators

| Category | Operators |
|---|---|
| Arithmetic | `+` `-` `*` `/` `%` |
| Comparison | `==` `!=` `<` `>` `<=` `>=` |
| Logical | `&&` `\|\|` `!` (prefix) |
| Bitwise | `&` `\|` `^` `<<` `>>` |
| Prefix | `!x` (logical-not), `-x` (negate) |
| String | `+` (concat); int/float/bool coerce to string when concatenated |

String comparison is lexicographic (`"apple" < "banana"`).

## Control Flow

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

Parentheses around conditions are optional. `while` has a built-in
1,000,000-iteration runaway guard.

## Comments

```rust
// line comment
/* block comment, can
   span multiple lines */
```

## Built-in Functions

| Name | Signature | Notes |
|---|---|---|
| `println(x)` | any → void | prints, trailing newline |
| `print(x)` | any → void | no trailing newline; stdout flushed |
| `len(s)` | string → int | Unicode scalar count |
| `abs(x)` | number → number | int or float |
| `min(a, b)` | two numbers → number | int↔float coercion |
| `max(a, b)` | two numbers → number | int↔float coercion |
| `sqrt(x)` | number → float | |
| `pow(a, b)` | two numbers → float | `a^b` |
| `floor(x)` | number → float | toward -∞ |
| `ceil(x)` | number → float | toward +∞ |

## Diagnostics

Parser errors carry `line:col:` prefixes. Neither the parser nor the
lexer panic on any input — everything surfaces as a recoverable
error. A program that fails to parse or evaluate exits non-zero, so
CI and shell pipelines can branch on success.

## Compiling and Running

```bash
cargo run -- examples/hello.rs         # run a program
cargo run -- --typecheck foo.rs        # with type checking
cargo run                              # interactive REPL
cargo test                             # run the test suite
cargo test -- --ignored                # see which examples still
                                       # lack .expected.txt sidecars
```
