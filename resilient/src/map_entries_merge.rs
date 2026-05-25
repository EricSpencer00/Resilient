//! RES-1144: `map_entries`, `map_merge`, `map_is_empty` — the three
//! missing operations on `Value::Map`. Each ships in two name flavours
//! (`map_*` and `hashmap_*`) to mirror the existing aliasing pattern.
//!
//! All six are pure leaf builtins; the `hashmap_*` aliases reuse the
//! same implementations as their `map_*` counterparts.
//!
//! | Builtin | Signature | Purpose |
//! |---|---|---|
//! | `map_entries(m)`      | `(Map) -> Array`     | `[[k, v], ...]` sorted by key |
//! | `map_merge(a, b)`     | `(Map, Map) -> Map`  | `b` overrides `a` on conflict |
//! | `map_is_empty(m)`     | `(Map) -> Bool`      | True iff zero entries |
//! | `hashmap_entries(m)`  | alias                | |
//! | `hashmap_merge(a, b)` | alias                | |
//! | `hashmap_is_empty(m)` | alias                | |

use crate::{MapKey, RResult, Value};

/// Deterministic ordering matching `map_keys`. Sorts `Int` < `Str` <
/// `Bool` cross-variant; within each variant uses the natural order.
pub(crate) fn cmp_map_keys(a: &MapKey, b: &MapKey) -> std::cmp::Ordering {
    cmp_keys(a, b)
}

fn cmp_keys(a: &MapKey, b: &MapKey) -> std::cmp::Ordering {
    match (a, b) {
        (MapKey::Int(x), MapKey::Int(y)) => x.cmp(y),
        (MapKey::Str(x), MapKey::Str(y)) => x.cmp(y),
        (MapKey::Bool(x), MapKey::Bool(y)) => x.cmp(y),
        (MapKey::Int(_), _) => std::cmp::Ordering::Less,
        (_, MapKey::Int(_)) => std::cmp::Ordering::Greater,
        (MapKey::Str(_), _) => std::cmp::Ordering::Less,
        (_, MapKey::Str(_)) => std::cmp::Ordering::Greater,
    }
}

fn entries_impl(name: &str, args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Map(m)] => {
            let mut keys: Vec<&MapKey> = m.keys().collect();
            keys.sort_by(|a, b| cmp_keys(a, b));
            let out: Vec<Value> = keys
                .into_iter()
                .map(|k| {
                    let v = m.get(k).expect("key must exist — iter over the same map");
                    Value::Array(vec![k.to_value(), v.clone()])
                })
                .collect();
            Ok(Value::Array(out))
        }
        [a] => Err(format!("{}: expected a Map, got {}", name, a)),
        _ => Err(format!("{}: expected 1 argument, got {}", name, args.len())),
    }
}

fn merge_impl(name: &str, args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Map(a), Value::Map(b)] => {
            let mut out = a.clone();
            for (k, v) in b.iter() {
                out.insert(k.clone(), v.clone());
            }
            Ok(Value::Map(out))
        }
        [Value::Map(_), other] => Err(format!(
            "{}: second argument must be a Map, got {}",
            name, other
        )),
        [other, _] => Err(format!(
            "{}: first argument must be a Map, got {}",
            name, other
        )),
        _ => Err(format!(
            "{}: expected 2 arguments, got {}",
            name,
            args.len()
        )),
    }
}

fn is_empty_impl(name: &str, args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Map(m)] => Ok(Value::Bool(m.is_empty())),
        [a] => Err(format!("{}: expected a Map, got {}", name, a)),
        _ => Err(format!("{}: expected 1 argument, got {}", name, args.len())),
    }
}

/// `map_entries(m) -> Array` — every `[k, v]` pair in deterministic
/// sort order (same comparator as `map_keys`).
pub(crate) fn builtin_map_entries(args: &[Value]) -> RResult<Value> {
    entries_impl("map_entries", args)
}

/// `map_merge(a, b) -> Map` — fresh map with `a`'s entries plus all of
/// `b`'s entries; `b` wins on key conflict.
pub(crate) fn builtin_map_merge(args: &[Value]) -> RResult<Value> {
    merge_impl("map_merge", args)
}

/// `map_is_empty(m) -> Bool` — `true` iff `m` has zero entries.
pub(crate) fn builtin_map_is_empty(args: &[Value]) -> RResult<Value> {
    is_empty_impl("map_is_empty", args)
}

/// `hashmap_entries(m)` — alias for `map_entries`.
pub(crate) fn builtin_hashmap_entries(args: &[Value]) -> RResult<Value> {
    entries_impl("hashmap_entries", args)
}

/// `hashmap_merge(a, b)` — alias for `map_merge`.
pub(crate) fn builtin_hashmap_merge(args: &[Value]) -> RResult<Value> {
    merge_impl("hashmap_merge", args)
}

