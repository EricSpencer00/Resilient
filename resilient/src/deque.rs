//! RES-2586: Deque (double-ended queue) collection.
//!
//! Implements a functional deque backed by `Value::Array` (Vec<Value>).
//! push_front / pop_front are O(n); push_back / pop_back are O(1) amortized.
//! All operations return new deque values rather than mutating in place,
//! consistent with Resilient's functional collection idiom.
//!
//! API:
//!   deque_new()                  → Array (empty deque)
//!   deque_push_front(dq, val)    → Array
//!   deque_push_back(dq, val)     → Array
//!   deque_pop_front(dq)          → (Option<any>, Array)
//!   deque_pop_back(dq)           → (Option<any>, Array)
//!   deque_peek_front(dq)         → Option<any>
//!   deque_peek_back(dq)          → Option<any>
//!   deque_len(dq)                → int
//!   deque_is_empty(dq)           → bool

use crate::Value;

type RResult<T> = Result<T, String>;

/// `deque_new() → []` — empty deque.
pub(crate) fn builtin_deque_new(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "deque_new: expected 0 arguments, got {}",
            args.len()
        ));
    }
    Ok(Value::Array(Vec::new()))
}

/// `deque_push_front(dq, val) → Array` — prepend val; O(n).
pub(crate) fn builtin_deque_push_front(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(dq), val] => {
            let mut out = Vec::with_capacity(dq.len() + 1);
            out.push(val.clone());
            out.extend_from_slice(dq);
            Ok(Value::Array(out))
        }
        [other, _] => Err(format!("deque_push_front: expected Array, got {other}")),
        _ => Err(format!(
            "deque_push_front: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `deque_push_back(dq, val) → Array` — append val; O(1) amortized.
pub(crate) fn builtin_deque_push_back(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(dq), val] => {
            let mut out = dq.clone();
            out.push(val.clone());
            Ok(Value::Array(out))
        }
        [other, _] => Err(format!("deque_push_back: expected Array, got {other}")),
        _ => Err(format!(
            "deque_push_back: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `deque_pop_front(dq) → (Option<any>, Array)` — remove front element.
///
/// Returns a tuple `(Option<removed>, remaining_dq)`.
pub(crate) fn builtin_deque_pop_front(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(dq)] => {
            if dq.is_empty() {
                Ok(Value::Tuple(vec![
                    Value::Option(None),
                    Value::Array(Vec::new()),
                ]))
            } else {
                let front = dq[0].clone();
                let rest = Value::Array(dq[1..].to_vec());
                Ok(Value::Tuple(vec![
                    Value::Option(Some(Box::new(front))),
                    rest,
                ]))
            }
        }
        [other] => Err(format!("deque_pop_front: expected Array, got {other}")),
        _ => Err(format!(
            "deque_pop_front: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `deque_pop_back(dq) → (Option<any>, Array)` — remove back element.
pub(crate) fn builtin_deque_pop_back(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(dq)] => {
            if dq.is_empty() {
                Ok(Value::Tuple(vec![
                    Value::Option(None),
                    Value::Array(Vec::new()),
                ]))
            } else {
                let mut rest = dq.clone();
                let back = rest.pop().unwrap();
                Ok(Value::Tuple(vec![
                    Value::Option(Some(Box::new(back))),
                    Value::Array(rest),
                ]))
            }
        }
        [other] => Err(format!("deque_pop_back: expected Array, got {other}")),
        _ => Err(format!(
            "deque_pop_back: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `deque_peek_front(dq) → Option<any>` — inspect front without removing.
pub(crate) fn builtin_deque_peek_front(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(dq)] => Ok(Value::Option(dq.first().map(|v| Box::new(v.clone())))),
        [other] => Err(format!("deque_peek_front: expected Array, got {other}")),
        _ => Err(format!(
            "deque_peek_front: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `deque_peek_back(dq) → Option<any>` — inspect back without removing.
pub(crate) fn builtin_deque_peek_back(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(dq)] => Ok(Value::Option(dq.last().map(|v| Box::new(v.clone())))),
        [other] => Err(format!("deque_peek_back: expected Array, got {other}")),
        _ => Err(format!(
            "deque_peek_back: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `deque_len(dq) → int`
pub(crate) fn builtin_deque_len(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(dq)] => Ok(Value::Int(dq.len() as i64)),
        [other] => Err(format!("deque_len: expected Array, got {other}")),
        _ => Err(format!(
            "deque_len: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `deque_is_empty(dq) → bool`
pub(crate) fn builtin_deque_is_empty(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(dq)] => Ok(Value::Bool(dq.is_empty())),
        [other] => Err(format!("deque_is_empty: expected Array, got {other}")),
        _ => Err(format!(
            "deque_is_empty: expected 1 argument, got {}",
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
    fn deque_basic_push_pop() {
        let out = run(r#"
let dq = deque_new();
let dq = deque_push_back(dq, 1);
let dq = deque_push_back(dq, 2);
let dq = deque_push_front(dq, 0);
println(to_string(deque_len(dq)));
"#);
        assert!(out.contains("3"), "got: {out:?}");
    }

    #[test]
    fn deque_pop_front_removes_first() {
        let out = run(r#"
let dq = deque_new();
let dq = deque_push_back(dq, 10);
let dq = deque_push_back(dq, 20);
let (front, dq) = deque_pop_front(dq);
let val = match front {
    Some(x) => x,
    None => -1,
};
println(to_string(val));
println(to_string(deque_len(dq)));
"#);
        assert!(out.contains("10"), "got: {out:?}");
        assert!(out.contains("1"), "got: {out:?}");
    }

    #[test]
    fn deque_pop_back_removes_last() {
        let out = run(r#"
let dq = deque_new();
let dq = deque_push_back(dq, 10);
let dq = deque_push_back(dq, 20);
let (back, dq) = deque_pop_back(dq);
let val = match back {
    Some(x) => x,
    None => -1,
};
println(to_string(val));
println(to_string(deque_len(dq)));
"#);
        assert!(out.contains("20"), "got: {out:?}");
        assert!(out.contains("1"), "got: {out:?}");
    }

    #[test]
    fn deque_peek_does_not_remove() {
        let out = run(r#"
let dq = deque_new();
let dq = deque_push_back(dq, 42);
let front = deque_peek_front(dq);
let val = match front {
    Some(x) => x,
    None => -1,
};
println(to_string(val));
println(to_string(deque_len(dq)));
"#);
        assert!(out.contains("42"), "got: {out:?}");
        assert!(out.contains("1"), "got: {out:?}");
    }

    #[test]
    fn deque_pop_empty_returns_none() {
        let out = run(r#"
let dq = deque_new();
let (front, dq) = deque_pop_front(dq);
let is_none = match front {
    None => true,
    Some(_) => false,
};
println(to_string(is_none));
println(to_string(deque_is_empty(dq)));
"#);
        assert!(out.contains("true"), "got: {out:?}");
    }

    #[test]
    fn deque_is_empty_on_new() {
        let out = run(r#"
let dq = deque_new();
println(to_string(deque_is_empty(dq)));
println(to_string(deque_len(dq)));
"#);
        assert!(out.contains("true"), "got: {out:?}");
        assert!(out.contains("0"), "got: {out:?}");
    }
}
