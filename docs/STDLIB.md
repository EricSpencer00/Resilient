# Resilient Standard Library Reference

Complete reference for every builtin function available in Resilient.

## Summary

| Category | Builtins |
|---|---|
| I/O | `print`, `println`, `input` |
| Math (basic) | `abs`, `min`, `max`, `clamp`, `to_float`, `to_int` |
| Math (float) | `sqrt`, `pow`, `floor`, `ceil`, `sin`, `cos`, `tan`, `atan2`, `ln`, `log`, `exp` |
| Bit casting | `as_int8`, `as_int16`, `as_int32`, `as_int64`, `as_uint8`, `as_uint16`, `as_uint32`, `as_uint64` |
| Time | `clock_ms`, `clock_now`, `clock_elapsed` |
| Random | `random_int`, `random_float` |
| String | `len`, `push`, `pop`, `slice`, `split`, `trim`, `contains`, `to_upper`, `to_lower`, `replace`, `format`, `starts_with`, `ends_with`, `repeat`, `char_at`, `pad_left`, `pad_right` |
| Parsing | `parse_int`, `parse_float` |
| Bytes | `bytes_len`, `bytes_slice`, `byte_at` |
| Result | `Ok`, `Err`, `is_ok`, `is_err`, `unwrap`, `unwrap_err` |
| Option | `Some`, `None`, `is_some`, `is_none`, `unwrap_option`, `option_unwrap`, `option_unwrap_or` |
| Collections | `map_*`, `hashmap_*`, `set_*` (see below) |
| File I/O | `file_read`, `file_write` |
| Environment | `env` |
| Control | `drop` |
| Live blocks | `live_retries`, `live_total_retries`, `live_total_exhaustions` |
| Other | `StringBuilder_new`, `cell` |

---

## I/O Functions

### `print`
**Signature:** `print() -> void` | `print(x: any) -> void`

Print a value to stdout without a newline. Takes 0 or 1 argument. Flushes stdout.

**Example:**
```rust
print("Hello");
print(42);
```

### `println`
**Signature:** `println() -> void` | `println(x: any) -> void`

Print a value to stdout with a trailing newline. Takes 0 or 1 argument.

**Example:**
```rust
println("Hello, world!");
println(x + 10);
```

### `input`
**Signature:** `input() -> string`

Read a single line from stdin (strips trailing newline).

**Example:**
```rust
let name = input();
println("Hello, " + name);
```

---

## Basic Math Functions

### `abs`
**Signature:** `abs(x: int) -> int` | `abs(x: float) -> float`

Return the absolute value.

**Example:**
```rust
abs(-5);           // 5
abs(-3.14);        // 3.14
```

### `min`
**Signature:** `min(a: int, b: int) -> int` | `min(a: float, b: float) -> float`

Return the smaller of two values.

**Example:**
```rust
min(3, 7);         // 3
min(2.5, 1.8);     // 1.8
```

### `max`
**Signature:** `max(a: int, b: int) -> int` | `max(a: float, b: float) -> float`

Return the larger of two values.

**Example:**
```rust
max(3, 7);         // 7
max(2.5, 1.8);     // 2.5
```

### `clamp`
**Signature:** `clamp(x: int, lo: int, hi: int) -> int` | `clamp(x: float, lo: float, hi: float) -> float`

Restrict `x` to the range `[lo, hi]`. Returns error if `lo > hi`.

**Example:**
```rust
clamp(5, 1, 10);   // 5
clamp(-1, 0, 10);  // 0
clamp(15, 0, 10);  // 10
```

### `to_float`
**Signature:** `to_float(x: int) -> float`

Convert an integer to a float.

**Example:**
```rust
to_float(42);      // 42.0
```

### `to_int`
**Signature:** `to_int(x: float) -> int`

Convert a float to an integer (truncates toward zero).

**Example:**
```rust
to_int(3.9);       // 3
to_int(-2.5);      // -2
```

---

## Floating-Point Math Functions

### `sqrt`
**Signature:** `sqrt(x: float) -> float`

Return the square root of `x`.

**Example:**
```rust
sqrt(16.0);        // 4.0
sqrt(2.0);         // ~1.414
```

### `pow`
**Signature:** `pow(base: float, exp: float) -> float`

Return `base` raised to the power `exp`.

