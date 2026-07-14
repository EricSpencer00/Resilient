# Resilient Standard Library

Reference for built-in functions visible in every Resilient program.
Implementations live in `resilient/src/lib.rs` (table: `BUILTINS`); type
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
| `int_min()` `int_max()` | () → int | RES-447: i64::MIN / i64::MAX boundary constants |
| `min3(a, b, c)` `max3(a, b, c)` | (number, number, number) → number | RES-473: ternary numeric min/max with same int↔float coercion as `min`/`max` |
| `sqrt(x)` | number → float | NaN on negative input |
| `pow(a, b)` | (number, number) → float | `a^b` |
| `floor(x)` | number → float | toward -∞ |
| `ceil(x)` | number → float | toward +∞ |
| `sin(x)` `cos(x)` `tan(x)` | float → float | std-only |
| `to_radians(d)` | float → float | RES-894: convert degrees to radians; std-only |
| `to_degrees(r)` | float → float | RES-895: convert radians to degrees; std-only |
| `atan2(y, x)` | (float, float) → float | std-only; returns angle of `(x, y)` in `(-π, π]` (note `y` first, matching IEEE / C) |
| `hypot(x, y)` | (float, float) → float | RES-892: sqrt(x² + y²) without overflow; std-only |
| `copysign(x, y)` | (float, float) → float | RES-893: magnitude of x with sign of y; std-only |
| `ln(x)` `log(x)` `exp(x)` | float → float | std-only; `ln`/`log` runtime error on non-positive args |
| `log10(x)` | float → float | RES-889: base-10 logarithm; std-only; runtime error on non-positive |
| `log2(x)` | float → float | RES-890: base-2 logarithm; std-only; runtime error on non-positive |
| `exp2(x)` | float → float | RES-891: 2^x; std-only; mirror of `exp` (e^x) |
| `sinh(x)` | float → float | RES-896: hyperbolic sine; std-only; mirror of `sin` |
| `cosh(x)` | float → float | RES-897: hyperbolic cosine; std-only; mirror of `cos` |
| `tanh(x)` | float → float | RES-898: hyperbolic tangent; std-only; mirror of `tan`; saturates to ±1 |
| `asinh(x)` | float → float | RES-899: inverse hyperbolic sine; std-only; total domain (no NaN cases) |
| `acosh(x)` | float → float | RES-900: inverse hyperbolic cosine; std-only; domain `x ≥ 1` (NaN otherwise) |
| `atanh(x)` | float → float | RES-901: inverse hyperbolic tangent; std-only; domain `(-1, 1)`; `±1` → ±∞; `|x|>1` → NaN |
| `asin(x)` | float → float | RES-902: inverse sine (radians); std-only; domain `[-1, 1]`; `|x|>1` → NaN; range `[-π/2, π/2]` |
| `acos(x)` | float → float | RES-903: inverse cosine (radians); std-only; domain `[-1, 1]`; `|x|>1` → NaN; range `[0, π]` |
| `atan(x)` | float → float | RES-904: inverse tangent (radians, single arg); std-only; total domain; range `(-π/2, π/2)`; sibling of `atan2(y, x)` |
| `cbrt(x)` | float → float | RES-905: cube root; std-only; total domain (handles negatives, unlike `sqrt`); odd |
| `count_ones(x)` `count_zeros(x)` | int → int | RES-907: 64-bit two's-complement bit population / complement |
| `leading_zeros(x)` `trailing_zeros(x)` | int → int | RES-907: count of leading / trailing zero bits; both return `64` for input `0` |
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
| `last_index_of(s, sub)` | (string, string) → int | RES-442: last byte index of `sub` in `s`, or -1; empty needle returns `len(s)` |
| `string_find_all(s, sub)` | (string, string) → array of int | RES-446: every non-overlapping match index; empty needle is a typed error |
| `string_at(s, i)` | (string, int) → string | RES-453: i-th Unicode scalar as a single-char string; out-of-range / negative is a typed error |
| `string_substring(s, start, end)` | (string, int, int) → string | RES-454: half-open Unicode-scalar slice; indices clamped; start > end errors |
| `string_capitalize(s)` | string → string | RES-457: ASCII first char upper, rest lower |
| `string_bytes_len(s)` | string → int | RES-463: UTF-8 byte length (vs `len` which counts scalars) |
| `string_indent(s, n)` | (string, int) → string | RES-461: prefix every line with n spaces; trailing newline preserved |
| `trim_chars(s, chars)` | (string, string) → string | RES-460: strip arbitrary char set from both sides |
| `is_ascii_alpha(s)` `is_ascii_digit(s)` `is_ascii_alnum(s)` | string → bool | RES-459: every-char ASCII-class predicates; empty is vacuously true |
| `parse_int_base(s, base)` | (string, int) → Result<Int, String> | RES-464: parse with explicit radix (2..=36); whitespace stripped |
| `int_to_base(n, base)` | (int, int) → string | RES-465: render with explicit radix; round-trips with `parse_int_base` |
| `string_strip_prefix(s, prefix)` `string_strip_suffix(s, suffix)` | (string, string) → string | RES-471: conditional removers; if absent returns s unchanged |

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
| `result_unwrap_or(r, d)` | (Result<T, E>, T) → T | RES-936: Ok payload, or `d` on Err — never panics |
| `result_unwrap_or_err(r, d)` | (Result<T, E>, E) → E | RES-937: Err payload, or `d` on Ok — symmetric to `result_unwrap_or` |
| `result_to_option(r)` | Result<T, E> → Option<T> | RES-938: `Ok(v)` → `Some(v)`, `Err(_)` → `None` |
| `option_to_result(o, e)` | (Option<T>, E) → Result<T, E> | RES-938: `Some(v)` → `Ok(v)`, `None` → `Err(e)` |
| `option_or(a, b)` | (Option<T>, Option<T>) → Option<T> | RES-939: `Some(_)` returns `a`; `None` returns `b` (chain alternatives) |
| `result_or(a, b)` | (Result<T, E>, Result<T, E>) → Result<T, E> | RES-939: `Ok(_)` returns `a`; `Err(_)` returns `b` |

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
| `array_position(arr, x, start)` | (array, T, int) → int | RES-448: array_index_of starting at `start` (clamped at 0); -1 if absent |
| `array_swap(arr, i, j)` | (array, int, int) → array | RES-450: bounds-checked element exchange; new array |
| `array_insert_at(arr, i, x)` | (array, int, T) → array | RES-451: insert at i; valid range [0, len]; i==len appends |
| `array_remove_at(arr, i)` | (array, int) → array | RES-451: remove at i; valid range [0, len) |
| `array_set_at(arr, i, x)` | (array, int, T) → array | RES-452: replace element at i; bounds-checked |
| `array_remove(arr, x)` | (array, T) → array | RES-466: drop the first element matching x; clone if absent |
| `array_remove_all(arr, x)` | (array, T) → array | RES-467: drop every matching element |
| `array_dedup(arr)` | array → array | RES-468: collapse adjacent duplicates (vs array_unique which dedupes globally) |
| `array_all_eq(arr, x)` | (array, T) → bool | RES-469: every element equals x; empty is vacuously true |
| `array_any_eq(arr, x)` | (array, T) → bool | RES-469: alias for `array_contains` |
| `array_eq(a, b)` | (array, array) → bool | RES-472: element-wise scalar equality; empty arrays equal |
| `array_ne(a, b)` | (array, array) → bool | RES-474: negation of `array_eq` |
| `array_fold_int(arr, init, op)` | (array, int, string) → int | RES-475: fold with named op (sum/product/min/max) starting from `init` |
| `array_starts_with(arr, prefix)` `array_ends_with(arr, suffix)` | (array, array) → bool | RES-445: scalar value-equality on element prefixes/suffixes |
| `array_window(arr, n)` | (array, int) → array of array | RES-455: sliding windows; n must be > 0 |
| `array_pairs(arr)` | array → array of tuple | RES-462: adjacent 2-tuples (`array_window` analog yielding tuples) |
| `array_rotate_left(arr, n)` `array_rotate_right(arr, n)` | (array, int) → array | RES-456: cyclic shift; n reduced modulo len |
| `array_shuffle(arr)` | array → array | RES-444: Fisher-Yates random permutation; impure (RNG) |
| `array_pad_left(arr, n, fill)` `array_pad_right(arr, n, fill)` | (array, int, T) → array | RES-449: pad to length n with fill |
| `array_cycle(arr, n)` | (array, int) → array | RES-458: concatenate arr to itself n times; cap 1B |
| `array_sort_desc(arr)` | array of int → array of int | RES-443: descending sort |
| `array_average(arr)` | array of int → float | RES-941: arithmetic mean as Float; empty errors |
| `array_median(arr)` | array of int → float | RES-941: middle element of sorted array; even-length returns mean of two middles; empty errors |
| `array_sum_float(arr)` | array of float → float | RES-942: float-array sum; identity 0.0 on empty |
| `array_product_float(arr)` | array of float → float | RES-942: float-array product; identity 1.0 on empty |
| `array_min_float(arr)` `array_max_float(arr)` | array of float → float | RES-942: float-array min/max; NaN propagates; empty errors |
| `array_average_float(arr)` | array of float → float | RES-942: float-array mean; empty errors |

