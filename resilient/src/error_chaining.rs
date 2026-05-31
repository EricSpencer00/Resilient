//! RES-2792: error chaining — `.context()`, `.root_cause()`, `.chain()` on Result.
//!
//! Adds three methods to `Result` values for building causal error chains:
//!
//! - `result.context("msg")` — if Err, wraps the error by prepending context.
//!   If Ok, passes through unchanged.
//! - `result.root_cause()` — returns the innermost error string.
//! - `result.chain()` — returns an array of error strings from outermost
//!   to innermost.
//!
//! Internally, chained errors are stored as `Err(Array[outer, ..., inner])`.
//! The `Display` impl for Result already formats this as `"outer: ... : inner"`.

use crate::{RResult, Value};

pub fn dispatch_result_method(
    ok: bool,
    payload: &Value,
    method: &str,
    args: &[Value],
) -> Option<RResult<Value>> {
    match method {
        "context" => Some(result_context(ok, payload, args)),
        "root_cause" => Some(result_root_cause(ok, payload)),
        "chain" => Some(result_chain(ok, payload)),
        _ => None,
    }
}

fn result_context(ok: bool, payload: &Value, args: &[Value]) -> RResult<Value> {
    if ok {
        return Ok(Value::Result {
            ok: true,
            payload: Box::new(payload.clone()),
        });
    }
    let ctx_msg = match args.first() {
        Some(Value::String(s)) => s.clone(),
        Some(other) => format!("{}", other),
        None => return Err("context: expected a message argument".to_string()),
    };
    let chain = match payload {
        Value::Array(segments) => {
            let mut new_chain = vec![Value::String(ctx_msg)];
            new_chain.extend(segments.iter().cloned());
            new_chain
        }
        _ => vec![Value::String(ctx_msg), payload.clone()],
    };
    Ok(Value::Result {
        ok: false,
        payload: Box::new(Value::Array(chain)),
    })
}

fn result_root_cause(ok: bool, payload: &Value) -> RResult<Value> {
    if ok {
        return Err("root_cause: called on Ok result".to_string());
    }
    match payload {
        Value::Array(segments) if !segments.is_empty() => Ok(segments.last().unwrap().clone()),
        _ => Ok(payload.clone()),
    }
}

fn result_chain(ok: bool, payload: &Value) -> RResult<Value> {
    if ok {
        return Err("chain: called on Ok result".to_string());
    }
    match payload {
        Value::Array(_) => Ok(payload.clone()),
        _ => Ok(Value::Array(vec![payload.clone()])),
    }
}

pub fn format_chained_error(payload: &Value) -> String {
    match payload {
        Value::Array(segments) => segments
            .iter()
            .map(value_to_plain_string)
            .collect::<Vec<_>>()
            .join(": "),
        _ => value_to_plain_string(payload),
    }
}

/// Unwrap an Err payload, formatting chains as strings but preserving
/// non-chain payloads as-is (e.g., `Err(99)` stays `Int(99)`).
pub fn unwrap_err_payload(payload: &Value) -> Value {
    match payload {
        Value::Array(_) => Value::String(format_chained_error(payload)),
        _ => payload.clone(),
    }
}

fn value_to_plain_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => format!("{}", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_wraps_err_string() {
        let r = dispatch_result_method(
            false,
            &Value::String("file not found".into()),
            "context",
            &[Value::String("loading config".into())],
        )
        .unwrap()
        .unwrap();
        match r {
            Value::Result { ok: false, payload } => {
                let formatted = format_chained_error(&payload);
                assert_eq!(formatted, "loading config: file not found");
            }
            other => panic!("expected Err result, got {:?}", other),
        }
    }

    #[test]
    fn context_passes_through_ok() {
        let r = dispatch_result_method(
            true,
            &Value::Int(42),
            "context",
            &[Value::String("ignored".into())],
        )
        .unwrap()
        .unwrap();
        match r {
            Value::Result {
                ok: true, payload, ..
            } => match *payload {
                Value::Int(42) => {}
                ref other => panic!("expected Int(42), got {:?}", other),
            },
            other => panic!("expected Ok result, got {:?}", other),
        }
    }

    #[test]
    fn chained_context_accumulates() {
        let inner = Value::String("parse error".into());
        let r1 = result_context(false, &inner, &[Value::String("reading file".into())]).unwrap();
        let Value::Result {
            ok: false,
            payload: p1,
        } = r1
        else {
            panic!("expected Err");
        };
        let r2 = result_context(false, &p1, &[Value::String("loading config".into())]).unwrap();
        let Value::Result {
            ok: false,
            payload: p2,
        } = r2
        else {
            panic!("expected Err");
        };
        let formatted = format_chained_error(&p2);
        assert_eq!(formatted, "loading config: reading file: parse error");
    }

    #[test]
    fn root_cause_returns_innermost() {
        let chain = Value::Array(vec![
            Value::String("loading config".into()),
            Value::String("reading file".into()),
            Value::String("parse error".into()),
        ]);
        let r = result_root_cause(false, &chain).unwrap();
        match r {
            Value::String(s) => assert_eq!(s, "parse error"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn chain_returns_all_segments() {
        let chain = Value::Array(vec![
            Value::String("loading config".into()),
            Value::String("reading file".into()),
            Value::String("parse error".into()),
        ]);
        let r = result_chain(false, &chain).unwrap();
        match r {
            Value::Array(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(value_to_plain_string(&items[0]), "loading config");
                assert_eq!(value_to_plain_string(&items[1]), "reading file");
                assert_eq!(value_to_plain_string(&items[2]), "parse error");
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn root_cause_plain_string_returns_itself() {
        let r = result_root_cause(false, &Value::String("simple error".into())).unwrap();
        match r {
            Value::String(s) => assert_eq!(s, "simple error"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn root_cause_on_ok_errors() {
        assert!(result_root_cause(true, &Value::Int(1)).is_err());
    }

    #[test]
    fn chain_on_ok_errors() {
        assert!(result_chain(true, &Value::Int(1)).is_err());
    }

    #[test]
    fn context_without_arg_errors() {
        assert!(result_context(false, &Value::String("x".into()), &[]).is_err());
    }

    #[test]
    fn end_to_end_via_run() {
        let r = crate::run_program(
            r#"
fn parse_int(string s) -> Result<int, string> {
    if s == "42" {
        return Ok(42)
    }
    Err("not a number")
}

fn load_value(string s) -> Result<int, string> {
    let r = parse_int(s)
    r.context("load_value failed")
}

let result = load_value("abc")
println(is_err(result))
let cause = result.root_cause()
println(cause)
let parts = result.chain()
println(len(parts))
"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.lines().collect();
        assert_eq!(lines[0], "true");
        assert_eq!(lines[1], "not a number");
        assert_eq!(lines[2], "2");
    }
}
