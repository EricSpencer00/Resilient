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
see [closed GitHub Issues](https://github.com/EricSpencer00/Resilient/issues?q=is%3Aissue+is%3Aclosed) for the full ledger.

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

### Function Contracts

Functions can declare contracts using `requires`, `ensures`, and `recovers_to`:

```rust
fn safe_divide(int a, int b)
    requires b != 0
    ensures  result >= 0
    recovers_to: result > 0
{ return a / b; }
```

- `requires` — pre-condition checked before the function executes
- `ensures` — post-condition checked after successful return
- `recovers_to` — recovery postcondition (V1: single-step property; see RES-396 for V2's multi-step `<>` operator)

All three clauses are checked at runtime and discharged by the Z3 verifier when `--features z3` is enabled. `recovers_to` is a **single-transition** postcondition — it asserts a property of the final state after recovery, not a guarantee of eventual reachability. Multi-step recovery operators are a V2 capability tracked under [RES-396](https://github.com/EricSpencer00/Resilient/issues/270); do not read `recovers_to` as providing temporal guarantees.

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

> **Formal spec:** the normative rules for retry counts, invariant
> ordering, state roll-back, nesting, timeouts, and the
> `live_retries()` builtin live in
> [`docs/live-block-semantics.md`](docs/live-block-semantics.md).
> This section is the friendly tour; that page is the contract.

Live blocks re-execute on recoverable error, restoring state from the
last known-good snapshot:

```rust
live {
    let sensor_value = read_sensor();
    assert(is_valid_reading(sensor_value), "Invalid reading");
    process_data(sensor_value, threshold);
}
```

During development, pass `--panic-on-fault` to `rz` to
disable the retry loop and surface the first fault immediately
(exit code 1); use `--no-panic-on-fault` to restore the default
self-healing behaviour (RES-211).

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

## Traits

Traits (RES-290) define interfaces — sets of methods that a type must implement.
The trait system enables polymorphism without virtual tables; dispatch is
monomorphic and type-specialized at compile time.

### Trait Declaration

Declare a trait with method signatures (no bodies):

```rust
trait Printable {
    fn to_string(self) -> string;
}

trait Iterator {
    fn next(self) -> int;
    fn has_next(self) -> bool;
}
```

Method signatures must include `self` (method receiver) as the first parameter.
Method names must be unique within a trait. RES-779 (future) will add associated
types.

### Trait Implementation

Implement a trait for a type using `impl Trait for Type { ... }`:

```rust
struct Point {
    int x,
    int y,
}

impl Printable for Point {
    fn to_string(self) -> string {
        return "(" + self.x + ", " + self.y + ")";
    }
}
```

The impl block must provide all methods declared in the trait, with matching
names and arities. Method implementations use the same `fn` syntax as top-level
functions, and can access struct fields via `self`.

### Generic Bounds with Traits

Use trait bounds to constrain generic type parameters:

```rust
fn announce<T: Printable>(item: T) {
    println(item.to_string());
}

struct Container { }

impl Printable for Container {
    fn to_string(self) -> string { return "Container"; }
}

fn main() {
    let c = new Container { };
    announce(c);  // OK: Container implements Printable
}
```

When a generic function is called with a type argument, the typechecker
verifies that the type implements all required traits. If the type has an
explicit `impl Trait for Type` block, that satisfies the bound. Otherwise,
the type's plain `impl Type { ... }` methods must match the trait's
signatures.

### Structural vs. Nominal Typing

Trait checking is **structural** — the typechecker examines method names
and arities, not explicit `impl` declarations. If a type has all methods
the trait requires (matching by name and parameter count), it satisfies the
bound, even without an explicit `impl Trait for Type` block:

```rust
struct Pair { int a, int b, }

impl Pair {
    fn to_string(self) -> string {
        return "Pair(" + self.a + ", " + self.b + ")";
    }
}

fn main() {
    let p = new Pair { a: 1, b: 2 };
    announce(p);  // OK: Pair has to_string(self) -> string
}
```

In this example, `Pair` is not explicitly declared to implement `Printable`,
but the call succeeds because the method exists. An explicit `impl
Printable for Pair` block is not required — it is useful mainly for
documentation and to catch mistakes early if a required method is missing.

### Associated Types (RES-783)

Traits can declare associated types — type members that each implementation must define:

```rust
trait Transport {
    type Message;      // Each impl defines what Message means
    type Error;        // Each impl defines its error type
    
    fn send(self, message: Self::Message) -> Result<int, Self::Error>;
}

struct UART { }

impl Transport for UART {
    type Message = [int];  // UART sends arrays of ints
    type Error = string;   // UART errors are strings
    
    fn send(self, message: [int]) -> Result<int, string> {
        return Ok(0);
    }
}

struct TCP { }

impl Transport for TCP {
    type Message = string; // TCP sends strings
    type Error = int;      // TCP errors are ints
    
    fn send(self, message: string) -> Result<int, int> {
        return Ok(0);
    }
}
```

Associated types enable polymorphic interfaces where each implementation can specify its own concrete types for `Message` and `Error`, making embedded driver abstractions type-safe without explicit generics at every call site.

### Limitations

The current trait system does not support:
- Projection syntax (`T::AssocType`) in generic bounds (RES-779 follow-up)
- `dyn Trait` / virtual tables (RES-293)
- Generic associated types (future)
- Default method bodies (future)
- Blanket impls or specialization (future)

## Data Types

- `int`: 64-bit signed integer. Accepts decimal (`42`), hex (`0xFF`),
  and binary (`0b1010`) literals. Underscore separators allowed in all
  three forms: `0xDEAD_BEEF`, `0b1010_0001`, `1_000_000` (RES-909).
- `float`: 64-bit floating point. Accepts decimal-point literals
  (`1.5`, `42.`), RES-906 scientific notation (`1e9`, `2E10`,
  `1.5e-3`, `1.e3`), and RES-909 underscore separators in mantissa
  and exponent body (`1_000.5e1_0`). Exponent body (`+`/`-` optional,
  then ≥1 digit) must follow `e`/`E` — bare `1e` lexes as
  `Int(1) Ident("e")`. An underscore must sit between two digits;
  `1_a` lexes as `Int(1) Ident("_a")`, never silently swallowing the
  separator.
- `string`: UTF-8 text; `len(s)` returns scalar count
- `bytes`: raw byte sequence, `b"\x00\x01abc"` literal (RES-152)
- `bool`: `true` / `false`

### Strings

String literals use double quotes with UTF-8 support:

```rust
let greeting = "Hello, World!";
let multiword = "hello" + " " + "world";
```

**String Concatenation** — The `+` operator concatenates strings and
coerces other types to strings:

```rust
let message = "Count: " + 42;           // "Count: 42"
let status = "Ready: " + true;          // "Ready: true"
let value = "Pi: " + 3.14;              // "Pi: 3.14"
```

**Common String Operations** — The following builtins work on strings:

- `len(s)` → int: UTF-8 scalar count (not byte length)
- `split(s, sep)` → [string]: split by separator (empty sep splits into scalars)
- `trim(s)` → string: strip leading/trailing whitespace
- `starts_with(s, prefix)` → bool: check if string starts with prefix
- `ends_with(s, suffix)` → bool: check if string ends with suffix
- `repeat(s, n)` → string: repeat string n times
- `parse_int(s)` → Result<int, string>: convert to integer (base 10)
- `parse_float(s)` → Result<float, string>: convert to float
- `char_at(s, i)` → Result<string, string>: single character at index i
- `pad_left(s, n, c)` → string: left-pad to n characters using char c
- `pad_right(s, n, c)` → string: right-pad to n characters using char c

**String Comparison** — Lexicographic ordering:

```rust
"apple" < "banana"   // true
"zebra" > "aardvark" // true
"cat" == "cat"       // true
```

## Operators

| Category | Operators |
|---|---|
| Arithmetic | `+` `-` `*` `/` `%` |
| Comparison | `==` `!=` `<` `>` `<=` `>=` |
| Logical | `&&` `\|\|` `!` (prefix) |
| Bitwise | `&` `\|` `^` `<<` `>>` |
| Prefix | `!x` (logical-not), `-x` (negate) |
| String | `+` (concat); int/float/bool coerce to string when concatenated |
| String × Int | `*` (RES-924) — `"-" * 10` repeats the string; both operand orders work |
| Pipe | `\|>` (RES-926) — `x \|> f` desugars to `f(x)`; `x \|> f(args)` to `f(x, args)`. Left-associative, lowest infix precedence |
| Array index | `arr[i]` — single element |
| Array slice | `arr[lo..hi]`, `arr[lo..=hi]`, `arr[lo..]`, `arr[..hi]`, `arr[..]` (RES-911) — returns a fresh array |
| Compound assign | `+= -= *= /= %= &= \|= ^= <<= >>=` (RES-912) — `x OP= rhs` desugars to `x = x OP rhs` for plain identifiers |

## Array slicing (RES-911)

```rust
let xs = [10, 20, 30, 40, 50];
xs[1..3];     // [20, 30]            — half-open
xs[1..=3];    // [20, 30, 40]        — inclusive
xs[2..];      // [30, 40, 50]        — open upper
xs[..3];      // [10, 20, 30]        — open lower
xs[..];       // [10, 20, 30, 40, 50] — full copy
```

Bounds clamp against `len(xs)` so `xs[..99]` returns the whole
array, **not** an error. RES-921: negative endpoints wrap from the
end (`-1` is the last element):

```rust
xs[-1];        // last element
xs[-2..];      // last two
xs[..-1];      // everything except the last
xs[-3..-1];    // inner two from the end
```

`lo > hi` after wrapping is still a runtime error.

## String slicing (RES-916)

Same five forms as array slicing, but indices are **Unicode-scalar
indices** (matching `len(s)` semantics) so multi-byte UTF-8 strings
slice on codepoint boundaries:

```rust
let s = "héllo";
s[0..3];     // "hél"     — 3 scalars, 4 bytes
s[2..=4];   // "llo"
s[..]; s[..3]; s[3..];
```

Out-of-range upper bound clamps to scalar length. `lo > hi` and
negative endpoints are runtime errors.

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

### `if` as expression (RES-925)

`if` works in expression position. Each branch is a block; the
block's value is its trailing bare expression (no semicolon).
`else if` chains parse as a recursive IfStatement on the
alternative branch.

```rust
let label = if n > 0 {
    "positive"
} else if n < 0 {
    "negative"
} else {
    "zero"
};
```

The statement form (`if cond { stmts; }` with no value, no `else`)
continues to work alongside.

### `loop { ... }` (RES-913)

Unconditional infinite loop — exit only via `break` (or `return`):

```rust
let i = 0;
loop {
    if i >= 10 { break; }
    i += 1;
}
```

Equivalent to `while true { ... }` and shares the same 1M-iteration
runaway guard. `loop` is a statement, not an expression — `let x =
loop { break v; }` (loop-with-value) is a future enhancement.

### `while let` (RES-914)

Iterate while a pattern matches the (re-evaluated each iteration)
scrutinee. Pure parser-level sugar over `loop { match ... }`:

```rust
while let Some(item) = pop_next() {
    use_item(item);
}
```

desugars to

```rust
loop {
    match pop_next() {
        Some(item) => { use_item(item); }
        _          => { break; }
    }
}
```

All `match` patterns work (literal, identifier-binding, struct,
tuple, variant, wildcard). Identifier patterns always match — so
`while let x = ...` is unconditional unless the body breaks.

### `break` and `continue` (RES-910)

Both statements affect the **innermost** enclosing `while` or
`for-in` loop:

```rust
let i = 0;
while i < 100 {
    if i == 5 {
        break;       // exit the loop now; post-loop block runs
    }
    if i < 0 {
        continue;    // skip to the next iteration
    }
    i = i + 1;
}
```

Both are typechecker-rejected outside any loop:

    'break' outside of a loop — `break` is only valid inside
    a `while` or `for-in` body

Labelled `break` (`break 'outer;`) and `break <value>;` are not
yet supported. Inside nested loops, `break` only exits one level.

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

### Result patterns (RES-923)

Match arms can destructure a `Result<T, E>` directly:

```rust
match parse_int(input) {
    Ok(n)  => use_value(n),
    Err(e) => report(e),
}
```

`Ok(<inner>)` and `Err(<inner>)` mirror `Some(<inner>)` /
`None` for `Option`. Inner pattern can be a wildcard,
identifier-binding, literal, range, or another nested pattern.

### Tuple-struct patterns (RES-931)

A tuple-struct value (declared with `struct Pair(int, string);`)
destructures positionally — the pattern's sub-patterns line up with
the struct's positional fields in declaration order:

```rust
struct Pair(int, string);

let p = new Pair(42, "hi");
match p {
    Pair(0, _) => "zero",
    Pair(n, s) => s + ":" + n,
}
```

Sub-patterns can be any other pattern atom — wildcard, identifier,
literal, range, nested `Some(_)` / `Ok(_)` / `Err(_)`, or another
tuple-struct pattern. The arity must match the struct's declared
field count; a mismatched count is a typecheck error
(`tuple-struct pattern \`Pair\` expects 2 field(s), got 1`).

`Pair(_, _)` is a catch-all for `Pair`-typed scrutinees and counts
toward exhaustiveness; `Pair(0, _)` does not (it misses every
non-zero `Pair`).

### Range patterns (RES-915)

Match arms accept integer range patterns:

```rust
match n {
    1..=5  => "small",        // inclusive: 1, 2, 3, 4, 5
    6..10  => "medium",       // half-open: 6, 7, 8, 9
    _      => "other",
}
```

Both endpoints must be integer literals (negative `hi` allowed:
`1..=-3` is degenerate and matches nothing). Range patterns only
fire on `Int` scrutinees; non-Int values fall through to the
wildcard. Range patterns bind no names today (`1..=5 @ x` binding
is a future enhancement).

### `if let` (RES-908)

`if let` is parser-level sugar over `match`. It is the idiomatic way
to bind a pattern conditionally without spelling out the full match:

```rust
if let Some(x) = optional {
    use_value(x);
} else {
    fallback();
}
```

desugars to

```rust
match optional {
    Some(x) => { use_value(x); }
    _       => { fallback(); }
}
```

All `match` patterns are supported (literal, identifier, struct,
tuple, variant, wildcard). Without an `else` branch, the no-match
arm is an empty block. `else if let` chaining is **not** sugar in
this PR — write it as nested `if let`s or fall back to a full
`match`.

```rust
// line comment
/* block comment, can
   span multiple lines */
```

## Built-in Functions

For Strings and Arrays, the most common builtins also accept
**method-call sugar** (RES-920). `target.method(args)` dispatches
to `method(target, args...)` — same semantics, less typing:

```rust
"  hi  ".trim().to_upper()        // "HI"
[1, 2, 3].push(4).len()           // 4
words.split(",").len()
```

Methods available today:
- String: `len`, `trim`, `to_upper`, `to_lower`, `contains(sub)`,
  `starts_with(p)`, `ends_with(p)`, `split(sep)`, `repeat(n)`.
- Array: `len`, `push(x)`, `pop`, plus the RES-927 functional trio
  `map(fn)`, `filter(pred)`, `reduce(init, fn)`.

The prefix forms (`len(s)` etc.) continue to work; this is additive
sugar, not a rename.

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
| `parse_int(s)` | string → Result<Int, String> | base 10; whitespace stripped; `Err` on invalid input — never panics |
| `parse_float(s)` | string → Result<Float, String> | whitespace stripped; `Err` on invalid input — never panics |
| `char_at(s, i)` | (string, int) → Result<String, String> | single-char string at Unicode-scalar index `i`; `Err` on out-of-range or negative |
| `pad_left(s, n, c)` | (string, int, string) → string | left-pad with single char `c` until char-length ≥ `n`; multi-char or empty `c` is a hard error |
| `pad_right(s, n, c)` | (string, int, string) → string | right-pad; same validation as `pad_left` |

## Foreign Function Interface

Resilient programs call into C libraries through `extern` blocks. Only
primitive types are supported in v1: `Int`, `Float`, `Bool`, `String`, and
`Void`.

```
extern "libm.so.6" {
    fn sqrt(x: Float) -> Float requires _0 >= 0.0 ensures result >= 0.0;
}
```

### Block form

```
extern "LIBRARY_PATH" {
    fn NAME(PARAM: TYPE, ...) -> RETURN_TYPE [contracts];
    fn NAME(PARAM: TYPE, ...) -> RETURN_TYPE = "C_SYMBOL_NAME" [contracts];
    ...
}
```

- `LIBRARY_PATH` — passed verbatim to the OS dynamic linker (`dlopen`).
  On Linux use `libm.so.6`; on macOS use `libm.dylib`.
- `= "C_SYMBOL_NAME"` — optional alias when the Resilient name differs
  from the C symbol (e.g. `fn c_abs(x: Int) -> Int = "abs";`).

### Type map

| Resilient | C ABI |
|-----------|-------|
| `Int`     | `int64_t` |
| `Float`   | `double` |
| `Bool`    | `bool` (i8) |
| `String`  | not yet supported |
| `Void`    | `void` / no return |

At most 8 parameters per extern function (v1 limit).

### Contracts on extern fns

```
fn sqrt(x: Float) -> Float
    requires _0 >= 0.0
    ensures  result >= 0.0;
```

- `requires` — pre-condition checked **before** the C call.
  Arguments are bound positionally as `_0`, `_1`, … (not by parameter name).
- `ensures` — post-condition checked **after** the C call returns.
  The return value is bound as `result`.
- Both clauses are evaluated by the tree-walker interpreter; violations
  produce a runtime error.

### `@trusted` extern fns

```
@trusted
fn fast_log(x: Float) -> Float requires _0 > 0.0 ensures result >= 0.0;
```

Mark a function `@trusted` to treat its `ensures` clauses as SMT axioms
(fed to Z3 at verification sites). The `ensures` clause is still evaluated
at runtime; a failure does not abort the program. Instead, the clause is
propagated as an SMT axiom for the Z3 verifier, which can reason about
foreign postconditions without you needing to prove them inline.

### Feature flags

| Feature | Effect |
|---------|--------|
| `ffi` *(opt-in)* | Enables the `extern` block, dynamic linker, trampolines |
| `ffi-static` | `resilient-runtime` static registry for `no_std` hosts |

Compile without the `ffi` feature to get a compile-time error on any program that uses `extern` blocks (`FfiError::FfiDisabled`).

## Diagnostics

Parser errors carry `line:col:` prefixes. Neither the parser nor the
lexer panic on any input — everything surfaces as a recoverable
error. A program that fails to parse or evaluate exits non-zero, so
CI and shell pipelines can branch on success.

### Partial proofs (Z3 `Unknown`) — RES-217

When a `requires` or `ensures` clause falls to the Z3 backend and the
solver answers `Unknown` (either hitting the per-query timeout set by
`--verifier-timeout-ms` or legitimately failing to decide the
obligation — typical for nonlinear integer arithmetic), the
typechecker emits a structured warning and keeps the runtime check:

```
warning[partial-proof]: Z3 returned Unknown for assertion at foo.rz:12:18 — proof is incomplete
```

Compilation still succeeds — the obligation downgrades silently to a
runtime assertion — but the warning gives CI and review tooling a
stable `[partial-proof]` tag to grep for, plus a precise
`<file>:<line>:<col>` pointer to the offending clause.

The warning is **on by default**. Pass `--no-warn-unverified` to
suppress it (useful for CI pipelines that already gate on separate
verification signals); pass `--warn-unverified` to opt in explicitly
even though that matches the default. The pre-existing per-function
`hint: proof timed out after <N>ms …` line is independent and
unaffected by this flag.

## Compiling and Running

After installing `rz` (see [README's Getting Started](README.md#getting-started)):

```bash
rz examples/hello.rz                   # run a program
rz --typecheck foo.rz                  # with type checking
rz                                     # interactive REPL
```

Contributor workflow (running tests, hacking on the compiler itself):

```bash
cargo test --manifest-path resilient/Cargo.toml             # run the test suite
cargo test --manifest-path resilient/Cargo.toml -- --ignored  # see which examples still
                                                              # lack .expected.txt sidecars
```

## Unsafe Blocks and Embedded I/O

Resilient provides `unsafe { ... }` blocks as the required wrapper for volatile MMIO access on bare-metal microcontrollers. Inside an `unsafe` block, you call fixed-width volatile intrinsics to read from and write to hardware registers; outside, calling these intrinsics is a compile-time error. This design forces programmers to acknowledge hardware access explicitly while preserving safety boundaries elsewhere in the program.

The eight volatile intrinsics — `volatile_read_u8`, `volatile_read_u16`, `volatile_read_u32`, `volatile_read_u64`, `volatile_write_u8`, `volatile_write_u16`, `volatile_write_u32`, `volatile_write_u64` — accept an address (as `int`, i.e., i64) and a value (also `int`). The runtime checks that the address fits in `usize` before the access. Inside `unsafe`, formal contracts (`@requires` / `@ensures` annotations) are inert — the compiler ignores them, treating the code as trusted by virtue of being explicitly marked `unsafe`. The programmer asserts correctness by writing `unsafe`; Z3 does not reason over unsafe blocks.

The `#[interrupt(name = "STRING")]` attribute registers a zero-parameter, unit-return function as an interrupt service routine (ISR) for a named interrupt vector. The compiler lowers this to an external symbol named `__resilient_isr_<NAME>` marked `extern "C"` and `no_mangle`, which the target's runtime crate (e.g., `resilient-runtime-cortex-m-demo`) links to a vector table entry via weak alias. ISR functions carry an implicit `isr` effect — calling them from non-ISR context is a compile-time error. Only the `name` attribute key is supported in V1; other keys are a compile-time error.

```resilient
unsafe fn write_led_on() {
    volatile_write_u32(0x4001_0C14, 1);
}

#[interrupt(name = "SysTick")]
fn tick_handler() {
    unsafe { volatile_write_u32(0x4001_0C14, 0); }
}
```

Use `unsafe` and volatile intrinsics when you need to directly manipulate hardware registers (GPIO, timers, peripherals) on an embedded target. The `#[interrupt]` attribute is the entry point for ISR handlers that respond to hardware events; it ensures the handler is discoverable by the runtime's vector table without requiring manual symbol registration.

## Region Annotations and the Borrow Checker

Resilient's region system prevents aliased mutable borrows at compile time. A *region* is a named memory area that tracks ownership. Declare regions at module scope with `region NAME;`, then annotate reference parameters with `&[NAME]` (shared) or `&mut[NAME]` (exclusive mutable).

The borrow checker rejects any function that declares two `&mut[A]` parameters with the **same** region label, because the caller could pass the same variable for both, creating aliased mutation:

```resilient
region A;

// error: `same_region_bad` has two &mut[A] params — aliasing is possible
fn same_region_bad(&mut[A] int x, &mut[A] int y) { ... }

// ok: A ≠ B, so x and y cannot alias
region B;
fn disjoint_ok(&mut[A] int x, &mut[B] int y) { ... }
```

### Region-polymorphic functions

When a function should work with *any* region but still enforce disjointness between its parameters, declare it with **region type parameters** using angle brackets after the function name:

```resilient
region A;

fn update<R, S>(&mut[R] int a, &mut[S] int b) {
    // R and S are distinct by declaration; the call site enforces it
}

fn caller(&mut[A] int a, &mut[A] int b) {
    update(a, b);  // ok — a and b carry different region labels at this call site
}
```

At each call site, the compiler infers the concrete region labels for `R` and `S` from the arguments. If both type params resolve to the same mutable region, it is a compile-time error:

```resilient
fn bad_caller(&mut[A] int a) {
    update(a, a);  // error: `update` aliases mutable region `A` via args 0 and 1
}
```

Region type parameters are scoped to the function declaration. They cannot be constrained further in V1 — two distinct type params always denote disjoint regions, and the borrow checker enforces that at every call site.

### Z3-assisted alias analysis

The syntactic borrow checker is intentionally conservative. When a function carries `@requires` preconditions that prove its reference parameters are disjoint, the Z3 backend can approve the call even when the syntactic rule would otherwise reject it. This fallback is used automatically; no extra syntax is required.