**Example:**
```rust
pow(2.0, 3.0);     // 8.0
pow(10.0, 2.0);    // 100.0
```

### `floor`
**Signature:** `floor(x: float) -> float`

Return the largest integer ≤ `x`.

**Example:**
```rust
floor(3.9);        // 3.0
floor(-2.1);       // -3.0
```

### `ceil`
**Signature:** `ceil(x: float) -> float`

Return the smallest integer ≥ `x`.

**Example:**
```rust
ceil(3.1);         // 4.0
ceil(-2.9);        // -2.0
```

### `sin`
**Signature:** `sin(x: float) -> float`

Return the sine of `x` (in radians).

**Example:**
```rust
sin(0.0);          // 0.0
sin(3.14159 / 2);  // ~1.0
```

### `cos`
**Signature:** `cos(x: float) -> float`

Return the cosine of `x` (in radians).

**Example:**
```rust
cos(0.0);          // 1.0
cos(3.14159);      // ~-1.0
```

### `tan`
**Signature:** `tan(x: float) -> float`

Return the tangent of `x` (in radians).

**Example:**
```rust
tan(0.0);          // 0.0
```

### `atan2`
**Signature:** `atan2(y: float, x: float) -> float`

Return the arctangent of `y / x` (in radians), accounting for quadrant.

**Example:**
```rust
atan2(1.0, 1.0);   // π/4 (~0.785)
```

### `ln`
**Signature:** `ln(x: float) -> float`

Return the natural logarithm (base e) of `x`.

**Example:**
```rust
ln(2.71828);       // ~1.0
```

### `log`
**Signature:** `log(x: float, base: float) -> float`

Return the logarithm of `x` in the given `base`.

**Example:**
```rust
log(100.0, 10.0);  // 2.0
log(8.0, 2.0);     // 3.0
```

### `exp`
**Signature:** `exp(x: float) -> float`

Return e raised to the power `x`.

**Example:**
```rust
exp(0.0);          // 1.0
exp(1.0);          // ~2.71828
```

---

## Bit-Casting Functions

Cast values to fixed-width integer types with wrapping truncation.

### `as_int8`, `as_int16`, `as_int32`, `as_int64`
**Signature:** `as_intN(x: int) -> int`

Cast to signed N-bit integer (wrapping).

**Example:**
```rust
as_int8(256);      // 0 (wraps)
as_int16(-1);      // -1
```

### `as_uint8`, `as_uint16`, `as_uint32`, `as_uint64`
**Signature:** `as_uintN(x: int) -> int`

Cast to unsigned N-bit integer (wrapping).

**Example:**
```rust
as_uint8(256);     // 0 (wraps)
as_uint8(-1);      // 255 (wraps)
```

---

## Time Functions

### `clock_ms`
**Signature:** `clock_ms() -> int`

Return milliseconds elapsed since an unspecified epoch.

**Example:**
```rust
let t1 = clock_ms();
// ... do work ...
let t2 = clock_ms();
println(t2 - t1);  // milliseconds elapsed
```

### `clock_now`
**Signature:** `clock_now() -> int`

Return the current Unix timestamp in seconds.

**Example:**
```rust
let now = clock_now();
println(now);      // seconds since 1970-01-01
```

### `clock_elapsed`
**Signature:** `clock_elapsed(start: int) -> int`

Return milliseconds elapsed since `start` (from `clock_ms()`).

**Example:**
```rust
let t0 = clock_ms();
// ... do work ...
println(clock_elapsed(t0));  // ms elapsed
```

---

## Random Functions

### `random_int`
**Signature:** `random_int(max: int) -> int`

Return a random integer in `[0, max)`.

**Example:**
```rust
let dice = random_int(6) + 1;  // 1..6
```

### `random_float`
**Signature:** `random_float() -> float`

Return a random float in `[0.0, 1.0)`.

**Example:**
```rust
let x = random_float();        // 0.0 <= x < 1.0
```

---

## String Functions

### `len`
**Signature:** `len(s: string) -> int`

Return the length of a string (byte count).

**Example:**
```rust
len("hello");      // 5
len("");           // 0
```

### `push`
**Signature:** `push(s: string, c: string) -> string`

Append a single character (or string) to the end.

**Example:**
```rust
push("hello", "!");    // "hello!"
```

### `pop`
**Signature:** `pop(s: string) -> string`