### Maps

| Name | Signature | Notes |
|---|---|---|
| `map_new()` | () → map | empty map |
| `map_insert(m, k, v)` | (map, K, V) → map | new map with insertion |
| `map_get(m, k)` | (map, K) → Result<V, String> | `Err("not found")` if absent |
| `map_remove(m, k)` | (map, K) → map | new map with key removed |
| `map_keys(m)` | map → array | all keys, sorted for determinism |
| `map_len(m)` | map → int | entry count |
| `map_values(m)` | map → array | RES-883: all values in same key-sorted order as `map_keys` |
| `map_contains_key(m, k)` | (map, K) → bool | RES-884: membership test; mirrors `hashmap_contains` |
| `map_get_or(m, k, default)` | (map, K, V) → V | RES-945: value at key, or `default` if missing — saves writing `match` over `map_get` |

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
| `hashmap_len(m)` | hashmap → int | RES-885: entry count; mirrors `map_len` |
| `hashmap_values(m)` | hashmap → array | RES-886: values in same key-sorted order as `hashmap_keys` |
| `hashmap_get_or(m, k, default)` | (hashmap, K, V) → V | RES-945: same default-fallback shape as `map_get_or` |

### Sets

| Name | Signature | Notes |
|---|---|---|
| `set_new()` | () → set | empty set |
| `set_insert(s, x)` | (set, T) → set | new set with insertion |
| `set_remove(s, x)` | (set, T) → set | new set with element removed |
| `set_has(s, x)` | (set, T) → bool | membership test |
| `set_len(s)` | set → int | element count |
| `set_items(s)` | set → array | snapshot of items |
| `set_union(a, b)` | (set, set) → set | RES-876: every element in either set; deduped |
| `set_intersection(a, b)` | (set, set) → set | RES-877: only elements present in both inputs |
| `set_difference(a, b)` | (set, set) → set | RES-878: elements in `a` but not in `b` |
| `set_is_subset(a, b)` | (set, set) → bool | RES-879: true iff every element of `a` is in `b`; empty is subset of all |
| `set_is_superset(a, b)` | (set, set) → bool | RES-880: true iff every element of `b` is in `a` |
| `set_is_disjoint(a, b)` | (set, set) → bool | RES-881: true iff the two sets share no elements |
| `set_symmetric_difference(a, b)` | (set, set) → set | RES-882: elements in either set but not both (XOR) |

### Bytes

| Name | Signature | Notes |
|---|---|---|
| `bytes_len(b)` | bytes → int | byte count |
| `bytes_slice(b, start, end)` | (bytes, int, int) → bytes | half-open range |
| `byte_at(b, i)` | (bytes, int) → int | byte at index |
| `bytes_concat(a, b)` | (bytes, bytes) → bytes | RES-887: a followed by b; inputs unchanged |
| `bytes_eq(a, b)` | (bytes, bytes) → bool | RES-888: byte-equality of two Bytes values |
| `bytes_starts_with(h, p)` | (bytes, bytes) → bool | RES-944: prefix predicate; empty prefix is always true |
| `bytes_ends_with(h, s)` | (bytes, bytes) → bool | RES-944: suffix predicate; empty suffix is always true |
| `bytes_index_of(h, n)` | (bytes, bytes) → int | RES-944: first byte index where `n` appears in `h`, or -1; empty `n` returns 0 |
| `bytes_to_hex(b)` | bytes → string | RES-943: lowercase hex string, no prefix or separator |
| `bytes_from_hex(s)` | string → Result<Bytes, String> | RES-943: parse hex (any case); errors on odd length / non-hex chars — never panics |

## Bitwise (RES-440)

