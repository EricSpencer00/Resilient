//! Grand-Implementation Pass 2 — Subsystem B: Provenance-Tagged Values.
//!
//! No production language has provenance tagging as a runtime primitive.
//! OpenTelemetry / Jaeger does it for distributed tracing, but at the
//! library level. ROS bag tools stamp messages, but you need to wrap each
//! value yourself. F# / OCaml / Haskell phantom types tag at compile time
//! but have no runtime representation. Resilient gives you a 3-builtin
//! kernel for runtime provenance:
//!
//!   * `tag(value: int, source: string) -> Tagged` — wrap an int with a
//!     provenance string. Encoded as a 2-element `Value::Array` so it
//!     interoperates with the existing language without a new Value variant.
//!     Layout: `[String(source), Int(value)]`.
//!   * `untag(t: Tagged, expected: string) -> int` — unwrap, asserting the
//!     tag matches `expected`. Returns `Result::Err` with a clear
//!     "provenance mismatch" message on conflict — no silent extraction.
//!   * `tag_of(t: Tagged) -> string` — read the tag without consuming.
//!
//! Why this is unique: the runtime makes provenance non-discardable. You
//! cannot read the inner value without naming the source you expect.
//! Sensor fusion, audit lineage, and cross-module value flow all benefit
//! — the language itself prevents "where did this number come from?"
//! ambiguity at every read site.

use crate::Value;

type RResult<T> = Result<T, String>;

/// `tag(value: Int, source: String) -> Tagged` — wrap a value with a
/// provenance string. Returned shape: `Array[String(source), Int(value)]`.
pub(crate) fn builtin_tag(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(v), Value::String(src)] => Ok(Value::Array(vec![
            Value::String(src.clone()),
            Value::Int(*v),
        ])),
        [a, b] => Err(format!(
            "tag: expected (Int, String), got ({}, {})",
            type_name(a),
            type_name(b)
        )),
        _ => Err(format!("tag: expected 2 arguments, got {}", args.len())),
    }
}

/// `untag(tagged: Tagged, expected: String) -> Int` — extract the inner
/// value, asserting the provenance string matches `expected`. Wrapped in
/// `Value::Result` so callers can `?`-propagate or pattern-match.
pub(crate) fn builtin_untag(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), Value::String(expected)] => {
            if items.len() != 2 {
                return Ok(Value::Result {
                    ok: false,
                    payload: Box::new(Value::String(format!(
                        "untag: not a tagged value (expected 2-element [String, Int], got {} elements)",
                        items.len()
                    ))),
                });
            }
            let (Value::String(actual), Value::Int(v)) = (&items[0], &items[1]) else {
                return Ok(Value::Result {
                    ok: false,
                    payload: Box::new(Value::String(
                        "untag: not a tagged value (slot 0 must be String, slot 1 must be Int)"
                            .to_string(),
                    )),
                });
            };
            if actual == expected {
                Ok(Value::Result {
                    ok: true,
                    payload: Box::new(Value::Int(*v)),
                })
            } else {
                Ok(Value::Result {
                    ok: false,
                    payload: Box::new(Value::String(format!(
                        "untag: provenance mismatch — expected '{expected}', got '{actual}'"
                    ))),
                })
            }
        }
        [a, b] => Err(format!(
            "untag: expected (Tagged, String), got ({}, {})",
            type_name(a),
            type_name(b)
        )),
        _ => Err(format!("untag: expected 2 arguments, got {}", args.len())),
    }
}

/// `tag_of(tagged: Tagged) -> String` — read the provenance string without
/// consuming the tagged value. Returns `Result::Err` if not a Tagged shape.
pub(crate) fn builtin_tag_of(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            if items.len() != 2 {
                return Ok(Value::Result {
                    ok: false,
                    payload: Box::new(Value::String(format!(
                        "tag_of: not a tagged value (expected 2-element array, got {})",
                        items.len()
                    ))),
                });
            }
            match &items[0] {
                Value::String(s) => Ok(Value::Result {
                    ok: true,
                    payload: Box::new(Value::String(s.clone())),
                }),
                _ => Ok(Value::Result {
                    ok: false,
                    payload: Box::new(Value::String(
                        "tag_of: not a tagged value (slot 0 must be String)".to_string(),
                    )),
                }),
            }
        }
        [a] => Err(format!(
            "tag_of: expected Tagged (Array), got {}",
            type_name(a)
        )),
        _ => Err(format!("tag_of: expected 1 argument, got {}", args.len())),
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
        Value::String(_) => "String",
        Value::Bool(_) => "Bool",
        Value::Array(_) => "Array",
        Value::Struct { .. } => "Struct",
        Value::Result { .. } => "Result",
        Value::Option(_) => "Option",
        _ => "<value>",
    }
}
