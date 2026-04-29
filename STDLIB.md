# Resilient Standard Library

Reference for built-in functions visible in every Resilient program.
Implementations live in `resilient/src/main.rs` (table: `BUILTINS`); type
signatures live in `resilient/src/typechecker.rs` (`prelude` setup).

The canonical, machine-checkable list of names is the `BUILTINS` table.
This document is a human-facing summary grouped by category.

---

## I/O

| Name | Signature | Notes |
|---|---|---|
| `println(x)` | any → void | prints, trailing newline |
| `print(x)` | any → void | no trailing newline; stdout flushed |
| `input(prompt)` | string → string | std-only; line read, EOF → `""` |
| `file_read(path)` | string → Result<String, String> | std-only |
| `file_write(path, contents)` | (string, string) → Result<Void, String> | std-only |
| `env(name)` | string → Result<String, String> | std-only; read-only env-var accessor |

## Numeric

| Name | Signature | Notes |
|---|---|---|
| `abs(x)` | number → number | int or float |
| `min(a, b)` | (number, number) → number | int↔float coercion |
| `max(a, b)` | (number, number) → number | int↔float coercion |
| `clamp(x, lo, hi)` | (number, number, number) → number | restrict to `[lo, hi]`; type-preserving for Int triples, promoted to Float otherwise; runtime error if `lo > hi` |
| `sign(x)` | number → number | RES-410: -1/0/+1 of int or float; NaN passes through |
| `gcd(a, b)` | (int, int) → int | RES-415: Euclidean algorithm on absolute values; gcd(0,0)=0 |
| `lcm(a, b)` | (int, int) → int | RES-415: lcm(0, _) = 0 by convention |
| `is_nan(x)` `is_inf(x)` `is_finite(x)` | number → bool | RES-411: IEEE 754 float predicates; ints flow through as finite |
| `sqrt(x)` | number → float | NaN on negative input |
| `pow(a, b)` | (number, number) → float | `a^b` |
| `floor(x)` | number → float | toward -∞ |
| `ceil(x)` | number → float | toward +∞ |
| `sin(x)` `cos(x)` `tan(x)` | float → float | std-only |
| `atan2(y, x)` | (float, float) → float | std-only; returns angle of `(x, y)` in `(-π, π]` (note `y` first, matching IEEE / C) |
| `ln(x)` `log(x)` `exp(x)` | float → float | std-only; `ln`/`log` runtime error on non-positive args |
| `to_float(x)` | int → float | explicit coercion |
| `to_int(x)` | float → int | explicit coercion |
| `as_int8/16/32/64(x)` | int → int | wrapping truncation to signed width |
| `as_uint8/16/32/64(x)` | int → int | wrapping truncation to unsigned width |
| `random_int(lo, hi)` | (int, int) → int | std-only; SplitMix64 |
| `random_float()` | () → float | std-only |

## Time

| Name | Signature | Notes |
|---|---|---|
| `clock_ms()` | () → int | std-only; monotonic ms |
| `clock_now()` | () → int | std-only; monotonic ns timestamp |
| `clock_elapsed(start)` | int → int | std-only; ns elapsed since `start` |

## String