| Name | Signature | Notes |
|---|---|---|
| `bit_and(a, b)` `bit_or(a, b)` `bit_xor(a, b)` | (int, int) → int | bitwise binary ops on i64 |
| `bit_not(a)` | int → int | one's complement |
| `bit_shl(a, n)` `bit_shr(a, n)` | (int, int) → int | shift amount must be 0..=63; arithmetic right shift |
| `is_power_of_two(n)` | int → bool | RES-940: true iff `n > 0` and exactly one bit is set |
| `next_power_of_two(n)` | int → int | RES-940: smallest power of two `>= n`; errors on negative input or overflow (`n > 2^62`) |

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

## Additional Numeric Functions

| Name | Signature | Notes |
|---|---|---|
| `abs_diff(a, b)` | (number, number) → number | RES-485: absolute difference |
| `binomial(n, k)` | (int, int) → int | RES-568: binomial coefficient C(n, k) |
| `factorial(n)` | int → int | RES-567: factorial with overflow detection |
| `fibonacci(n)` | int → int | RES-569: n-th Fibonacci number with overflow detection |
| `is_prime(n)` | int → bool | RES-570: trial-division primality test |
| `next_prime(n)` | int → int | RES-571: smallest prime greater than n |
| `gcd_array(arr)` | array of int → int | RES-536: gcd reduction over an integer array |
| `lcm_array(arr)` | array of int → int | RES-536: lcm reduction over an integer array |
| `as_f32(x)` | number → float | RES-2618: f32 precision cast |
| `as_f64(x)` | number → float | RES-2618: f64 precision cast |
| `as_int8/16/32/64(x)` | int → int | RES-366: wrapping truncation to pinned widths |
| `as_uint8/16/32/64(x)` | int → int | RES-366: wrapping truncation to unsigned widths |
| `isqrt(n)` | int → int | RES-1124: integer square root |
| `ipow(base, exp)` | (int, int) → int | RES-1124: integer exponentiation |
| `ceil_div(a, b)` | (int, int) → int | RES-518: integer division rounding toward +∞ |
| `floor_div(a, b)` | (int, int) → int | RES-518: integer division rounding toward -∞ |
| `div_ceil(a, b)` | (int, int) → int | RES-1126: direction-rounded division (ceiling) |
| `div_floor(a, b)` | (int, int) → int | RES-1126: direction-rounded division (floor) |
| `div_euclid(a, b)` | (int, int) → int | RES-1127: Euclidean division |
| `rem_euclid(a, b)` | (int, int) → int | RES-1127: Euclidean remainder (always non-negative) |
| `modulo(a, b)` | (int, int) → int | RES-519: Python-style modulo (sign of divisor) |
| `divmod(a, b)` | (int, int) → (int, int) | RES-486: quotient and remainder tuple |
| `midpoint(a, b)` | (int, int) → int | RES-1128: overflow-safe arithmetic mean |
| `ilog2(n)` | int → int | RES-1129: integer log base 2 (no f64 round-trip) |
| `ilog10(n)` | int → int | RES-1129: integer log base 10 (no f64 round-trip) |
| `int_log(n, base)` | (int, int) → int | Integer logarithm with explicit base |
| `int_sqrt(n)` | int → int | RES-491: integer square root |
| `int_log2(n)` | int → int | RES-492: floor log₂ |
| `pow_int(base, exp)` | (int, int) → int | RES-517: integer exponentiation |
| `int_pow(base, exp)` | (int, int) → int | Alias for integer power |
| `saturating_add(a, b)` | (int, int) → int | RES-1115: addition with saturation on overflow |
| `saturating_sub(a, b)` | (int, int) → int | RES-1115: subtraction with saturation on overflow |
| `saturating_mul(a, b)` | (int, int) → int | RES-1115: multiplication with saturation on overflow |
| `wrapping_add(a, b)` | (int, int) → int | RES-1115: addition with wrapping on overflow |
| `wrapping_sub(a, b)` | (int, int) → int | RES-1115: subtraction with wrapping on overflow |
| `wrapping_mul(a, b)` | (int, int) → int | RES-1115: multiplication with wrapping on overflow |
| `checked_add(a, b)` | (int, int) → Result<Int, String> | RES-1115: addition with overflow check |
| `checked_sub(a, b)` | (int, int) → Result<Int, String> | RES-1115: subtraction with overflow check |
| `checked_mul(a, b)` | (int, int) → Result<Int, String> | RES-1115: multiplication with overflow check |
| `checked_div(a, b)` | (int, int) → Result<Int, String> | RES-1115: division with overflow/zero check |
| `next_multiple_of(n, m)` | (int, int) → int | RES-1136: next multiple of m >= n |
| `is_multiple_of(n, m)` | (int, int) → bool | RES-1136: test if n is a multiple of m |
| `lerp(a, b, t)` | (float, float, float) → float | RES-2650: linear interpolation |
| `remap(x, in_lo, in_hi, out_lo, out_hi)` | (float, float, float, float, float) → float | RES-2650: remap value from one range to another |
| `float_approx_eq(a, b, epsilon)` | (float, float, float) → bool | RES-2650: approximate equality check |
| `round_to(x, decimals)` | (float, int) → float | RES-2650: round to specified decimal places |

## Advanced Float Math

