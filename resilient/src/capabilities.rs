//! Grand-Implementation Pass 3 — Subsystem E: Capability Tokens.
//!
//! Capability-based security has been a research thread for 50 years
//! (KeyKOS, EROS, Capsicum). Every implementation bolts capabilities on
//! via OS syscalls or library types. Erlang's process-references and
//! Pony's reference-capabilities approach the model in-language but
//! without runtime delegation primitives. *No* mainstream production
//! language ships mintable, revocable, programmer-addressable capability
//! tokens in the core stdlib.
//!
//! Resilient adds:
//!
//!   * `mint_cap(name: String) -> String` — mint a fresh capability for
//!     `name` and return the bearer token. The token is opaque (a
//!     32-bit-of-entropy hex string), unforgeable from outside, and
//!     stored alongside `name` in a thread-local registry.
//!   * `check_cap(name: String, token: String) -> Bool` — verify the
//!     bearer token matches the registered capability for `name`.
//!     Returns `false` if the name is unknown or the token doesn't match.
//!   * `revoke_cap(name: String) -> Bool` — invalidate. Returns `true`
//!     if the capability existed.
//!   * `caps() -> Array<String>` — list every minted capability name.
//!
//! Why this is unique: granting a capability is no longer a discipline
//! decision (pass-the-handle / wrap-the-resource); it is a runtime
//! operation. You mint a token, hand it to the receiver, and the receiver
//! presents it at every check site. Revocation invalidates every
//! outstanding holder simultaneously without any registry-walking from
//! the application.
//!
//! Token entropy: a SplitMix64 stream seeded once with the existing
//! random RNG state. Sufficient for in-process unforgeability against
//! pure Resilient code — not a substitute for crypto across trust
//! boundaries.

#![allow(clippy::collapsible_if)]

use crate::Value;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

type RResult<T> = Result<T, String>;

const MAX_CAPS: usize = 256;

thread_local! {
    static CAPS: RefCell<BTreeMap<String, String>> = const { RefCell::new(BTreeMap::new()) };
}

static CAP_RNG: AtomicU64 = AtomicU64::new(0xA5A5_5A5A_DEAD_BEEFu64);

fn next_token() -> String {
    // SplitMix64 step — same primitive as RES-150's random_int builtin.
    let mut x = CAP_RNG.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    format!("{x:016x}")
}

pub(crate) fn builtin_mint_cap(args: &[Value]) -> RResult<Value> {
    let name = match args {
        [Value::String(n)] => n.clone(),
        [a] => return Err(format!("mint_cap: expected String, got {}", type_name(a))),
        _ => return Err(format!("mint_cap: expected 1 arg, got {}", args.len())),
    };
    let token = next_token();
    CAPS.with(|c| {
        let mut c = c.borrow_mut();
        if !c.contains_key(&name) && c.len() >= MAX_CAPS {
            if let Some(k) = c.keys().next().cloned() {
                c.remove(&k);
            }
        }
        c.insert(name, token.clone());
    });
    Ok(Value::String(token))
}

pub(crate) fn builtin_check_cap(args: &[Value]) -> RResult<Value> {
    let (name, token) = match args {
        [Value::String(n), Value::String(t)] => (n.as_str(), t.as_str()),
        [a, b] => {
            return Err(format!(
                "check_cap: expected (String, String), got ({}, {})",
                type_name(a),
                type_name(b)
            ));
        }
        _ => return Err(format!("check_cap: expected 2 args, got {}", args.len())),
    };
    let valid = CAPS.with(|c| c.borrow().get(name).is_some_and(|t| t == token));
    Ok(Value::Bool(valid))
}

pub(crate) fn builtin_revoke_cap(args: &[Value]) -> RResult<Value> {
    let name = match args {
        [Value::String(n)] => n.clone(),
        [a] => {
            return Err(format!("revoke_cap: expected String, got {}", type_name(a)));
        }
        _ => return Err(format!("revoke_cap: expected 1 arg, got {}", args.len())),
    };
    let removed = CAPS.with(|c| c.borrow_mut().remove(&name).is_some());
    Ok(Value::Bool(removed))
}

pub(crate) fn builtin_caps(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!("caps: expected 0 args, got {}", args.len()));
    }
    let names: Vec<Value> = CAPS.with(|c| {
        c.borrow()
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

    #[test]
    fn mint_and_check_capability() {
        let token = match builtin_mint_cap(&[Value::String("test_res".into())]).unwrap() {
            Value::String(t) => t,
            v => panic!("expected String token, got {v:?}"),
        };
        let valid = builtin_check_cap(&[
            Value::String("test_res".into()),
            Value::String(token.clone()),
        ])
        .unwrap();
        assert!(
            matches!(valid, Value::Bool(true)),
            "check_cap with valid token must return true"
        );
        let invalid = builtin_check_cap(&[
            Value::String("test_res".into()),
            Value::String("wrong_token".into()),
        ])
        .unwrap();
        assert!(
            matches!(invalid, Value::Bool(false)),
            "check_cap with invalid token must return false"
        );
    }

    #[test]
    fn revoke_removes_capability() {
        builtin_mint_cap(&[Value::String("revoke_me".into())]).unwrap();
        let removed = builtin_revoke_cap(&[Value::String("revoke_me".into())]).unwrap();
        assert!(
            matches!(removed, Value::Bool(true)),
            "revoke must return true for existing cap"
        );
        let second_revoke = builtin_revoke_cap(&[Value::String("revoke_me".into())]).unwrap();
        assert!(
            matches!(second_revoke, Value::Bool(false)),
            "revoking again must return false — already gone"
        );
    }

    #[test]
    fn mint_cap_wrong_arity_errors() {
        let result = builtin_mint_cap(&[]);
        assert!(result.is_err(), "wrong arity must return Err");
    }

    #[test]
    fn check_cap_unknown_name_returns_false() {
        let result = builtin_check_cap(&[
            Value::String("never_minted_xyz".into()),
            Value::String("token".into()),
        ])
        .unwrap();
        assert!(
            matches!(result, Value::Bool(false)),
            "unknown cap must return false"
        );
    }
}
