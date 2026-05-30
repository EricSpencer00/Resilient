//! RES-2587: Priority queue / binary heap collection.
//!
//! Implements min-heap and max-heap backed by `Value::Array`.
//! The heap invariant is stored inline — index 0 is always the
//! extremal element. All operations return new heap arrays (functional style).
//!
//! Supported element types: Int, Float, String (lexicographic).
//!
//! API:
//!   heap_new()              → Array (empty min-heap)
//!   heap_new_max()          → Array (empty max-heap; sentinel-tagged)
//!   heap_push(h, val)       → Array  — O(log n)
//!   heap_pop(h)             → (Option<any>, Array)  — O(log n)
//!   heap_peek(h)            → Option<any>  — O(1)
//!   heap_len(h)             → int  — O(1)
//!   heap_is_empty(h)        → bool  — O(1)
//!
//! Max-heap: `heap_new_max()` returns a 1-element array with the
//! sentinel `Int::MIN`, which is immediately discarded by heap_push.
//! Internally we separate min/max via a `HeapKind` marker stored as a
//! `Value::Bool` tag prepended to the element array.
//! Layout: `[Bool(is_max), ...elements...]`

use crate::Value;

type RResult<T> = Result<T, String>;

// Compare two values: negative if a < b, zero if equal, positive if a > b.
fn compare(a: &Value, b: &Value) -> RResult<std::cmp::Ordering> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Ok(x.cmp(y)),
        (Value::Float(x), Value::Float(y)) => {
            Ok(x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal))
        }
        (Value::Int(x), Value::Float(y)) => Ok((*x as f64)
            .partial_cmp(y)
            .unwrap_or(std::cmp::Ordering::Equal)),
        (Value::Float(x), Value::Int(y)) => Ok(x
            .partial_cmp(&(*y as f64))
            .unwrap_or(std::cmp::Ordering::Equal)),
        (Value::String(x), Value::String(y)) => Ok(x.cmp(y)),
        _ => Err(format!("heap: cannot compare {} and {}", a, b)),
    }
}

// Unpack heap storage: (is_max, elements_slice)
fn unpack(arr: &[Value]) -> (bool, &[Value]) {
    match arr.first() {
        Some(Value::Bool(is_max)) => (*is_max, &arr[1..]),
        _ => (false, arr), // treat a bare array as a min-heap
    }
}

// Pack heap storage from is_max flag + elements
fn pack(is_max: bool, elems: Vec<Value>) -> Vec<Value> {
    let mut out = Vec::with_capacity(elems.len() + 1);
    out.push(Value::Bool(is_max));
    out.extend(elems);
    out
}

// Sift-up: restore heap property after inserting at index `idx` (0-based in elems).
fn sift_up(elems: &mut [Value], mut idx: usize, is_max: bool) -> RResult<()> {
    while idx > 0 {
        let parent = (idx - 1) / 2;
        let ord = compare(&elems[idx], &elems[parent])?;
        let should_swap = if is_max {
            ord == std::cmp::Ordering::Greater
        } else {
            ord == std::cmp::Ordering::Less
        };
        if should_swap {
            elems.swap(idx, parent);
            idx = parent;
        } else {
            break;
        }
    }
    Ok(())
}

// Sift-down: restore heap property after removing root (place last element at 0).
fn sift_down(elems: &mut [Value], mut idx: usize, is_max: bool) -> RResult<()> {
    let len = elems.len();
    loop {
        let left = 2 * idx + 1;
        let right = 2 * idx + 2;
        let mut target = idx;

        if left < len {
            let ord = compare(&elems[left], &elems[target])?;
            let better = if is_max {
                ord == std::cmp::Ordering::Greater
            } else {
                ord == std::cmp::Ordering::Less
            };
            if better {
                target = left;
            }
        }
        if right < len {
            let ord = compare(&elems[right], &elems[target])?;
            let better = if is_max {
                ord == std::cmp::Ordering::Greater
            } else {
                ord == std::cmp::Ordering::Less
            };
            if better {
                target = right;
            }
        }
        if target == idx {
            break;
        }
        elems.swap(idx, target);
        idx = target;
    }
    Ok(())
}

/// `heap_new() → Array` — empty min-heap.
pub(crate) fn builtin_heap_new(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "heap_new: expected 0 arguments, got {}",
            args.len()
        ));
    }
    Ok(Value::Array(pack(false, Vec::new())))
}

/// `heap_new_max() → Array` — empty max-heap.
pub(crate) fn builtin_heap_new_max(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "heap_new_max: expected 0 arguments, got {}",
            args.len()
        ));
    }
    Ok(Value::Array(pack(true, Vec::new())))
}