/// `hashmap_is_empty(m)` — alias for `map_is_empty`.
pub(crate) fn builtin_hashmap_is_empty(args: &[Value]) -> RResult<Value> {
    is_empty_impl("hashmap_is_empty", args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn map_with(pairs: &[(MapKey, Value)]) -> Value {
        let mut m = HashMap::new();
        for (k, v) in pairs {
            m.insert(k.clone(), v.clone());
        }
        Value::Map(m)
    }

    fn ki(n: i64) -> MapKey {
        MapKey::Int(n)
    }

    fn ks(s: &str) -> MapKey {
        MapKey::Str(s.to_string())
    }

    fn vi(n: i64) -> Value {
        Value::Int(n)
    }

    fn vs(s: &str) -> Value {
        Value::String(s.to_string())
    }

    fn as_bool(v: Value) -> bool {
        match v {
            Value::Bool(b) => b,
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    fn as_int(v: Value) -> i64 {
        match v {
            Value::Int(n) => n,
            other => panic!("expected Int, got {:?}", other),
        }
    }

    fn entry(v: Value) -> (Value, Value) {
        match v {
            Value::Array(items) if items.len() == 2 => {
                let mut it = items.into_iter();
                (it.next().unwrap(), it.next().unwrap())
            }
            other => panic!("expected 2-element Array, got {:?}", other),
        }
    }

    fn collect_entries(v: Value) -> Vec<(Value, Value)> {
        match v {
            Value::Array(items) => items.into_iter().map(entry).collect(),
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn entries_empty_map() {
        let r = builtin_map_entries(&[map_with(&[])]).unwrap();
        assert_eq!(collect_entries(r).len(), 0);
    }

    #[test]
    fn entries_sorted_by_int_key() {
        let m = map_with(&[(ki(3), vi(30)), (ki(1), vi(10)), (ki(2), vi(20))]);
        let r = collect_entries(builtin_map_entries(&[m]).unwrap());
        assert_eq!(r.len(), 3);
        // Sorted ascending by key
        assert_eq!(as_int(r[0].0.clone()), 1);
        assert_eq!(as_int(r[1].0.clone()), 2);
        assert_eq!(as_int(r[2].0.clone()), 3);
        assert_eq!(as_int(r[0].1.clone()), 10);
        assert_eq!(as_int(r[1].1.clone()), 20);
        assert_eq!(as_int(r[2].1.clone()), 30);
    }

    #[test]
    fn entries_int_keys_sort_before_string_keys() {
        let m = map_with(&[
            (ks("z"), vi(99)),
            (ki(5), vi(50)),
            (ks("a"), vi(1)),
            (ki(1), vi(10)),
        ]);
        let r = collect_entries(builtin_map_entries(&[m]).unwrap());
        assert_eq!(r.len(), 4);
        // Ints first (sorted), then strings (sorted).
        assert!(matches!(r[0].0, Value::Int(1)));
        assert!(matches!(r[1].0, Value::Int(5)));
        assert!(matches!(r[2].0, Value::String(ref s) if s == "a"));
        assert!(matches!(r[3].0, Value::String(ref s) if s == "z"));
    }

    #[test]
    fn entries_rejects_non_map() {
        let err = builtin_map_entries(&[vi(7)]).unwrap_err();
        assert!(err.contains("expected a Map"));
    }

    #[test]
    fn entries_rejects_wrong_arity() {
        let err = builtin_map_entries(&[]).unwrap_err();
        assert!(err.contains("expected 1"));
    }

    fn lookup_int(v: &Value, k: &MapKey) -> Option<i64> {
        match v {
            Value::Map(m) => match m.get(k)? {
                Value::Int(n) => Some(*n),
                _ => None,
            },
            _ => None,
        }
    }

    fn lookup_str(v: &Value, k: &MapKey) -> Option<String> {
        match v {
            Value::Map(m) => match m.get(k)? {
                Value::String(s) => Some(s.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    fn map_len(v: &Value) -> usize {
        match v {
            Value::Map(m) => m.len(),
            _ => panic!("expected Map"),
        }
    }

    #[test]
    fn merge_disjoint_keys_combines_both() {
        let a = map_with(&[(ki(1), vi(10)), (ki(2), vi(20))]);
        let b = map_with(&[(ki(3), vi(30)), (ki(4), vi(40))]);
        let r = builtin_map_merge(&[a, b]).unwrap();
        assert_eq!(map_len(&r), 4);
        assert_eq!(lookup_int(&r, &ki(1)), Some(10));
        assert_eq!(lookup_int(&r, &ki(4)), Some(40));
    }

    #[test]
    fn merge_b_overrides_a_on_conflict() {
        let a = map_with(&[(ki(1), vi(10)), (ki(2), vi(20))]);
        let b = map_with(&[(ki(2), vi(200)), (ki(3), vi(30))]);
        let r = builtin_map_merge(&[a, b]).unwrap();
        assert_eq!(map_len(&r), 3);
        assert_eq!(lookup_int(&r, &ki(1)), Some(10)); // from a, untouched
        assert_eq!(lookup_int(&r, &ki(2)), Some(200)); // b wins
        assert_eq!(lookup_int(&r, &ki(3)), Some(30)); // from b
    }

    #[test]
    fn merge_empty_left_returns_clone_of_right() {
        let a = map_with(&[]);
        let b = map_with(&[(ki(1), vi(10))]);
        let r = builtin_map_merge(&[a, b]).unwrap();
        assert_eq!(map_len(&r), 1);
        assert_eq!(lookup_int(&r, &ki(1)), Some(10));
    }

    #[test]
    fn merge_empty_right_returns_clone_of_left() {
        let a = map_with(&[(ks("k"), vs("v"))]);
        let b = map_with(&[]);
        let r = builtin_map_merge(&[a, b]).unwrap();
        assert_eq!(map_len(&r), 1);
        assert_eq!(lookup_str(&r, &ks("k")).as_deref(), Some("v"));
    }

    #[test]
    fn merge_does_not_mutate_inputs() {
        let a = map_with(&[(ki(1), vi(10))]);
        let b = map_with(&[(ki(1), vi(99))]);
        let _ = builtin_map_merge(&[a.clone(), b.clone()]).unwrap();
        // a still has the original value (b didn't mutate it)
        assert_eq!(lookup_int(&a, &ki(1)), Some(10));
    }

    #[test]
    fn merge_rejects_non_map_args() {
        let err = builtin_map_merge(&[map_with(&[]), vi(0)]).unwrap_err();
        assert!(err.contains("second argument"));
        let err = builtin_map_merge(&[vi(0), map_with(&[])]).unwrap_err();
        assert!(err.contains("first argument"));
    }

    #[test]
    fn merge_rejects_wrong_arity() {
        let err = builtin_map_merge(&[map_with(&[])]).unwrap_err();
        assert!(err.contains("expected 2"));
    }

    #[test]
    fn is_empty_basic() {
        assert!(as_bool(builtin_map_is_empty(&[map_with(&[])]).unwrap()));
        assert!(!as_bool(
            builtin_map_is_empty(&[map_with(&[(ki(1), vi(1))])]).unwrap()
        ));
    }

    #[test]
    fn is_empty_rejects_non_map() {
        let err = builtin_map_is_empty(&[vi(0)]).unwrap_err();
        assert!(err.contains("expected a Map"));
    }

    #[test]
    fn hashmap_aliases_share_behavior() {
        let m = map_with(&[(ki(2), vi(20)), (ki(1), vi(10))]);
        let from_map = collect_entries(builtin_map_entries(std::slice::from_ref(&m)).unwrap());
        let from_hashmap =
            collect_entries(builtin_hashmap_entries(std::slice::from_ref(&m)).unwrap());
        assert_eq!(from_map.len(), from_hashmap.len());
        for (a, b) in from_map.iter().zip(from_hashmap.iter()) {
            assert_eq!(as_int(a.0.clone()), as_int(b.0.clone()));
            assert_eq!(as_int(a.1.clone()), as_int(b.1.clone()));
        }

        let a = map_with(&[(ki(1), vi(10))]);
        let b = map_with(&[(ki(2), vi(20))]);
        let merged_m = builtin_map_merge(&[a.clone(), b.clone()]).unwrap();
        let merged_h = builtin_hashmap_merge(&[a, b]).unwrap();
        assert_eq!(map_len(&merged_m), map_len(&merged_h));
        assert_eq!(lookup_int(&merged_m, &ki(1)), lookup_int(&merged_h, &ki(1)));
        assert_eq!(lookup_int(&merged_m, &ki(2)), lookup_int(&merged_h, &ki(2)));

        assert!(as_bool(builtin_hashmap_is_empty(&[map_with(&[])]).unwrap()));
        assert!(!as_bool(
            builtin_hashmap_is_empty(&[map_with(&[(ki(1), vi(1))])]).unwrap()
        ));
    }

    #[test]
    fn hashmap_aliases_use_correct_error_name() {
        let err = builtin_hashmap_entries(&[vi(0)]).unwrap_err();
        assert!(err.contains("hashmap_entries"), "got {}", err);
        let err = builtin_hashmap_merge(&[vi(0), vi(0)]).unwrap_err();
        assert!(err.contains("hashmap_merge"), "got {}", err);
        let err = builtin_hashmap_is_empty(&[vi(0)]).unwrap_err();
        assert!(err.contains("hashmap_is_empty"), "got {}", err);
    }
}