Remove and return the last character; returns original if empty.

**Example:**
```rust
pop("hello");      // "h" (and mutates if mutable)
pop("");           // ""
```

### `slice`
**Signature:** `slice(s: string, start: int, end: int) -> string`

Return substring from index `start` (inclusive) to `end` (exclusive).

**Example:**
```rust
slice("hello", 1, 4);  // "ell"
```

### `split`
**Signature:** `split(s: string, sep: string) -> [string]`

Split string by separator; returns a static array (up to 255 elements).

**Example:**
```rust
split("a,b,c", ",");   // ["a", "b", "c"]
```

### `trim`
**Signature:** `trim(s: string) -> string`

Remove leading and trailing whitespace.

**Example:**
```rust
trim("  hello  ");     // "hello"
```

### `contains`
**Signature:** `contains(haystack: string, needle: string) -> bool`

Check if `haystack` contains `needle`.

**Example:**
```rust
contains("hello world", "world");  // true
contains("hello", "x");            // false
```

### `to_upper`
**Signature:** `to_upper(s: string) -> string`

Convert to uppercase (ASCII only).

**Example:**
```rust
to_upper("Hello");     // "HELLO"
```

### `to_lower`
**Signature:** `to_lower(s: string) -> string`

Convert to lowercase (ASCII only).

**Example:**
```rust
to_lower("Hello");     // "hello"
```

### `replace`
**Signature:** `replace(s: string, old: string, new: string) -> string`

Replace all occurrences of `old` with `new`.

**Example:**
```rust
replace("hello world", "world", "Resilient");  // "hello Resilient"
```

### `format`
**Signature:** `format(fmt: string, args: [any]) -> string`

Format a string (simple `%s` placeholder support).

**Example:**
```rust
format("Value: %s", ["42"]);   // "Value: 42"
```

### `starts_with`
**Signature:** `starts_with(s: string, prefix: string) -> bool`

Check if string starts with prefix.

**Example:**
```rust
starts_with("hello", "hel");   // true
starts_with("hello", "bye");   // false
```

### `ends_with`
**Signature:** `ends_with(s: string, suffix: string) -> bool`

Check if string ends with suffix.

**Example:**
```rust
ends_with("hello.txt", ".txt"); // true
```

### `repeat`
**Signature:** `repeat(s: string, n: int) -> string`

Return a string containing `s` repeated `n` times.

**Example:**
```rust
repeat("ab", 3);       // "ababab"
```

### `char_at`
**Signature:** `char_at(s: string, idx: int) -> string`

Return the character at index `idx` (or empty string if out of bounds).

**Example:**
```rust
char_at("hello", 0);   // "h"
char_at("hello", 10);  // ""
```

### `pad_left`
**Signature:** `pad_left(s: string, len: int, pad: string) -> string`

Pad string on the left to width `len` using character `pad`.

**Example:**
```rust
pad_left("5", 3, "0"); // "005"
```

### `pad_right`
**Signature:** `pad_right(s: string, len: int, pad: string) -> string`

Pad string on the right to width `len` using character `pad`.

**Example:**
```rust
pad_right("5", 3, "0"); // "500"
```

---

## Parsing Functions

### `parse_int`
**Signature:** `parse_int(s: string) -> Result[int]`

Parse a string as a decimal integer.

**Example:**
```rust
parse_int("42");       // Ok(42)
parse_int("hello");    // Err("invalid integer")
```

### `parse_float`
**Signature:** `parse_float(s: string) -> Result[float]`

Parse a string as a floating-point number.

**Example:**
```rust
parse_float("3.14");   // Ok(3.14)
parse_float("abc");    // Err("invalid float")
```

---

## Bytes Functions

### `bytes_len`
**Signature:** `bytes_len(data: [u8]) -> int`

Return the length of a byte array.

**Example:**
```rust
bytes_len([1, 2, 3]); // 3
```

### `bytes_slice`
**Signature:** `bytes_slice(data: [u8], start: int, end: int) -> [u8]`

Return a slice of bytes from `start` to `end`.

**Example:**
```rust
bytes_slice([1, 2, 3, 4], 1, 3); // [2, 3]
```

### `byte_at`
**Signature:** `byte_at(data: [u8], idx: int) -> int`

Return the byte value at index `idx`.

