# Resilient Language Syntax Guide

> **Tool authors, static analysis developers, and safety auditors**:
> this document is an informal, tutorial-oriented guide. For the
> formal specification — EBNF grammar, type inference rules,
> evaluation semantics, and the full runtime-error model — see
> [`docs/language-reference.md`](docs/language-reference.md). That
> reference is authoritative for questions about the grammar, the
> type system, and runtime behaviour; the present document
> complements it with prose and worked examples.

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

### Type aliases

`type <Name> = <Target>;` at top level declares an alias:

```rust
type Meters = int;
fn step(Meters m) -> Meters { return m + 1; }
```

Aliases are **structural, not nominal**. `Meters` unifies with
`int` at every use site — there is no distinction between
`int` and `Meters` once typechecking has expanded the alias. If
you want a fresh nominal type (`Meters` ≠ `int`), wrap the
target in a one-field struct instead (see RES-126):

```rust
struct Meters { int val, }
// `new Meters { val: 5 }` does NOT flow into an `int` parameter.
```

Within-file forward references work — `fn foo(Meters x) { ...}`
can precede `type Meters = int;` because the typechecker hoists
alias declarations in the same pass it uses for function
contracts. Cross-module forward references wait for the module
system to grow (today imports are textually spliced before the
typechecker runs, so aliases imported from another file are
already present).

Cycles (`type A = B; type B = A;`) surface as a clean
diagnostic: `type alias cycle: A -> B -> A`. No infinite loop,
no panic.

### Return types

A `-> TYPE` annotation is **optional** (RES-123). When omitted, the
return type is inferred from the body — identical to what you'd get
by writing it out explicitly:

```rust
// Both of these typecheck to `int`. The version without `-> int`
// is inferred; writing it out is still supported and still checked
// against the body.
fn square(int x) -> int { return x * x; }
fn square(int x)        { return x * x; }

// Body with no `return` statement infers `void`:
fn log_once(string msg) { println(msg); }
```

If you DO write the annotation and it disagrees with the body, the
typechecker rejects with a clean `return type mismatch — declared
<X>, body produces <Y>` diagnostic. **Parameter types stay
required** — inferring them from call-site usage is a worse DX
(errors fire at callers, not at the definition).

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

### Nesting (RES-140)

`live` blocks **compose**. When an inner block exhausts its own
retry budget, the failure escalates to the enclosing block as a
single recoverable error — the outer block counts that as one
failure and, if it still has retries left, re-runs its entire
body (which re-enters the inner block from scratch). Retry
counters at each level are independent; `live_retries()`
(RES-138) reads the innermost block's counter.

Be careful: retries at different levels multiply. Two nested
`live` blocks with the default `MAX_RETRIES=3` cap run the
inner body up to `3 * 3 = 9` times before the outer gives up.
Use `live backoff(...)` (RES-139) on at least one level when
the inner operation touches real hardware.

Worked example: inner always fails, outer swallows the
escalation until its own budget runs out.

```rust
fn main() {
    live {
        live {
            // Always fails.
            assert(false, "inner");
        }
    }
}
```

Final error:

```
Runtime error: Live block failed after 3 attempts (retry depth: 1):
    Live block failed after 3 attempts (retry depth: 2):
        ASSERTION ERROR: inner
```

Each `(retry depth: N)` note corresponds to one nesting level
— `depth: 1` is the outermost block, `depth: 2` is its child,
and so on. Inner invocations total `3 × 3 = 9`.

### Wall-clock timeout (RES-142)

A `live` block may cap its total retry time with a `within
<duration>` clause. Duration literals are `<integer><unit>`
where `unit ∈ {ns, us, ms, s}` — they exist only inside this
clause; they are not a general time library.

```rust
live within 10ms {
    let sample = poll_sensor();
    assert(is_fresh(sample), "stale");
}
```

Backoff sleeps count against the budget:

```rust
live backoff(base_ms=2, factor=2, max_ms=20) within 50ms {
    let r = flaky_io();
}
```

Either order is accepted by the parser — `backoff(...)` then
`within`, or `within` then `backoff(...)`. Neither clause may
appear twice.

When the budget is exceeded, the block escalates exactly like
exhaustion (RES-140) — counter bumps via
`live_total_exhaustions()` (RES-141) and the error's footer
note. Timeout uses a distinct prefix so diagnostics can tell
"hit retry cap" apart from "hit wall-clock":

```
Runtime error: Live block timed out after 1 attempt(s) (retry depth: 1):
    ASSERTION ERROR: forced
```

Note: the `no_std` embedded runtime shares RES-139's clock
placeholder — the wall-clock check is currently std-only;
embedded targets ignore the clause until a real monotonic
clock is wired in.

## Assertions

Assertions halt with a diagnostic. For comparison conditions both
operand values appear in the error:

```rust
assert(fuel >= 0, "Fuel must be non-negative");
// ASSERTION ERROR: Fuel must be non-negative
//   - condition -5 >= 0 was false
```

## Numeric coercion policy

**Resilient does not implicitly coerce between numeric types.**
Mixing `int` and `float` in arithmetic (`+ - * / %`), comparison
patterns, or any other operator is a **type error**. Users must
convert explicitly:

```rust
let a = 1 + 2.0;              // ERROR: Cannot apply '+' to int and float
let b = to_float(1) + 2.0;    // ok → float 3.0
let c = 1 + to_int(2.0);      // ok → int 3
```