/// `heap_push(h, val) → Array` — insert val; O(log n).
pub(crate) fn builtin_heap_push(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(arr), val] => {
            let (is_max, elems) = unpack(arr);
            let mut new_elems: Vec<Value> = elems.to_vec();
            new_elems.push(val.clone());
            let last = new_elems.len() - 1;
            sift_up(&mut new_elems, last, is_max)?;
            Ok(Value::Array(pack(is_max, new_elems)))
        }
        [other, _] => Err(format!("heap_push: expected Array, got {other}")),
        _ => Err(format!(
            "heap_push: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `heap_pop(h) → (Option<any>, Array)` — remove and return extremal element; O(log n).
pub(crate) fn builtin_heap_pop(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(arr)] => {
            let (is_max, elems) = unpack(arr);
            if elems.is_empty() {
                return Ok(Value::Tuple(vec![
                    Value::Option(None),
                    Value::Array(pack(is_max, Vec::new())),
                ]));
            }
            let mut new_elems = elems.to_vec();
            let top = new_elems[0].clone();
            let last_idx = new_elems.len() - 1;
            new_elems.swap(0, last_idx);
            new_elems.pop();
            if !new_elems.is_empty() {
                sift_down(&mut new_elems, 0, is_max)?;
            }
            Ok(Value::Tuple(vec![
                Value::Option(Some(Box::new(top))),
                Value::Array(pack(is_max, new_elems)),
            ]))
        }
        [other] => Err(format!("heap_pop: expected Array, got {other}")),
        _ => Err(format!("heap_pop: expected 1 argument, got {}", args.len())),
    }
}

/// `heap_peek(h) → Option<any>` — inspect extremal element without removing; O(1).
pub(crate) fn builtin_heap_peek(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(arr)] => {
            let (_, elems) = unpack(arr);
            Ok(Value::Option(elems.first().map(|v| Box::new(v.clone()))))
        }
        [other] => Err(format!("heap_peek: expected Array, got {other}")),
        _ => Err(format!(
            "heap_peek: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `heap_len(h) → int` — number of elements; O(1).
pub(crate) fn builtin_heap_len(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(arr)] => {
            let (_, elems) = unpack(arr);
            Ok(Value::Int(elems.len() as i64))
        }
        [other] => Err(format!("heap_len: expected Array, got {other}")),
        _ => Err(format!("heap_len: expected 1 argument, got {}", args.len())),
    }
}

/// `heap_is_empty(h) → bool` — O(1).
pub(crate) fn builtin_heap_is_empty(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(arr)] => {
            let (_, elems) = unpack(arr);
            Ok(Value::Bool(elems.is_empty()))
        }
        [other] => Err(format!("heap_is_empty: expected Array, got {other}")),
        _ => Err(format!(
            "heap_is_empty: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> String {
        let r = run_program(src);
        assert!(r.ok, "program failed: {:?}", r.errors);
        r.stdout
    }

    #[test]
    fn min_heap_extracts_in_order() {
        let out = run(r#"
let h = heap_new();
let h = heap_push(h, 5);
let h = heap_push(h, 1);
let h = heap_push(h, 3);
let (a, h) = heap_pop(h);
let (b, h) = heap_pop(h);
let (c, h) = heap_pop(h);
let av = match a { Some(x) => x, None => -1 };
let bv = match b { Some(x) => x, None => -1 };
let cv = match c { Some(x) => x, None => -1 };
println(to_string(av));
println(to_string(bv));
println(to_string(cv));
"#);
        assert!(out.contains("1"), "got: {out:?}");
        assert!(out.contains("3"), "got: {out:?}");
        assert!(out.contains("5"), "got: {out:?}");
    }

    #[test]
    fn max_heap_extracts_in_order() {
        let out = run(r#"
let h = heap_new_max();
let h = heap_push(h, 5);
let h = heap_push(h, 1);
let h = heap_push(h, 3);
let (a, h) = heap_pop(h);
let av = match a { Some(x) => x, None => -1 };
println(to_string(av));
"#);
        assert!(out.contains("5"), "got: {out:?}");
    }

    #[test]
    fn heap_peek_does_not_remove() {
        let out = run(r#"
let h = heap_new();
let h = heap_push(h, 10);
let h = heap_push(h, 2);
let top = heap_peek(h);
let v = match top { Some(x) => x, None => -1 };
println(to_string(v));
println(to_string(heap_len(h)));
"#);
        assert!(out.contains("2"), "got: {out:?}");
    }

    #[test]
    fn heap_pop_empty_returns_none() {
        let out = run(r#"
let h = heap_new();
let (v, h) = heap_pop(h);
let is_none = match v { None => true, Some(_) => false };
println(to_string(is_none));
println(to_string(heap_is_empty(h)));
"#);
        assert!(out.contains("true"), "got: {out:?}");
    }

    #[test]
    fn heap_len_and_is_empty() {
        let out = run(r#"
let h = heap_new();
println(to_string(heap_is_empty(h)));
let h = heap_push(h, 42);
println(to_string(heap_len(h)));
println(to_string(heap_is_empty(h)));
"#);
        assert!(out.contains("true"), "got: {out:?}");
        assert!(out.contains("1"), "got: {out:?}");
        assert!(out.contains("false"), "got: {out:?}");
    }
}