**Example:**
```rust
byte_at([65, 66, 67], 0); // 65 (ASCII 'A')
```

---

## Result Functions

### `Ok`
**Signature:** `Ok[T](value: T) -> Result[T]`

Construct a success result.

**Example:**
```rust
let r: Result[int] = Ok(42);
```

### `Err`
**Signature:** `Err[T](msg: string) -> Result[T]`

Construct an error result.

**Example:**
```rust
let r: Result[int] = Err("something went wrong");
```

### `is_ok`
**Signature:** `is_ok(r: Result[T]) -> bool`

Check if a result is `Ok`.

**Example:**
```rust
is_ok(Ok(42));  // true
is_ok(Err("no")); // false
```

### `is_err`
**Signature:** `is_err(r: Result[T]) -> bool`

Check if a result is `Err`.

**Example:**
```rust
is_err(Err("oops")); // true
is_err(Ok(0));       // false
```

### `unwrap`
**Signature:** `unwrap[T](r: Result[T]) -> T`

Extract the value from `Ok`, or halt with the error message if `Err`.

**Example:**
```rust
let x = unwrap(Ok(42));  // 42
let y = unwrap(Err("fail")); // halts with "fail"
```

### `unwrap_err`
**Signature:** `unwrap_err[T](r: Result[T]) -> string`

Extract the error message from `Err`, or halt if `Ok`.

**Example:**
```rust
let msg = unwrap_err(Err("oops")); // "oops"
```

---

## Option Functions

### `Some`
**Signature:** `Some[T](value: T) -> Option[T]`

Construct a present option.

**Example:**
```rust
let x: Option[int] = Some(42);
```

### `None`
**Signature:** `None[T]() -> Option[T]`

Construct an absent option.

**Example:**
```rust
let x: Option[int] = None();
```

### `is_some`
**Signature:** `is_some(opt: Option[T]) -> bool`

Check if an option is `Some`.

**Example:**
```rust
is_some(Some(5));   // true
is_some(None());    // false
```

### `is_none`
**Signature:** `is_none(opt: Option[T]) -> bool`

Check if an option is `None`.

**Example:**
```rust
is_none(None());    // true
is_none(Some(5));   // false
```

### `unwrap_option`
**Signature:** `unwrap_option[T](opt: Option[T]) -> T`

Extract the value from `Some`, or halt if `None`.

**Example:**
```rust
let x = unwrap_option(Some(42)); // 42
let y = unwrap_option(None()); // halts
```

### `option_unwrap`
**Signature:** `option_unwrap[T](opt: Option[T]) -> T`

Alias for `unwrap_option`.

**Example:**
```rust
option_unwrap(Some(10)); // 10
```

### `option_unwrap_or`
**Signature:** `option_unwrap_or[T](opt: Option[T], default: T) -> T`

Extract the value from `Some`, or return `default` if `None`.

**Example:**
```rust
option_unwrap_or(Some(5), 0); // 5
option_unwrap_or(None(), 0);  // 0
```

---

## Collection Functions

### Map Functions (ordered)

#### `map_new`
**Signature:** `map_new[K, V]() -> Map[K, V]`

Create an empty ordered map.

#### `map_insert`
**Signature:** `map_insert[K, V](m: Map[K, V], key: K, value: V) -> Map[K, V]`

Insert or update a key-value pair.

#### `map_get`
**Signature:** `map_get[K, V](m: Map[K, V], key: K) -> Option[V]`

Look up a key; returns `Some(value)` or `None()`.

#### `map_remove`
**Signature:** `map_remove[K, V](m: Map[K, V], key: K) -> Map[K, V]`

Remove a key (no-op if not present).

#### `map_keys`
**Signature:** `map_keys[K, V](m: Map[K, V]) -> [K]`

Return all keys as a static array.

#### `map_len`
**Signature:** `map_len[K, V](m: Map[K, V]) -> int`

Return the number of key-value pairs.

**Example:**
```rust
let m = map_new();
let m = map_insert(m, "name", "Alice");
let v = map_get(m, "name"); // Some("Alice")
```

### HashMap Functions (unordered, faster)

#### `hashmap_new`
**Signature:** `hashmap_new[K, V]() -> HashMap[K, V]`

Create an empty unordered hashmap.

#### `hashmap_insert`
**Signature:** `hashmap_insert[K, V](m: HashMap[K, V], key: K, value: V) -> HashMap[K, V]`

