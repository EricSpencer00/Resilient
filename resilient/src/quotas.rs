//! Grand-Implementation Pass 3 — Subsystem D: Named Resource Quotas.
//!
//! Embedded systems track many resource budgets simultaneously: radio
//! airtime, SD-card writes, encryption operations, sensor sample counts.
//! Every project rolls a fresh ad-hoc counter map. Linux cgroups, Kubernetes
//! resource quotas, AWS API throttles — all live outside the language.
//! No production language ships *named, programmer-addressable resource
//! quotas* in the core stdlib.
//!
//! Resilient adds:
//!
//!   * `quota_set(name: String, limit: Int) -> Int` — install (or replace)
//!     a quota with the given limit; resets the used counter to 0.
//!     Returns the limit for chaining.
//!   * `quota_charge(name: String, amount: Int) -> Bool` — atomically
//!     consume `amount` against the quota. Returns `true` if the charge
//!     succeeded (used + amount ≤ limit), `false` otherwise. The counter
//!     is *not* incremented on a `false` return — a denied charge has no
//!     side effect.
//!   * `quota_remaining(name: String) -> Int` — peek (limit − used);
//!     returns -1 if the quota does not exist.
//!   * `quota_reset(name: String) -> Int` — set used back to 0; returns
//!     the prior used count.
//!   * `quota_used(name: String) -> Int` — peek used count; returns -1
//!     if quota missing.
//!   * `quotas() -> Array<String>` — list quota names in lex order.
//!
//! Bounded by `MAX_QUOTAS` so embedded targets cannot OOM.

#![allow(clippy::collapsible_if)]

use crate::Value;
use std::cell::RefCell;
use std::collections::BTreeMap;

type RResult<T> = Result<T, String>;

const MAX_QUOTAS: usize = 256;

#[derive(Clone, Copy)]
struct Quota {
    limit: i64,
    used: i64,
}

thread_local! {
    static QUOTAS: RefCell<BTreeMap<String, Quota>> = const { RefCell::new(BTreeMap::new()) };
}

pub(crate) fn builtin_quota_set(args: &[Value]) -> RResult<Value> {
    let (name, limit) = match args {
        [Value::String(n), Value::Int(l)] => (n.clone(), *l),
        [a, b] => {
            return Err(format!(
                "quota_set: expected (String, Int), got ({}, {})",
                type_name(a),
                type_name(b)
            ));
        }
        _ => return Err(format!("quota_set: expected 2 args, got {}", args.len())),
    };
    if limit < 0 {
        return Err("quota_set: limit must be non-negative".to_string());
    }
    QUOTAS.with(|q| {
        let mut q = q.borrow_mut();
        if !q.contains_key(&name) && q.len() >= MAX_QUOTAS {
            // Bounded: evict the lex-smallest existing quota.
            if let Some(k) = q.keys().next().cloned() {
                q.remove(&k);
            }
        }
        q.insert(name, Quota { limit, used: 0 });
    });
    Ok(Value::Int(limit))
}

pub(crate) fn builtin_quota_charge(args: &[Value]) -> RResult<Value> {
    // RES-2128: borrow the name as `&str` from `args` instead of cloning.
    // `BTreeMap::get_mut(&Q)` uses `String: Borrow<str>`, so the lookup
    // is allocation-free; `quota_charge` does not insert into `QUOTAS`,
    // it only reads-or-updates an existing entry. Same for `quota_remaining`,
    // `quota_reset`, and `quota_used` below. `quota_set` (above) keeps
    // its clone because `q.insert(name, …)` consumes an owned `String`
    // key.
    let (name, amount) = match args {
        [Value::String(n), Value::Int(a)] => (n.as_str(), *a),
        [a, b] => {
            return Err(format!(
                "quota_charge: expected (String, Int), got ({}, {})",
                type_name(a),
                type_name(b)
            ));
        }
        _ => return Err(format!("quota_charge: expected 2 args, got {}", args.len())),
    };
    if amount < 0 {
        return Err("quota_charge: amount must be non-negative".to_string());
    }
    let granted = QUOTAS.with(|q| {
        let mut q = q.borrow_mut();
        match q.get_mut(name) {
            Some(quota) => {
                let next = quota.used.saturating_add(amount);
                if next <= quota.limit {
                    quota.used = next;
                    true
                } else {
                    false
                }
            }
            None => false,
        }
    });
    Ok(Value::Bool(granted))
}

