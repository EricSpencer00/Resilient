//! Grand-Implementation Pass 2 — Subsystem C: Named Snapshots.
//!
//! Erlang's process state, Smalltalk image, Common Lisp save-image: each
//! supports whole-VM snapshots. Java has Serialization, .NET has
//! AppDomain — both library-level. *No* mainstream language has
//! programmer-named, addressable, in-process snapshots in the core
//! stdlib. The closest analogue is database checkpoints, which live
//! outside the language.
//!
//! Resilient adds:
//!
//!   * `snapshot_save(name: String, value: Int) -> Int` — store the value
//!     under `name`. Returns the value (chains nicely in expressions).
//!   * `snapshot_load(name: String) -> Result<Int>` — read the stored value.
//!     `Err` if no snapshot exists for that name.
//!   * `snapshot_keys() -> Array<String>` — list every saved snapshot name.
//!   * `snapshot_clear(name: String) -> Bool` — remove a snapshot, return
//!     true if it existed.
//!
//! The store is bounded (`MAX_SNAPSHOTS`) so embedded targets cannot OOM.
//! When at capacity, additional `snapshot_save` calls evict the
//! lexicographically-smallest existing key — predictable and bounded.
//!
//! This MVP is integer-valued; richer payloads (struct snapshots, array
//! snapshots) follow naturally as Value variants are added. The point of
//! this PR is the *primitive*: the language now lets you name a moment in
//! state and re-anchor to it later, without a library, agent, or DB.

use crate::Value;
use std::cell::RefCell;
use std::collections::BTreeMap;

type RResult<T> = Result<T, String>;

const MAX_SNAPSHOTS: usize = 256;

thread_local! {
    static STORE: RefCell<BTreeMap<String, i64>> = const { RefCell::new(BTreeMap::new()) };
}

pub(crate) fn builtin_snapshot_save(args: &[Value]) -> RResult<Value> {
    let (name, value) = match args {
        [Value::String(n), Value::Int(v)] => (n.clone(), *v),
        [a, b] => {
            return Err(format!(
                "snapshot_save: expected (String, Int), got ({}, {})",
                type_name(a),
                type_name(b)
            ));
        }
        _ => {
            return Err(format!(
                "snapshot_save: expected 2 args, got {}",
                args.len()
            ));
        }
    };
    STORE.with(|s| {
        let mut s = s.borrow_mut();
        if !s.contains_key(&name) && s.len() >= MAX_SNAPSHOTS {
            // Evict the lexicographically-smallest key — bounded behaviour.
            if let Some(k) = s.keys().next().cloned() {
                s.remove(&k);
            }
        }
        s.insert(name, value);
    });
    Ok(Value::Int(value))
}

pub(crate) fn builtin_snapshot_load(args: &[Value]) -> RResult<Value> {
    let name = match args {
        [Value::String(n)] => n.clone(),
        [a] => {
            return Err(format!(
                "snapshot_load: expected String, got {}",
                type_name(a)
            ));
        }
        _ => {
            return Err(format!("snapshot_load: expected 1 arg, got {}", args.len()));
        }
    };
    let result = STORE.with(|s| s.borrow().get(&name).copied());
    Ok(match result {
        Some(v) => Value::Result {
            ok: true,
            payload: Box::new(Value::Int(v)),
        },
        None => Value::Result {
            ok: false,
            payload: Box::new(Value::String(format!(
                "snapshot_load: no snapshot named '{name}'"
            ))),
        },
    })
}

pub(crate) fn builtin_snapshot_keys(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "snapshot_keys: expected 0 args, got {}",
            args.len()
        ));
    }
    let keys: Vec<Value> = STORE.with(|s| {
        s.borrow()
            .keys()
            .map(|k| Value::String(k.clone()))
            .collect()
    });
    Ok(Value::Array(keys))
}

pub(crate) fn builtin_snapshot_clear(args: &[Value]) -> RResult<Value> {
    let name = match args {
        [Value::String(n)] => n.clone(),
        [a] => {
            return Err(format!(
                "snapshot_clear: expected String, got {}",
                type_name(a)
            ));
        }
        _ => {
            return Err(format!(
                "snapshot_clear: expected 1 arg, got {}",
                args.len()
            ));
        }
    };
    let removed = STORE.with(|s| s.borrow_mut().remove(&name).is_some());
    Ok(Value::Bool(removed))
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
        Value::String(_) => "String",
        Value::Bool(_) => "Bool",
        Value::Array(_) => "Array",
        _ => "<value>",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_roundtrip() {
        builtin_snapshot_save(&[Value::String("test_key".into()), Value::Int(42)]).unwrap();
        let loaded = builtin_snapshot_load(&[Value::String("test_key".into())]).unwrap();
        // load returns a Result value wrapping the Int
        assert!(
            format!("{loaded:?}").contains("42"),
            "loaded value must be 42: {loaded:?}"
        );
    }

    #[test]
    fn load_missing_snapshot_returns_err_result_value() {
        let result = builtin_snapshot_load(&[Value::String("does_not_exist_xyz".into())]).unwrap();
        assert!(
            matches!(result, Value::Result { ok: false, .. }),
            "loading a missing snapshot must return a Result with ok=false"
        );
    }

    #[test]
    fn snapshot_clear_returns_true_if_existed() {
        builtin_snapshot_save(&[Value::String("clearme".into()), Value::Int(1)]).unwrap();
        let cleared = builtin_snapshot_clear(&[Value::String("clearme".into())]).unwrap();
        assert!(
            matches!(cleared, Value::Bool(true)),
            "clear must return true for existing snapshot"
        );
        let cleared_again = builtin_snapshot_clear(&[Value::String("clearme".into())]).unwrap();
        assert!(
            matches!(cleared_again, Value::Bool(false)),
            "second clear must return false — already gone"
        );
    }

    #[test]
    fn save_wrong_arity_errors() {
        let result = builtin_snapshot_save(&[Value::String("k".into())]);
        assert!(result.is_err(), "wrong arity must return Err");
    }
}