Insert or update a key-value pair.

#### `hashmap_get`
**Signature:** `hashmap_get[K, V](m: HashMap[K, V], key: K) -> Option[V]`

Look up a key; returns `Some(value)` or `None()`.

#### `hashmap_remove`
**Signature:** `hashmap_remove[K, V](m: HashMap[K, V], key: K) -> HashMap[K, V]`

Remove a key (no-op if not present).

#### `hashmap_contains`
**Signature:** `hashmap_contains[K, V](m: HashMap[K, V], key: K) -> bool`

Check if a key exists.

#### `hashmap_keys`
**Signature:** `hashmap_keys[K, V](m: HashMap[K, V]) -> [K]`

Return all keys as a static array.

**Example:**
```rust
let m = hashmap_new();
let m = hashmap_insert(m, "x", 10);
let found = hashmap_contains(m, "x"); // true
```

### Set Functions

#### `set_new`
**Signature:** `set_new[T]() -> Set[T]`

Create an empty set.

#### `set_insert`
**Signature:** `set_insert[T](s: Set[T], value: T) -> Set[T]`

Insert a value (no-op if already present).

#### `set_remove`
**Signature:** `set_remove[T](s: Set[T], value: T) -> Set[T]`

Remove a value (no-op if not present).

#### `set_has`
**Signature:** `set_has[T](s: Set[T], value: T) -> bool`

Check if a value is in the set.

#### `set_len`
**Signature:** `set_len[T](s: Set[T]) -> int`

Return the number of elements.

#### `set_items`
**Signature:** `set_items[T](s: Set[T]) -> [T]`

Return all elements as a static array.

**Example:**
```rust
let s = set_new();
let s = set_insert(s, 1);
let s = set_insert(s, 2);
let has_1 = set_has(s, 1); // true
```

---

## File I/O Functions

### `file_read`
**Signature:** `file_read(path: string) -> Result[string]`

Read the entire contents of a file into a string.

**Example:**
```rust
let contents = file_read("data.txt");
match contents {
    Ok(data) => println(data),
    Err(msg) => println("Error: " + msg),
}
```

### `file_write`
**Signature:** `file_write(path: string, data: string) -> Result[void]`

Write data to a file (creates or overwrites).

**Example:**
```rust
file_write("output.txt", "Hello, world!");
```

---

## Environment Functions

### `env`
**Signature:** `env(key: string, default_value: string) -> string`

Get an environment variable, or return a default value if not set.

**Example:**
```rust
let user = env("USER", "anonymous");
println(user);
```

---

## Control Functions

### `drop`
**Signature:** `drop[T](value: T) -> void`

Explicitly consume (drop) a value. Useful in linear-type contexts to mark a value as intentionally unused.

**Example:**
```rust
let x = expensive_computation();
drop(x);  // Mark as consumed
```

---

## Live Block Telemetry Functions

### `live_retries`
**Signature:** `live_retries() -> int`

Return the retry count inside the currently executing live block.

**Example:**
```rust
live {
    if live_retries() > 100 {
        println("Exhausted retries");
    }
    // ... recovery code ...
}
```

### `live_total_retries`
**Signature:** `live_total_retries() -> int`

Return the total number of retries across all live blocks in the program.

### `live_total_exhaustions`
**Signature:** `live_total_exhaustions() -> int`

Return the number of live blocks that have exhausted their retry limit.

---

## Utility Functions

### `StringBuilder_new`
**Signature:** `StringBuilder_new() -> StringBuilder`

Create a new string builder (for efficient string concatenation).

**Example:**
```rust
let sb = StringBuilder_new();
// Use in I/O or specialized contexts
```

### `cell`
**Signature:** `cell[T](value: T) -> Cell[T]`

Wrap a value in a `Cell` for interior mutability in `no_std` contexts.

**Example:**
```rust
let x = cell(42);
// Use in specific memory-safe patterns
```

---

## Notes

- **String operations** work on UTF-8 text; byte count may differ from character count.
- **Random functions** are seeded from system entropy (not cryptographically secure).
- **File I/O** uses the process's current working directory.
- **Collections** return immutable copies; use the returned value for persistence.
- **Effect annotations** (e.g., `@io`, `@pure`) will be documented once the effect system lands.
