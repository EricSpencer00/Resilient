//! RES-2588: Linked list collection (no_std compatible).
//!
//! Implements a doubly-linked list API backed by `Value::Array`.
//! In the interpreter, all operations are functional and return new lists.
//! In the embedded runtime (no_std), these would use intrusive pointers
//! for O(1) push/pop; in the interpreter we accept O(n) for front ops.
//!
//! API:
//!   linked_list_new()              → Array (empty list)
//!   linked_list_push_front(l, val) → Array — O(n) in interpreter
//!   linked_list_push_back(l, val)  → Array — O(1) amortized
//!   linked_list_pop_front(l)       → (Option<any>, Array) — O(n) in interpreter
//!   linked_list_pop_back(l)        → (Option<any>, Array) — O(1)
//!   linked_list_peek_front(l)      → Option<any> — O(1)
//!   linked_list_peek_back(l)       → Option<any> — O(1)
//!   linked_list_len(l)             → int — O(1)
//!   linked_list_is_empty(l)        → bool — O(1)
//!   linked_list_to_array(l)        → Array — O(1) (identity)
//!
//! Iteration: use `for elem in linked_list_to_array(l)` — works because
//! the backing store is already a `Value::Array`.

use crate::Value;

type RResult<T> = Result<T, String>;

/// `linked_list_new() → []`
pub(crate) fn builtin_linked_list_new(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "linked_list_new: expected 0 arguments, got {}",
            args.len()
        ));
    }
    Ok(Value::Array(Vec::new()))
}