| Name | Signature | Notes |
|---|---|---|
| `len(s)` | string → int | Unicode-scalar count |
| `split(s, sep)` | (string, string) → array of string | empty `sep` splits into Unicode scalars |
| `trim(s)` | string → string | strips leading/trailing ASCII whitespace |
| `contains(haystack, needle)` | (string, string) → bool | substring test |
| `to_upper(s)` | string → string | ASCII-only uppercase |
| `to_lower(s)` | string → string | ASCII-only lowercase |
| `replace(s, from, to)` | (string, string, string) → string | empty `from` is a hard error |
| `format(fmt, args)` | (string, array) → string | `{}` placeholder; `{{`/`}}` escape |
| `starts_with(s, prefix)` | (string, string) → bool | empty prefix always matches |
| `ends_with(s, suffix)` | (string, string) → bool | empty suffix always matches |
| `repeat(s, n)` | (string, int) → string | `n >= 0`; negative is a hard error |
| `parse_int(s)` | string → Result<Int, String> | base 10; whitespace stripped; `Err` on invalid input — never panics |
| `parse_float(s)` | string → Result<Float, String> | whitespace stripped; `Err` on invalid input — never panics |
| `char_at(s, i)` | (string, int) → Result<String, String> | single-char string at Unicode-scalar index `i`; `Err` on out-of-range or negative |
| `pad_left(s, n, c)` | (string, int, string) → string | left-pad with single char `c` until char-length ≥ `n`; multi-char or empty `c` is a hard error |
| `pad_right(s, n, c)` | (string, int, string) → string | right-pad; same validation as `pad_left` |
| `string_pad_left(s, n, c)` `string_pad_right(s, n, c)` | (string, int, string) → string | RES-429: aliases for `pad_left`/`pad_right` with explicit string-namespace prefix |
| `string_repeat(s, n)` | (string, int) → string | RES-413: alias for `repeat` |
| `string_reverse(s)` | string → string | RES-412: reverse by Unicode scalar |
| `string_chars(s)` | string → array of string | RES-433: split into single-char strings (one per scalar) |
| `string_lines(s)` | string → array of string | RES-434: split on LF/CRLF; trailing newline is not an empty element |
| `string_count(s, sub)` | (string, string) → int | RES-436: non-overlapping occurrence count; empty needle is a typed error |
| `index_of(s, sub)` | (string, string) → int | RES-414: first byte index, or -1; empty needle returns 0 |
| `trim_start(s)` `trim_end(s)` | string → string | RES-438: one-sided Unicode whitespace trimmers |
| `chr(n)` | int → string | RES-419: single-char string for Unicode scalar; surrogate / out-of-range errors |
| `ord(s)` | string → int | RES-419: Unicode scalar of single-character string |
| `to_string(x)` | scalar → string | RES-425: explicit conversion (Int / Float / Bool / String pass-through) |

### Notes on RES-339 parsing builtins

`parse_int` and `parse_float` are explicitly designed to be safe on
untrusted input: they return `Err(message)` on any failure (empty
string, non-numeric characters, overflow) and never panic. This is the
contract that makes them suitable for embedded-target use where an
unwrap on a parse failure cannot be tolerated.

```resilient
let r = parse_int(input("count> "));
match r {
    Ok(n) => println(n),
    Err(msg) => println(msg),
}
```

## Result and Option

| Name | Signature | Notes |
|---|---|---|
| `Ok(v)` | T → Result<T, E> | tag a value as success |
| `Err(e)` | E → Result<T, E> | tag a value as failure |
| `is_ok(r)` `is_err(r)` | Result → bool | tag tests |
| `unwrap(r)` | Result → T | runtime error on `Err` |
| `unwrap_err(r)` | Result → E | runtime error on `Ok` |
| `Some(v)` | T → Option<T> | wrap a present value |
| `None()` | () → Option<T> | the absent option |
| `is_some(o)` `is_none(o)` | Option → bool | tag tests |
| `unwrap_option(o)` | Option<T> → T | runtime error on `None` |
| `option_unwrap(o)` | Option<T> → T | alias of `unwrap_option` |
| `option_unwrap_or(o, d)` | (Option<T>, T) → T | default fallback |

## Collections

### Arrays

| Name | Signature | Notes |
|---|---|---|
| `len(arr)` | array → int | element count |
| `push(arr, x)` | (array, T) → array | returns a new array |
| `pop(arr)` | array → array | runtime error on empty |
| `slice(arr, start, end)` | (array, int, int) → array | half-open `[start, end)` |
| `array_reverse(arr)` | array → array | RES-412: new array with elements reversed; clones |
| `array_concat(a, b)` | (array, array) → array | RES-420: returns a + b; heterogeneous element types allowed |
| `array_take(arr, n)` `array_drop(arr, n)` | (array, int) → array | RES-421: first n / skip first n; clamped at len |
| `array_split_at(arr, n)` | (array, int) → (array, array) | RES-439: bisect into `(first n, rest)` tuple |
| `array_chunk(arr, n)` | (array, int) → array of array | RES-435: fixed-size chunks; last may be short; n > 0 |
| `array_flatten(arr)` | array of array → array | RES-423: concatenate inner arrays one level |
| `array_join(arr, sep)` | (array, string) → string | RES-424: join string elements with separator |
| `array_intersperse(arr, x)` | (array, T) → array | RES-437: insert x between adjacent elements |
| `array_zip(a, b)` | (array, array) → array of tuple | RES-430: pair as 2-tuples; truncate to shorter |
| `array_range(start, end)` | (int, int) → array of int | RES-431: half-open integer range; capped at 1B |
| `array_repeat(elem, n)` | (T, int) → array | RES-432: array of n clones of elem; capped at 1B |
| `array_first(arr)` `array_last(arr)` | array → T | RES-428: endpoint accessors; empty array errors |
| `array_min(arr)` `array_max(arr)` | array of int → int | RES-417: min/max over int array; empty errors |
| `array_sum(arr)` `array_product(arr)` | array of int → int | RES-416: identity 0 / 1 for empty |
| `array_sort(arr)` | array of int → array of int | RES-422: ascending sort; new array, input unchanged |
| `array_unique(arr)` | array → array | RES-426: first-occurrence dedupe; non-scalar elements error |
| `array_contains(arr, x)` | (array, T) → bool | RES-418: scalar value-equality (Int↔Float coerce) |
| `array_index_of(arr, x)` | (array, T) → int | RES-418: first matching index, or -1 |
| `array_count(arr, x)` | (array, T) → int | RES-427: number of matching elements |