| Name | Signature | Notes |
|---|---|---|
| `exp2(x)` | float → float | RES-891: 2^x; std-only |
| `expm1(x)` | float → float | RES-1168: e^x - 1 with precision for small x |
| `ln_1p(x)` | float → float | RES-1168: ln(1 + x) with precision for small x |
| `mul_add(x, y, z)` | (float, float, float) → float | RES-1168: fused multiply-add x*y + z |
| `recip(x)` | float → float | RES-1168: reciprocal 1/x |
| `float_to_bits(x)` | float → int | RES-1130: IEEE 754 bit reinterpret cast |
| `float_from_bits(bits)` | int → float | RES-1130: IEEE 754 bit reinterpret cast (reverse) |
| `float_classify(x)` | float → string | RES-1138: classify as NaN/Infinite/Normal/Subnormal/Zero |
| `float_total_cmp(a, b)` | (float, float) → int | RES-1138: total order comparison (-1/0/+1) |
| `float_is_normal(x)` | float → bool | RES-1138: IEEE 754 normal number predicate |
| `float_is_subnormal(x)` | float → bool | RES-1138: IEEE 754 subnormal number predicate |
| `float_sign_bit(x)` | float → bool | RES-1138: check sign bit (true if negative/negative zero) |
| `round(x)` | float → float | RES-1166: round to nearest integer (banker's rounding) |
| `trunc(x)` | float → float | RES-1166: truncate toward zero |
| `round_to_int(x)` | float → int | RES-1166: round to nearest integer as int |
| `trunc_to_int(x)` | float → int | RES-1166: truncate toward zero as int |

## Bit Manipulation (Extended)

| Name | Signature | Notes |
|---|---|---|
| `rotate_left_int(n, shift)` | (int, int) → int | RES-1119: rotate left |
| `rotate_right_int(n, shift)` | (int, int) → int | RES-1119: rotate right |
| `reverse_bits(n)` | int → int | RES-1119: reverse all 64 bits |
| `swap_bytes(n)` | int → int | RES-1119: swap byte order (endianness) |
| `to_be_bytes(n)` | int → bytes | RES-1122: convert to big-endian byte array |
| `to_le_bytes(n)` | int → bytes | RES-1122: convert to little-endian byte array |
| `from_be_bytes(b)` | bytes → int | RES-1122: parse from big-endian bytes |
| `from_le_bytes(b)` | bytes → int | RES-1122: parse from little-endian bytes |
| `rotate_left(n, shift)` | (int, int) → int | RES-1182: scalar bit rotation left |
| `rotate_right(n, shift)` | (int, int) → int | RES-1182: scalar bit rotation right |
| `signum(n)` | int → int | RES-1182: -1 / 0 / +1 for negative/zero/positive |
| `bit_set(n, i)` | (int, int) → int | RES-1156: set bit i |
| `bit_clear(n, i)` | (int, int) → int | RES-1156: clear bit i |
| `bit_get(n, i)` | (int, int) → int | RES-1156: get bit i (0 or 1) |
| `bit_flip(n, i)` | (int, int) → int | RES-1156: toggle bit i |
| `get_bit(n, i)` | (int, int) → bool | RES-1156: bit i as boolean |
| `set_bit(n, i)` | (int, int) → int | RES-1156: set bit i to 1 |
| `clear_bit(n, i)` | (int, int) → int | RES-1156: set bit i to 0 |
| `flip_bit(n, i)` | (int, int) → int | RES-1156: toggle bit i |

## String Functions (Extended)

| Name | Signature | Notes |
|---|---|---|
| `string_split(s, sep)` | (string, string) → array of string | RES-1859: explicit-name alias for `split` |
| `string_split_n(s, sep, n)` | (string, string, int) → array of string | RES-535: split with maximum splits limit |
| `string_split_last(s, sep)` | (string, string) → array of string | RES-545: split on last occurrence of separator |
| `string_replace_first(s, from, to)` | (string, string, string) → string | RES-480: replace only first occurrence |
| `string_replace_n(s, from, to, n)` | (string, string, string, int) → string | RES-482: replace up to n occurrences |
| `string_find(s, sub)` | (string, string) → int | RES-546: first byte index of substring, -1 if missing |
| `string_rfind(s, sub)` | (string, string) → int | RES-547: last byte index of substring, -1 if missing |
| `string_split_at(s, i)` | (string, int) → (string, string) | RES-548: split at byte index into [before, after] |
| `string_take_while_char(s, pred)` | (string, string) → string | RES-525: take while named predicate matches |
| `string_drop_while_char(s, pred)` | (string, string) → string | RES-525: drop while named predicate matches |
| `string_filter_char(s, pred)` | (string, string) → string | RES-526: named-predicate global char filter |
| `string_eq_ignore_case(a, b)` | (string, string) → bool | RES-527: ASCII case-insensitive equality |
| `string_find_char(s, c)` | (string, string) → int | RES-524: char-index of single character (-1 if absent) |
| `string_count_char(s, c)` | (string, string) → int | RES-523: count occurrences of single character |
| `string_words(s)` | string → array of string | RES-496: split on Unicode whitespace |
| `string_join_lines(arr)` | array of string → string | RES-497: join with newline separator |
| `string_unwords(arr)` | array of string → string | RES-498: join with single-space separator |
| `string_take(s, n)` | (string, int) → string | RES-499: take first n Unicode scalars |
| `string_drop(s, n)` | (string, int) → string | RES-506: drop first n Unicode scalars |
| `string_truncate(s, len)` | (string, int) → string | RES-1164: truncate string to max length |
| `string_pad_center(s, n, c)` | (string, int, string) → string | RES-540: center-pad to Unicode-scalar width |
| `string_strip_prefix(s, prefix)` | (string, string) → string | RES-471: conditional prefix removal |
| `string_strip_suffix(s, suffix)` | (string, string) → string | RES-471: conditional suffix removal |
| `string_split_once(s, sep)` | (string, string) → Result<(String, String), String> | RES-1172: split on first occurrence |
| `string_rsplit_once(s, sep)` | (string, string) → Result<(String, String), String> | RES-1172: split on last occurrence |
| `string_from_chars(arr)` | array of string → string | RES-1172: join char array into string |
| `string_to_bytes(s)` | string → bytes | RES-565: string to array of UTF-8 bytes |
| `string_from_bytes(b)` | bytes → Result<String, String> | RES-566: array of bytes to string (UTF-8 validated) |
| `string_byte_at(s, i)` | (string, int) → int | RES-564: byte at index (-1 if out of range) |
| `char_to_digit(c)` | string → Result<Int, String> | RES-505: parse single char to base-36 digit |
| `digit_to_char(d)` | int → string | RES-513: int 0..=35 to base-36 digit char |
| `char_to_int(c)` | string → int | Unicode codepoint of single character |
| `int_to_char(n)` | int → string | Convert unicode codepoint to single character |
| `char_is_alpha(c)` | string → bool | RES-2619: alphabetic predicate |
| `char_is_ascii(c)` | string → bool | RES-2619: ASCII predicate |
| `char_is_digit(c)` | string → bool | RES-2619: numeric digit predicate |
| `char_is_lower(c)` | string → bool | RES-2619: lowercase letter predicate |
| `char_is_upper(c)` | string → bool | RES-2619: uppercase letter predicate |
| `char_to_lower(c)` | string → string | RES-2619: convert char to lowercase |
| `char_to_upper(c)` | string → string | RES-2619: convert char to uppercase |
| `is_ascii(s)` | string → bool | RES-1140: check if all chars are ASCII |
| `is_ascii_whitespace(s)` | string → bool | RES-1140: ASCII whitespace predicate |
| `is_ascii_hexdigit(s)` | string → bool | RES-1140: ASCII hex digit predicate |
| `is_ascii_uppercase(s)` | string → bool | RES-1140: ASCII uppercase predicate |
| `is_ascii_lowercase(s)` | string → bool | RES-1140: ASCII lowercase predicate |
| `is_ascii_punctuation(s)` | string → bool | RES-1140: ASCII punctuation predicate |
| `is_ascii_control(s)` | string → bool | RES-1140: ASCII control character predicate |
| `trim_start_chars(s, chars)` | (string, string) → string | RES-477: left-trim arbitrary char set |
| `trim_end_chars(s, chars)` | (string, string) → string | RES-477: right-trim arbitrary char set |
| `intern(s)` | string → string | RES-2612: intern for runtime deduplication |

## Array Functions (Extended)

| Name | Signature | Notes |
|---|---|---|
| `array_concat3(a, b, c)` | (array, array, array) → array | RES-515: three-way concatenation |
| `array_take_last(arr, n)` | (array, int) → array | RES-537: take trailing n elements |
| `array_drop_last(arr, n)` | (array, int) → array | RES-537: drop trailing n elements |
| `array_rest(arr)` | array → array | RES-481: drop first element; empty stays empty |
| `array_init(arr)` | array → array | RES-481: drop last element; empty stays empty |
| `array_slice(arr, lo, hi)` | (array, int, int) → array | RES-921: half-open sub-array slice |
| `array_step(arr, n)` | (array, int) → array | RES-514: pick every nth element |
| `array_indices(arr)` | array → array of int | RES-522: generate [0, 1, ..., len-1] |
| `array_get_or(arr, i, default)` | (array, int, T) → T | RES-528: bounded indexing with fallback |
| `array_count_eq(arr, x)` | (array, T) → int | RES-478: explicit-name alias for array_count |
| `array_max_or(arr, default)` | (array, T) → T | RES-543: max with fallback for empty |
| `array_min_or(arr, default)` | (array, T) → T | RES-543: min with fallback for empty |
| `array_first_or(arr, default)` | (array, T) → T | RES-1158: first element or default |
| `array_last_or(arr, default)` | (array, T) → T | RES-1158: last element or default |
| `array_index_of_last(arr, x)` | (array, T) → int | RES-1158: last matching index, or -1 |

### Array Integer-specific Functions

| Name | Signature | Notes |
|---|---|---|
| `array_all_int(arr, pred)` | (array of int, string) → bool | RES-501: all elements match named predicate |
| `array_any_int(arr, pred)` | (array of int, string) → bool | RES-500: any element matches named predicate |
| `array_count_int(arr, pred)` | (array of int, string) → int | RES-530: count elements matching predicate |
| `array_filter_int(arr, pred)` | (array of int, string) → array | RES-484: keep only elements matching predicate |
| `array_partition_int(arr, pred)` | (array of int, string) → (array, array) | RES-484: partition into matching/non-matching |
| `array_take_while_int(arr, pred)` | (array of int, string) → array | RES-483: take while predicate matches |
| `array_drop_while_int(arr, pred)` | (array of int, string) → array | RES-483: drop while predicate matches |
| `array_mean_int(arr)` | array of int → float | RES-549: mean (truncating toward zero) |
| `array_median_int(arr)` | array of int → float | RES-550: median (mean of middles for even) |
| `array_mode_int(arr)` | array of int → int | RES-551: most common element (smallest on ties) |
| `array_range_int(arr)` | array of int → int | RES-552: peak-to-peak (max - min) |
| `array_diff_consec_int(arr)` | array of int → array of int | RES-553: consecutive pairwise differences |
| `array_clamp_int(arr, lo, hi)` | (array of int, int, int) → array of int | RES-554: per-element clamp |
| `array_signum_int(arr)` | array of int → array of int | RES-555: per-element sign (-1/0/+1) |
| `array_abs_int(arr)` | array of int → array of int | RES-556: per-element absolute value |
| `array_dot_int(a, b)` | (array of int, array of int) → int | RES-557: dot product |
| `array_sum_squares_int(arr)` | array of int → int | RES-558: sum of squares (Σ x²) |
| `array_cumsum_int(arr)` | array of int → array of int | RES-559: running prefix sum |
| `array_cummax_int(arr)` | array of int → array of int | RES-560: running maximum |
| `array_cummin_int(arr)` | array of int → array of int | RES-561: running minimum |
| `array_cumprod_int(arr)` | array of int → array of int | RES-562: running product |
| `array_count_in_range_int(arr, lo, hi)` | (array of int, int, int) → int | RES-563: count in [lo, hi] |
| `array_group_by_int(arr)` | array of int → array of array of int | RES-504: group into maximal runs of equal |
| `array_count_runs(arr)` | array of int → int | RES-533: count maximal runs |
| `array_scan_int(arr, init, op)` | (array of int, int, string) → array of int | RES-502: running fold (intermediate results) |
| `array_zip_with_int(a, b, op)` | (array of int, array of int, string) → array of int | RES-521: element-wise binary op |
| `array_argmax_int(arr)` | array of int → int | RES-503: index of max (first on ties) |
| `array_argmin_int(arr)` | array of int → int | RES-503: index of min (first on ties) |
| `array_indices_where(arr, pred)` | (array of int, string) → array of int | RES-539: indices of elements matching predicate |
| `array_fold_int(arr, init, op)` | (array of int, int, string) → int | RES-475: fold with named op (sum/product/min/max) |

### Array Set-like Operations

| Name | Signature | Notes |
|---|---|---|
| `array_intersect(a, b)` | (array, array) → array | RES-541: set intersection |
| `array_diff(a, b)` | (array, array) → array | RES-541: set difference (elements in a not in b) |
| `array_union(a, b)` | (array, array) → array | RES-542: order-preserving dedup union |
| `array_index_of_all(arr, x)` | (array, T) → array of int | RES-544: every index where element equals x |
| `array_interleave(a, b)` | (array, array) → array | RES-516: alternate elements from a and b |
| `array_difference(a, b)` | (array, array) → array | RES-1158: set difference |
| `array_intersection(a, b)` | (array, array) → array | RES-1158: set intersection |

### Array Sorting and Searching

| Name | Signature | Notes |
|---|---|---|
| `array_sort_desc(arr)` | array of int → array of int | RES-443: descending sort |
| `array_sort_float(arr)` | array of float → array of float | RES-1146: float array sort |
| `array_sort_string(arr)` | array of string → array of string | RES-1146: string array sort |
| `array_is_sorted(arr)` | array of int → bool | RES-1146: check if sorted ascending |
| `array_is_sorted_float(arr)` | array of float → bool | RES-1146: check if float array sorted |
| `array_is_sorted_string(arr)` | array of string → bool | RES-1146: check if string array sorted |
| `array_binary_search(arr, x)` | (array of int, int) → Result<Int, Int> | RES-1148: binary search on sorted int array |
| `array_binary_search_float(arr, x)` | (array of float, float) → Result<Int, Int> | RES-1148: binary search on sorted float array |
| `array_binary_search_string(arr, x)` | (array of string, string) → Result<Int, Int> | RES-1148: binary search on sorted string array |
| `array_argmax_float(arr)` | array of float → int | RES-1160: index of max float |
| `array_argmin_float(arr)` | array of float → int | RES-1160: index of min float |
| `array_argmax_string(arr)` | array of string → int | RES-1160: index of max string |
| `array_argmin_string(arr)` | array of string → int | RES-1160: index of min string |

### Array Chunking and Windowing

| Name | Signature | Notes |
|---|---|---|
| `array_chunks(arr, n)` | (array, int) → array of array | RES-1142: fixed-size chunks (last may be short) |
| `array_chunks_exact(arr, n)` | (array, int) → array of array | RES-1142: fixed-size chunks (errors if not divisible) |
| `array_windows(arr, n)` | (array, int) → array of array | RES-2648: sliding windows of size n |
| `array_zip3(a, b, c)` | (array, array, array) → array of tuple | RES-1164: zip three arrays |
| `array_unzip(arr)` | array of (T, U) → (array of T, array of U) | RES-531: unzip array of 2-tuples |
| `array_cumsum(arr)` | array of float → array of float | RES-1170: cumulative sum (float) |
| `array_cumprod(arr)` | array of float → array of float | RES-1170: cumulative product (float) |
| `array_diffs(arr)` | array of float → array of float | RES-1170: pairwise differences (float) |
| `array_min_max(arr)` | array → (T, T) | RES-1170: combined min/max tuple |
| `array_is_empty(arr)` | array → bool | RES-1172: check if empty |
| `array_frequency_map(arr)` | array of T → map of (T → Int) | RES-2650: count element frequencies |

### Array Statistics (Float)

| Name | Signature | Notes |
|---|---|---|
| `array_sum_float(arr)` | array of float → float | RES-942: float array sum |
| `array_product_float(arr)` | array of float → float | RES-942: float array product |
| `array_min_float(arr)` | array of float → float | RES-942: float array min (NaN propagates) |
| `array_max_float(arr)` | array of float → float | RES-942: float array max (NaN propagates) |
| `array_average_float(arr)` | array of float → float | RES-942: float array mean |
| `array_median_float(arr)` | array of float → float | RES-1150: median of floats |
| `array_range_float(arr)` | array of float → float | RES-1150: peak-to-peak of floats |
| `array_variance_int(arr)` | array of int → float | RES-1150: integer array variance |
| `array_variance_float(arr)` | array of float → float | RES-1150: float array variance |
| `array_stddev_int(arr)` | array of int → float | RES-1150: integer array standard deviation |
| `array_stddev_float(arr)` | array of float → float | RES-1150: float array standard deviation |

## Bytes Functions (Extended)

| Name | Signature | Notes |
|---|---|---|
| `bytes_count_byte(b, byte)` | (bytes, int) → int | RES-1152: count occurrences of byte |
| `bytes_replace_byte(b, from, to)` | (bytes, int, int) → bytes | RES-1152: replace all occurrences of byte |
| `bytes_repeat(b, n)` | (bytes, int) → bytes | RES-1152: repeat bytes n times |
| `bytes_xor(a, b)` | (bytes, bytes) → bytes | RES-1134: bitwise XOR |
| `bytes_and(a, b)` | (bytes, bytes) → bytes | RES-1134: bitwise AND |
| `bytes_or(a, b)` | (bytes, bytes) → bytes | RES-1134: bitwise OR |
| `bytes_not(b)` | bytes → bytes | RES-1134: bitwise NOT |
| `bytes_fill(len, byte)` | (int, int) → bytes | RES-1134: create bytes filled with byte |
| `bytes_reverse(b)` | bytes → bytes | RES-1134: reverse byte order |
| `bytes_take(b, n)` | (bytes, int) → bytes | RES-1178: take first n bytes |
| `bytes_drop(b, n)` | (bytes, int) → bytes | RES-1178: drop first n bytes |
| `bytes_take_last(b, n)` | (bytes, int) → bytes | RES-1178: take last n bytes |
| `bytes_drop_last(b, n)` | (bytes, int) → bytes | RES-1178: drop last n bytes |
| `bytes_strip_prefix(b, prefix)` | (bytes, bytes) → bytes | RES-1176: conditional prefix removal |
| `bytes_strip_suffix(b, suffix)` | (bytes, bytes) → bytes | RES-1176: conditional suffix removal |
| `bytes_to_string(b)` | bytes → Result<String, String> | RES-1176: convert to string (UTF-8 validated) |

## Map Functions (Extended)

| Name | Signature | Notes |
|---|---|---|
| `map_entries(m)` | map → array of (K, V) | RES-1144: all key-value pairs |
| `map_merge(a, b)` | (map, map) → map | RES-1144: merge two maps (b overwrites a) |
| `map_is_empty(m)` | map → bool | RES-1144: check if empty |
| `map_from_pairs(arr)` | array of (K, V) → map | RES-2646: construct map from key-value pairs |
| `map_to_pairs(m)` | map → array of (K, V) | RES-2647: convert to array of pairs |
| `map_invert(m)` | map → map | RES-2647: swap keys and values |
| `hashmap_entries(m)` | hashmap → array of (K, V) | RES-1144: hashmap pairs |
| `hashmap_merge(a, b)` | (hashmap, hashmap) → hashmap | RES-1144: merge hashmaps |
| `hashmap_is_empty(m)` | hashmap → bool | RES-1144: check if hashmap empty |

## Set Functions (Extended)

| Name | Signature | Notes |
|---|---|---|
| `set_is_empty(s)` | set → bool | RES-1154: check if empty |
| `set_from_array(arr)` | array of T → set | RES-1154: construct from array |

## Result/Option Functions (Extended)

| Name | Signature | Notes |
|---|---|---|
| `result_and(a, b)` | (Result, Result) → Result | RES-1154: combine results (first Err wins) |
| `option_and(a, b)` | (Option, Option) → Option | RES-1154: combine options (first None wins) |
| `option_ok_or(o, e)` | (Option<T>, E) → Result<T, E> | RES-2651: convert Some → Ok, None → Err |
| `option_or(a, b)` | (Option<T>, Option<T>) → Option<T> | RES-939: chain alternatives |
| `result_or(a, b)` | (Result<T, E>, Result<T, E>) → Result<T, E> | RES-939: chain alternatives |
| `result_collect(arr)` | array of Result<T, E> → Result<Array, E> | RES-2652: collect results (first Err short-circuits) |

## Type Introspection

| Name | Signature | Notes |
|---|---|---|
| `type_of(x)` | T → string | RES-2652: get runtime type name |
| `struct_name(x)` | struct → string | RES-2652: get struct type name |
| `is_tuple(x)` | T → bool | RES-932: distinguish tuple from array |
| `identity(x)` | T → T | RES-2656: identity function (pure) |

## Cryptographic Hash Functions

| Name | Signature | Notes |
|---|---|---|
| `sha256(b)` | bytes → bytes | RES-2560: SHA-256 hash |
| `sha256_str(s)` | string → bytes | RES-2560: SHA-256 of string |
| `sha512(b)` | bytes → bytes | RES-2560: SHA-512 hash |
| `sha512_str(s)` | string → bytes | RES-2560: SHA-512 of string |
| `crc32(b)` | bytes → int | RES-2561: CRC-32 checksum |
| `crc32_str(s)` | string → int | RES-2561: CRC-32 of string |
| `crc16(b)` | bytes → int | RES-2561: CRC-16 checksum |
| `crc16_str(s)` | string → int | RES-2561: CRC-16 of string |
| `hash_int(n)` | int → int | RES-1162: deterministic hash of integer |
| `hash_string(s)` | string → int | RES-1162: deterministic hash of string |
| `hash_bytes(b)` | bytes → int | RES-1162: deterministic hash of bytes |
| `hash_combine(h1, h2)` | (int, int) → int | RES-1162: combine two hashes |

## JSON Functions

| Name | Signature | Notes |
|---|---|---|
| `to_json(x)` | T → string | RES-2657: serialize to JSON |
| `from_json(s)` | string → T | RES-2657: deserialize from JSON |
| `json_encode(x)` | T → string | RES-2554: alias for `to_json` |
| `json_decode(s)` | string → T | RES-2554: alias for `from_json` |
| `json_encode_pretty(x)` | T → string | RES-2554: pretty-print JSON |
| `json_valid(s)` | string → bool | RES-2554: validate JSON syntax |

## Regular Expressions

| Name | Signature | Notes |
|---|---|---|
| `regex_match(s, pattern)` | (string, string) → bool | RES-2585: check if pattern matches |
| `regex_find(s, pattern)` | (string, string) → Result<String, String> | RES-2585: find first match |
| `regex_find_all(s, pattern)` | (string, string) → array of string | RES-2585: find all non-overlapping matches |
| `regex_captures(s, pattern)` | (string, string) → Result<Array, String> | RES-2585: extract capture groups |
| `regex_replace(s, pattern, repl)` | (string, string, string) → string | RES-2585: replace first match |
| `regex_replace_all(s, pattern, repl)` | (string, string, string) → string | RES-2585: replace all matches |

## Date/Time Functions

| Name | Signature | Notes |
|---|---|---|
| `datetime_now()` | () → datetime | RES-2559: current date/time |
| `datetime_from_unix(timestamp)` | int → datetime | RES-2559: convert Unix timestamp to datetime |
| `datetime_to_unix(dt)` | datetime → int | RES-2559: convert datetime to Unix timestamp |
| `datetime_format(dt, fmt)` | (datetime, string) → string | RES-2559: format datetime |
| `datetime_parse(s, fmt)` | (string, string) → datetime | RES-2559: parse datetime |
| `unix_time_s()` | () → int | RES-1174: current Unix time in seconds |
| `unix_time_ms()` | () → int | RES-1174: current Unix time in milliseconds |
| `unix_time_ns()` | () → int | RES-1174: current Unix time in nanoseconds |
| `tick_now()` | () → int | Event journal: get current tick |
| `tick_advance()` | () → void | Event journal: advance tick |
| `record_event(data)` | string → void | Event journal: record event |
| `replay_events()` | () → array | Event journal: replay recorded events |
| `clear_events()` | () → void | Event journal: clear recorded events |
| `event_count()` | () → int | Event journal: count recorded events |

## HTTP Client Functions

| Name | Signature | Notes |
|---|---|---|
| `http_get(url)` | string → Result<String, String> | RES-2556: HTTP GET request |
| `http_post(url, body)` | (string, string) → Result<String, String> | RES-2556: HTTP POST request |

## Linear Algebra

| Name | Signature | Notes |
|---|---|---|
| `vec_add(a, b)` | (array, array) → array | RES-2658: vector addition |
| `vec_sub(a, b)` | (array, array) → array | RES-2658: vector subtraction |
| `vec_scale(v, s)` | (array, number) → array | RES-2658: scalar multiplication |
| `vec_dot(a, b)` | (array, array) → number | RES-2658: dot product |
| `vec_norm(v)` | array → float | RES-2658: Euclidean norm |
| `vec_normalize(v)` | array → array | RES-2658: unit vector |
| `vec_cross(a, b)` | (array, array) → array | RES-2658: cross product (3D) |
| `vec_lerp(a, b, t)` | (array, array, float) → array | RES-2658: linear interpolation |
| `mat_mul(a, b)` | (array of array, array of array) → array of array | RES-2658: matrix multiplication |
| `mat_add(a, b)` | (array of array, array of array) → array of array | RES-2658: matrix addition |
| `mat_scale(m, s)` | (array of array, number) → array of array | RES-2658: matrix scaling |
| `mat_transpose(m)` | array of array → array of array | RES-2658: matrix transpose |
| `mat_identity(n)` | int → array of array | RES-2658: n×n identity matrix |
| `mat_trace(m)` | array of array → number | RES-2658: sum of diagonal |

## Graph Algorithms

| Name | Signature | Notes |
|---|---|---|
| `graph_bfs(g, start)` | (array of array, int) → array of int | RES-2659: breadth-first search |
| `graph_dfs(g, start)` | (array of array, int) → array of int | RES-2659: depth-first search |
| `graph_has_path(g, from, to)` | (array of array, int, int) → bool | RES-2659: check if path exists |
| `graph_topological_sort(g)` | array of array → array of int | RES-2659: topological sort (DAG only) |
| `graph_connected_components(g)` | array of array → array of array of int | RES-2659: find connected components |
| `graph_num_components(g)` | array of array → int | RES-2659: count connected components |
| `graph_out_degrees(g)` | array of array → array of int | RES-2659: out-degree for each vertex |
| `graph_in_degrees(g)` | array of array → array of int | RES-2659: in-degree for each vertex |
| `graph_reverse(g)` | array of array → array of array | RES-2659: reverse all edges |
| `graph_is_dag(g)` | array of array → bool | RES-2659: test for directed acyclic graph |
| `graph_reachable(g, start)` | (array of array, int) → array of bool | RES-2659: reachability from start |
| `graph_dijkstra(g, start)` | (array of array, int) → array of int | RES-2659: shortest paths (weighted) |

## Combinatorics

| Name | Signature | Notes |
|---|---|---|
| `array_cartesian_product(a, b)` | (array, array) → array of array | RES-2654: Cartesian product |
| `array_cartesian_product_n(arrays)` | array of array → array of array | RES-2654: n-ary Cartesian product |
| `array_combinations(arr, k)` | (array, int) → array of array | RES-2654: k-combinations |
| `array_permutations(arr)` | array → array of array | RES-2654: all permutations |
| `array_powerset(arr)` | array → array of array | RES-2654: all subsets |
| `array_transpose(m)` | array of array → array of array | RES-2654: transpose 2D array |

## Statistics

| Name | Signature | Notes |
|---|---|---|
| `stats_covariance(a, b)` | (array, array) → float | RES-2660: covariance of two sequences |
| `stats_correlation(a, b)` | (array, array) → float | RES-2660: Pearson correlation |
| `stats_percentile(arr, p)` | (array of number, float) → number | RES-2660: p-th percentile (0.0-1.0) |

## Number Theory

| Name | Signature | Notes |
|---|---|---|
| `prime_factors(n)` | int → array of int | RES-2655: prime factorization |
| `primes_up_to(n)` | int → array of int | RES-2655: all primes <= n |
| `euler_totient(n)` | int → int | RES-2655: Euler's totient function |
| `divisors(n)` | int → array of int | RES-2655: all divisors of n |
| `is_perfect(n)` | int → bool | RES-2655: test if perfect number |
| `digit_sum(n)` | int → int | RES-2655: sum of digits |
| `digital_root(n)` | int → int | RES-2655: iterative digit sum until single digit |
| `collatz_length(n)` | int → int | RES-2655: length of Collatz sequence |
| `is_fibonacci(n)` | int → bool | RES-2655: test if Fibonacci number |
| `count_digits(n)` | int → int | RES-2655: number of digits |
| `int_to_digits(n)` | int → array of int | RES-2655: array of digits |
| `int_from_digits(arr)` | array of int → int | RES-2655: construct int from digits |

## Runtime Provenance and Events

| Name | Signature | Notes |
|---|---|---|
| `tag(value, source)` | (T, string) → Tagged<T> | Grand Pass 2B: wrap with provenance |
| `untag(t, expected)` | (Tagged<T>, string) → Result<T, String> | Grand Pass 2B: extract with validation |
| `tag_of(t)` | Tagged<T> → Result<String, String> | Grand Pass 2B: read source non-destructively |
| `snapshot_save(name, data)` | (string, T) → void | Grand Pass 2C: save named checkpoint |
| `snapshot_load(name)` | string → Result<T, String> | Grand Pass 2C: load checkpoint |
| `snapshot_keys()` | () → array of string | Grand Pass 2C: list all checkpoint names |
| `snapshot_clear(name)` | string → void | Grand Pass 2C: delete checkpoint |
| `quota_set(name, limit)` | (string, int) → void | Grand Pass 3D: set resource quota |
| `quota_charge(name, amount)` | (string, int) → void | Grand Pass 3D: charge against quota |
| `quota_remaining(name)` | string → int | Grand Pass 3D: remaining quota |
| `quota_reset(name)` | string → void | Grand Pass 3D: reset quota to limit |
| `quota_used(name)` | string → int | Grand Pass 3D: total used |
| `quotas()` | () → map | Grand Pass 3D: all quotas and usage |
| `mint_cap(name, value)` | (string, T) → Capability | Grand Pass 3E: create capability token |
| `check_cap(cap, expected)` | (Capability, string) → bool | Grand Pass 3E: verify capability |
| `revoke_cap(cap)` | Capability → void | Grand Pass 3E: revoke token |
| `caps()` | () → array | Grand Pass 3E: active capabilities |

## Concurrency and Actors

| Name | Signature | Notes |
|---|---|---|
| `spawn(fn)` | fn() → int | RES-332: spawn actor (returns PID) |
| `send(pid, msg)` | (int, T) → Result<Void, String> | RES-332: send message to actor |
| `receive()` | () → T | RES-332: receive message (blocks) |

## Shared Mutable State

| Name | Signature | Notes |
|---|---|---|
| `cell(value)` | T → Cell<T> | RES-328: create shared mutable cell |
| `StringBuilder_new()` | () → StringBuilder | RES-353: create string builder |

## Volatile Memory (MMIO)

| Name | Signature | Notes |
|---|---|---|
| `volatile_read_u8(addr)` | int → int | RES-406: volatile memory read (u8) |
| `volatile_read_u16(addr)` | int → int | RES-406: volatile memory read (u16) |
| `volatile_read_u32(addr)` | int → int | RES-406: volatile memory read (u32) |
| `volatile_read_u64(addr)` | int → int | RES-406: volatile memory read (u64) |
| `volatile_write_u8(addr, val)` | (int, int) → void | RES-406: volatile memory write (u8) |
| `volatile_write_u16(addr, val)` | (int, int) → void | RES-406: volatile memory write (u16) |
| `volatile_write_u32(addr, val)` | (int, int) → void | RES-406: volatile memory write (u32) |
| `volatile_write_u64(addr, val)` | (int, int) → void | RES-406: volatile memory write (u64) |

## Other Compiler and Runtime Builtins

| Name | Signature | Notes |
|---|---|---|
| `version()` | () → string | RES-1100: compiler version string |
| `include_str(path)` | string → string | RES-2610: compile-time file embedding (string) |
| `include_bytes(path)` | string → bytes | RES-2610: compile-time file embedding (bytes) |

---

When adding a new builtin, the canonical list to update is:

1. The `BUILTINS` table in `resilient/src/lib.rs`.
2. The type signature in the prelude block of `resilient/src/typechecker.rs`.
3. The `PURE_BUILTINS` list in `resilient/src/typechecker.rs` (unless impure).
4. A row in this file and in `SYNTAX.md`.
5. A focused Rust test in `resilient/src/lib.rs` or `resilient/tests/`.