/// `linked_list_push_front(l, val) → Array` — prepend; O(n).
pub(crate) fn builtin_linked_list_push_front(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(lst), val] => {
            let mut out = Vec::with_capacity(lst.len() + 1);
            out.push(val.clone());
            out.extend_from_slice(lst);
            Ok(Value::Array(out))
        }
        [other, _] => Err(format!(
            "linked_list_push_front: expected Array, got {other}"
        )),
        _ => Err(format!(
            "linked_list_push_front: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `linked_list_push_back(l, val) → Array` — append; O(1) amortized.
pub(crate) fn builtin_linked_list_push_back(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(lst), val] => {
            let mut out = lst.clone();
            out.push(val.clone());
            Ok(Value::Array(out))
        }
        [other, _] => Err(format!(
            "linked_list_push_back: expected Array, got {other}"
        )),
        _ => Err(format!(
            "linked_list_push_back: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `linked_list_pop_front(l) → (Option<any>, Array)` — remove head; O(n).
pub(crate) fn builtin_linked_list_pop_front(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(lst)] => {
            if lst.is_empty() {
                Ok(Value::Tuple(vec![
                    Value::Option(None),
                    Value::Array(Vec::new()),
                ]))
            } else {
                let head = lst[0].clone();
                Ok(Value::Tuple(vec![
                    Value::Option(Some(Box::new(head))),
                    Value::Array(lst[1..].to_vec()),
                ]))
            }
        }
        [other] => Err(format!(
            "linked_list_pop_front: expected Array, got {other}"
        )),
        _ => Err(format!(
            "linked_list_pop_front: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `linked_list_pop_back(l) → (Option<any>, Array)` — remove tail; O(1).
pub(crate) fn builtin_linked_list_pop_back(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(lst)] => {
            if lst.is_empty() {
                Ok(Value::Tuple(vec![
                    Value::Option(None),
                    Value::Array(Vec::new()),
                ]))
            } else {
                let mut rest = lst.clone();
                let tail = rest.pop().unwrap();
                Ok(Value::Tuple(vec![
                    Value::Option(Some(Box::new(tail))),
                    Value::Array(rest),
                ]))
            }
        }
        [other] => Err(format!("linked_list_pop_back: expected Array, got {other}")),
        _ => Err(format!(
            "linked_list_pop_back: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `linked_list_peek_front(l) → Option<any>` — inspect head; O(1).
pub(crate) fn builtin_linked_list_peek_front(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(lst)] => Ok(Value::Option(lst.first().map(|v| Box::new(v.clone())))),
        [other] => Err(format!(
            "linked_list_peek_front: expected Array, got {other}"
        )),
        _ => Err(format!(
            "linked_list_peek_front: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `linked_list_peek_back(l) → Option<any>` — inspect tail; O(1).
pub(crate) fn builtin_linked_list_peek_back(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(lst)] => Ok(Value::Option(lst.last().map(|v| Box::new(v.clone())))),
        [other] => Err(format!(
            "linked_list_peek_back: expected Array, got {other}"
        )),
        _ => Err(format!(
            "linked_list_peek_back: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `linked_list_len(l) → int` — O(1).
pub(crate) fn builtin_linked_list_len(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(lst)] => Ok(Value::Int(lst.len() as i64)),
        [other] => Err(format!("linked_list_len: expected Array, got {other}")),
        _ => Err(format!(
            "linked_list_len: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `linked_list_is_empty(l) → bool` — O(1).
pub(crate) fn builtin_linked_list_is_empty(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(lst)] => Ok(Value::Bool(lst.is_empty())),
        [other] => Err(format!("linked_list_is_empty: expected Array, got {other}")),
        _ => Err(format!(
            "linked_list_is_empty: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `linked_list_to_array(l) → Array` — expose for iteration; O(1).
pub(crate) fn builtin_linked_list_to_array(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(_)] => Ok(args[0].clone()),
        [other] => Err(format!("linked_list_to_array: expected Array, got {other}")),
        _ => Err(format!(
            "linked_list_to_array: expected 1 argument, got {}",
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
    fn push_front_and_back() {
        let out = run(r#"
let l = linked_list_new();
let l = linked_list_push_back(l, 2);
let l = linked_list_push_back(l, 3);
let l = linked_list_push_front(l, 1);
println(to_string(linked_list_len(l)));
"#);
        assert!(out.contains("3"), "got: {out:?}");
    }

    #[test]
    fn pop_front_removes_head() {
        let out = run(r#"
let l = linked_list_new();
let l = linked_list_push_back(l, 10);
let l = linked_list_push_back(l, 20);
let (head, l) = linked_list_pop_front(l);
let v = match head { Some(x) => x, None => -1 };
println(to_string(v));
println(to_string(linked_list_len(l)));
"#);
        assert!(out.contains("10"), "got: {out:?}");
        assert!(out.contains("1"), "got: {out:?}");
    }

    #[test]
    fn pop_back_removes_tail() {
        let out = run(r#"
let l = linked_list_new();
let l = linked_list_push_back(l, 10);
let l = linked_list_push_back(l, 20);
let (tail, l) = linked_list_pop_back(l);
let v = match tail { Some(x) => x, None => -1 };
println(to_string(v));
println(to_string(linked_list_len(l)));
"#);
        assert!(out.contains("20"), "got: {out:?}");
        assert!(out.contains("1"), "got: {out:?}");
    }

    #[test]
    fn peek_does_not_remove() {
        let out = run(r#"
let l = linked_list_new();
let l = linked_list_push_back(l, 42);
let front = linked_list_peek_front(l);
let v = match front { Some(x) => x, None => -1 };
println(to_string(v));
println(to_string(linked_list_len(l)));
"#);
        assert!(out.contains("42"), "got: {out:?}");
        assert!(out.contains("1"), "got: {out:?}");
    }

    #[test]
    fn iteration_via_to_array() {
        let out = run(r#"
let l = linked_list_new();
let l = linked_list_push_back(l, 10);
let l = linked_list_push_back(l, 20);
let l = linked_list_push_back(l, 30);
for x in linked_list_to_array(l) {
    println(to_string(x));
}
"#);
        assert!(out.contains("10"), "got: {out:?}");
        assert!(out.contains("20"), "got: {out:?}");
        assert!(out.contains("30"), "got: {out:?}");
    }

    #[test]
    fn pop_empty_returns_none() {
        let out = run(r#"
let l = linked_list_new();
let (v, l) = linked_list_pop_front(l);
let is_none = match v { None => true, Some(_) => false };
println(to_string(is_none));
println(to_string(linked_list_is_empty(l)));
"#);
        assert!(out.contains("true"), "got: {out:?}");
    }
}