### Maps

| Name | Signature | Notes |
|---|---|---|
| `map_new()` | () → map | empty map |
| `map_insert(m, k, v)` | (map, K, V) → map | new map with insertion |
| `map_get(m, k)` | (map, K) → Result<V, String> | `Err("not found")` if absent |
| `map_remove(m, k)` | (map, K) → map | new map with key removed |
| `map_keys(m)` | map → array | all keys, sorted for determinism |
| `map_len(m)` | map → int | entry count |

### HashMap (RES-293)

`hashmap_*` are the user-facing names for the same backing storage as
the `map_*` builtins above. They share the same key restriction
(`Int`, `String`, or `Bool` — anything else is a runtime error) and
the same immutable-value semantics (each mutation returns a new map).

| Name | Signature | Notes |
|---|---|---|
| `hashmap_new()` | () → hashmap | empty HashMap |
| `hashmap_insert(m, k, v)` | (hashmap, K, V) → hashmap | new map with insertion / overwrite |
| `hashmap_get(m, k)` | (hashmap, K) → Result<V, String> | `Ok(v)` or `Err("not found")` |
| `hashmap_remove(m, k)` | (hashmap, K) → hashmap | no-op when key missing |
| `hashmap_contains(m, k)` | (hashmap, K) → bool | membership test |
| `hashmap_keys(m)` | hashmap → array | keys, sorted for determinism |

### Sets

| Name | Signature | Notes |
|---|---|---|
| `set_new()` | () → set | empty set |
| `set_insert(s, x)` | (set, T) → set | new set with insertion |
| `set_remove(s, x)` | (set, T) → set | new set with element removed |
| `set_has(s, x)` | (set, T) → bool | membership test |
| `set_len(s)` | set → int | element count |
| `set_items(s)` | set → array | snapshot of items |

### Bytes

| Name | Signature | Notes |
|---|---|---|
| `bytes_len(b)` | bytes → int | byte count |
| `bytes_slice(b, start, end)` | (bytes, int, int) → bytes | half-open range |
| `byte_at(b, i)` | (bytes, int) → int | byte at index |

## Bitwise (RES-440)

| Name | Signature | Notes |
|---|---|---|
| `bit_and(a, b)` `bit_or(a, b)` `bit_xor(a, b)` | (int, int) → int | bitwise binary ops on i64 |
| `bit_not(a)` | int → int | one's complement |
| `bit_shl(a, n)` `bit_shr(a, n)` | (int, int) → int | shift amount must be 0..=63; arithmetic right shift |

## Live blocks (RES-138, RES-141)

| Name | Signature | Notes |
|---|---|---|
| `live_retries()` | () → int | current retry count inside an active live block |
| `live_total_retries()` | () → int | process-wide retry counter |
| `live_total_exhaustions()` | () → int | process-wide exhaustion counter |

## Linear-type machinery (RES-385)

| Name | Signature | Notes |
|---|---|---|
| `drop(v)` | T → void | explicitly consumes a linear value |

## StringBuilder (RES-353)

| Name | Signature | Notes |
|---|---|---|
| `StringBuilder_new()` | () → StringBuilder | construct an empty builder |

Methods on a builder (`b.append(x)`, `b.to_string()`, etc.) are
dispatched via the special StringBuilder method handler in
`CallExpression` evaluation.

---

When adding a new builtin, the canonical list to update is:

1. The `BUILTINS` table in `resilient/src/main.rs`.
2. The type signature in the prelude block of `resilient/src/typechecker.rs`.
3. The `PURE_BUILTINS` list in `resilient/src/typechecker.rs` (unless impure).
4. A row in this file and in `SYNTAX.md`.
5. A unit test in `resilient/src/main.rs`'s `mod tests`.