pub(crate) fn builtin_quota_remaining(args: &[Value]) -> RResult<Value> {
    let name = match args {
        [Value::String(n)] => n.as_str(),
        [a] => {
            return Err(format!(
                "quota_remaining: expected String, got {}",
                type_name(a)
            ));
        }
        _ => {
            return Err(format!(
                "quota_remaining: expected 1 arg, got {}",
                args.len()
            ));
        }
    };
    let remaining = QUOTAS.with(|q| q.borrow().get(name).map(|q| q.limit - q.used).unwrap_or(-1));
    Ok(Value::Int(remaining))
}

pub(crate) fn builtin_quota_reset(args: &[Value]) -> RResult<Value> {
    let name = match args {
        [Value::String(n)] => n.as_str(),
        [a] => {
            return Err(format!(
                "quota_reset: expected String, got {}",
                type_name(a)
            ));
        }
        _ => return Err(format!("quota_reset: expected 1 arg, got {}", args.len())),
    };
    let prior = QUOTAS.with(|q| {
        let mut q = q.borrow_mut();
        match q.get_mut(name) {
            Some(quota) => {
                let prior = quota.used;
                quota.used = 0;
                prior
            }
            None => -1,
        }
    });
    Ok(Value::Int(prior))
}

pub(crate) fn builtin_quota_used(args: &[Value]) -> RResult<Value> {
    let name = match args {
        [Value::String(n)] => n.as_str(),
        [a] => {
            return Err(format!("quota_used: expected String, got {}", type_name(a)));
        }
        _ => return Err(format!("quota_used: expected 1 arg, got {}", args.len())),
    };
    let used = QUOTAS.with(|q| q.borrow().get(name).map(|q| q.used).unwrap_or(-1));
    Ok(Value::Int(used))
}

pub(crate) fn builtin_quotas(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!("quotas: expected 0 args, got {}", args.len()));
    }
    let names: Vec<Value> = QUOTAS.with(|q| {
        q.borrow()
            .keys()
            .map(|k| Value::String(k.clone()))
            .collect()
    });
    Ok(Value::Array(names))
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

    fn set(name: &str, limit: i64) -> Value {
        builtin_quota_set(&[Value::String(name.into()), Value::Int(limit)]).unwrap()
    }

    fn charge(name: &str, amount: i64) -> bool {
        match builtin_quota_charge(&[Value::String(name.into()), Value::Int(amount)]).unwrap() {
            Value::Bool(b) => b,
            _ => panic!("expected Bool"),
        }
    }

    fn remaining(name: &str) -> i64 {
        match builtin_quota_remaining(&[Value::String(name.into())]).unwrap() {
            Value::Int(n) => n,
            _ => panic!("expected Int"),
        }
    }

    #[test]
    fn quota_set_and_charge_basic() {
        set("test_basic", 10);
        assert!(charge("test_basic", 5), "first charge must succeed");
        assert_eq!(remaining("test_basic"), 5);
        assert!(
            charge("test_basic", 5),
            "charge that hits exactly the limit must succeed"
        );
        assert_eq!(remaining("test_basic"), 0);
        assert!(!charge("test_basic", 1), "charge past limit must fail");
        assert_eq!(
            remaining("test_basic"),
            0,
            "failed charge must not change counter"
        );
    }

    #[test]
    fn quota_remaining_unknown_returns_minus_one() {
        assert_eq!(remaining("nonexistent_quota_xyz"), -1);
    }

    #[test]
    fn quota_set_wrong_arity_errors() {
        let result = builtin_quota_set(&[Value::String("q".into())]);
        assert!(result.is_err(), "wrong arity must return Err");
    }

    #[test]
    fn quota_charge_wrong_arity_errors() {
        let result = builtin_quota_charge(&[]);
        assert!(result.is_err(), "wrong arity must return Err");
    }

    #[test]
    fn quota_reset_clears_used_count() {
        set("test_reset", 5);
        charge("test_reset", 3);
        let prior = match builtin_quota_reset(&[Value::String("test_reset".into())]).unwrap() {
            Value::Int(n) => n,
            _ => panic!("expected Int"),
        };
        assert_eq!(prior, 3, "reset must return prior used count");
        assert_eq!(
            remaining("test_reset"),
            5,
            "after reset, remaining == limit"
        );
    }
}