Two builtins handle the bridge:

| Signature | Semantics |
|---|---|
| `to_float(int) -> float` | widen with exact representation (for `abs(x) < 2^53`) |
| `to_int(float) -> int` | truncate toward zero; `NaN` / `±∞` / out-of-range are **runtime errors** (not silent saturation) |

Rationale: a safety-critical language should surface numeric-
domain changes at the source rather than paper over them. The
RES-130 change is a one-time break for pre-1.0 code that relied
on silent coercion; the errors explicitly point users at the
`to_float` / `to_int` hint.

## Structs

```rust
struct Point {
    int x,
    int y,
}

fn main(int _d) {
    let x = 3;
    let y = 4;

    // Explicit form — field name followed by colon and value.
    let a = new Point { x: x, y: y };

    // Shorthand (RES-154): when the value expression is simply the
    // field's name, drop the `:<name>`. Equivalent to the explicit
    // form above.
    let b = new Point { x, y };

    // Shorthand and explicit can mix in the same literal, in any
    // order:
    let c = new Point { x, y: y + 1 };
}
```

The shorthand is pure parser sugar — the AST reconstructs the
`field -> Identifier(name)` pair before typechecking — so an
unbound name produces the same `Identifier not found` diagnostic as
any other use.

### Destructuring let (RES-155)

`let <StructName> { ... } = expr;` pulls fields into local
bindings in a single statement:

```rust
let p = new Point { x: 3, y: 4 };

// Full destructure — shorthand form mirrors the struct-literal
// shorthand on the construction side.
let Point { x, y } = p;

// Renaming — `field: local_name` binds the field to a
// differently-named local.
let Point { x: a, y: b } = p;

// `..` rest pattern ignores the remaining fields; without it,
// every declared field must appear in the pattern or the
// typechecker errors listing what's missing.
struct Foo { int a, int b, int c, }
let f = new Foo { a: 1, b: 2, c: 3 };
let Foo { a, .. } = f;           // ok — ignores b, c
// let Foo { a } = f;            // type error: missing field(s) b, c
```

Only one layer deep — nested struct patterns are a follow-up. The
pattern struct name must match the value's struct name at runtime.

## Data Types

- `int`: 64-bit signed integer. Accepts decimal (`42`), hex (`0xFF`),
  and binary (`0b1010`) literals. Underscore separators allowed:
  `0xDEAD_BEEF`.
- `float`: 64-bit floating point
- `string`: UTF-8 text; `len(s)` returns scalar count
- `bytes`: raw byte sequence, `b"\x00\x01abc"` literal (RES-152)
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

## Match expressions

`match` picks the first arm whose pattern matches the scrutinee.
Patterns can be literals (int / float / string / bool), `_`
(wildcard), or an identifier that binds the scrutinee.

```rust
match n {
    0 => "zero",
    x => "non-zero: " + x,
}
```

### Arm guards (RES-159)

An optional `if <bool-expr>` between the pattern and `=>` gates
the arm on a runtime condition. The guard is evaluated in the
pattern's binding scope, so it can reference pattern bindings:

```rust
match n {
    x if x < 0 => "negative",
    0          => "zero",
    x if x > 100 => "big",
    _          => "small-positive",
}
```

A guarded arm **does not count** toward exhaustiveness — the
typechecker treats it as "might not fire", so a match with only
guarded arms still needs an unguarded catch-all (`_` / bare
identifier) or full literal coverage of a finite type like
`bool`. Non-boolean guard expressions are a typecheck error.

Guards can call impure functions or inspect mutable state, but
this is strongly discouraged: the verifier (G9) will refuse to
reason about them.

### Or-patterns (RES-160)

Alternatives can share an arm — `<p1> | <p2> | ...`. First match
wins, and exhaustiveness unions the covered space across
branches:

```rust
match d {
    0 | 6         => "weekend",
    1 | 2 | 3 | 4 | 5 => "weekday",
    _             => "invalid",
}

match b {
    true | false => "any bool",   // exhaustive, no `_` needed
}
```

Every branch of an or-pattern must bind the same names.
`x | 0 => ...` is rejected at typecheck with "or-pattern
branches bind different names" — the body would otherwise
reference a binding whose presence depends on which branch
fired. This mirrors Rust's rule.

### `default` keyword (RES-163)

`default` is a reserved alias for `_` at the top of a match
arm — pure readability sugar; both forms produce identical
AST and runtime behaviour:

```rust
match n {
    0 => "zero",
    1 => "one",
    default => "other",   // same as `_ => "other"`
}
```

Because `default` is reserved, it cannot appear as an
identifier: `let default = 3;` is a parse error
(`Expected identifier after 'let', found \`default\``).
No other `_` synonyms (`otherwise`, `else`, ...) are planned
— one alias is plenty.

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
| `split(s, sep)` | (string, string) → array of string | empty `sep` splits into Unicode scalars |
| `trim(s)` | string → string | strips leading/trailing ASCII whitespace |
| `starts_with(s, prefix)` | (string, string) → bool | empty prefix always matches |
| `ends_with(s, suffix)` | (string, string) → bool | empty suffix always matches |
| `repeat(s, n)` | (string, int) → string | `n >= 0`; negative `n` is a hard error |

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
