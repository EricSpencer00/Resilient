//! Ralph-Loop Uniqueness #3 — secret erasure on function exit.
//!
//! Cryptographic key material in C/C++/Rust often lingers in stack memory
//! past its useful life because the compiler legitimately optimizes away
//! "dead" stores to it. The standard mitigations — `volatile_zero`,
//! `OPENSSL_cleanse`, `zeroize::Zeroize` — are crate-level conventions, not
//! language rules, and zero language *requires* them.
//!
//! Resilient encodes erasure as a static contract on identifier shape:
//!
//!   - Any local `let` binding whose name begins with `secret_`, `key_`,
//!     `priv_`, `password`, or `nonce_`, OR whose declared type starts
//!     with `Secret` (`Secret`, `SecretKey`, `Secret<T>`), must be passed
//!     to `zeroize(<name>)` / `zero_out(<name>)` / `wipe(<name>)` somewhere
//!     in the same function body, OR the identifier must reach a `return`
//!     statement (i.e., it leaves the frame as an output, not as garbage).
//!   - A function that holds a secret which never reaches a wipe call
//!     and is never returned emits a warning.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::{any_node, for_each_function, visit};
use std::collections::HashSet;

const SECRET_NAME_PREFIXES: &[&str] = &["secret_", "key_", "priv_", "password", "nonce_"];
const SECRET_TYPE_PREFIXES: &[&str] = &["Secret", "&Secret", "&mut Secret"];
const WIPE_FNS: &[&str] = &["zeroize", "zero_out", "wipe", "scrub"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1256: fast-reject. `scan_body` walks each function body in
    // full; for programs without any secret-prefixed binding the walks
    // produce no diagnostic but still touch every node. Pre-scan the
    // program once via `any_node` (RES-1238 made this early-terminating)
    // and skip the pass entirely when no secret-prefixed binding exists.
    let has_secret = any_node(program, |n| match n {
        Node::LetStatement {
            name, type_annot, ..
        } => {
            SECRET_NAME_PREFIXES.iter().any(|p| name.starts_with(*p))
                || type_annot.as_deref().map(is_secret_type).unwrap_or(false)
        }
        Node::StaticLet { name, .. } => SECRET_NAME_PREFIXES.iter().any(|p| name.starts_with(*p)),
        _ => false,
    });
    if !has_secret {
        return Ok(());
    }
    for_each_function(program, |fname, _params, body| {
        // RES-1970: single-pass scan. Previously this loop did one walk
        // per `collect_local_secrets` plus *two more* whole-body walks
        // per secret (one for `is_wiped`, one for `is_returned`) — a
        // structural O(k * |body|). Both `is_wiped` and `is_returned`
        // are body-wide membership queries; we can compute their
        // closures once in a single recursive walk, then drop the
        // per-secret loop to O(1) lookups.
        let (secrets, wiped, returned) = scan_body(body);
        for var in &secrets {
            if !wiped.contains(*var) && !returned.contains(*var) {
                eprintln!(
                    "warning: function '{fname}' binds secret '{var}' but never \
                     calls zeroize()/zero_out()/wipe() on it before exit — \
                     key material may persist on the stack"
                );
            }
        }
    });
    Ok(())
}

fn is_secret_type(ty: &str) -> bool {
    SECRET_TYPE_PREFIXES.iter().any(|p| ty.starts_with(*p))
}

/// Single-pass body scan: returns `(secrets, wiped, returned)`.
///
/// * `secrets` — distinct names of `let` / `static let` bindings whose
///   name or type annotation matches the SECRET prefix list.
/// * `wiped` — identifier names that appear as a direct argument to a
///   `zeroize` / `zero_out` / `wipe` / `scrub` call anywhere in the body.
/// * `returned` — identifier names that appear transitively in any
///   `return <expr>` value in the body (i.e., possibly leaving the frame).
///
/// All three borrow from the AST (`&'a str`), so no per-name allocation
/// happens during collection. Names are deduped via `HashSet` for the
/// two membership sets; `secrets` preserves source order via a small
/// dedup-on-push check (k is tiny — typically 1).
fn scan_body<'a>(body: &'a Node) -> (Vec<&'a str>, HashSet<&'a str>, HashSet<&'a str>) {
    let mut secrets: Vec<&'a str> = Vec::new();
    let mut secret_dedup: HashSet<&'a str> = HashSet::new();
    let mut wiped: HashSet<&'a str> = HashSet::new();
    let mut returned: HashSet<&'a str> = HashSet::new();

    // First pass via shared `visit`: collect secret-binding names and
    // wiped argument names. This catches every `let` / `static let`
    // and every `zeroize(x)`-style call regardless of nesting.
    visit(body, &mut |n| match n {
        Node::LetStatement {
            name, type_annot, ..
        } => {
            let by_name = SECRET_NAME_PREFIXES.iter().any(|p| name.starts_with(*p));
            let by_type = type_annot.as_deref().map(is_secret_type).unwrap_or(false);
            if (by_name || by_type) && secret_dedup.insert(name.as_str()) {
                secrets.push(name.as_str());
            }
        }
        Node::StaticLet { name, .. } => {
            let by_name = SECRET_NAME_PREFIXES.iter().any(|p| name.starts_with(*p));
            if by_name && secret_dedup.insert(name.as_str()) {
                secrets.push(name.as_str());
            }
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                if WIPE_FNS.contains(&name.as_str()) {
                    for a in arguments {
                        if let Node::Identifier { name: arg, .. } = a {
                            wiped.insert(arg.as_str());
                        }
                    }
                }
            }
        }
        _ => {}
    });

    // Second pass: for each `ReturnStatement` value, collect *all*
    // identifier names within it. `visit` walks into `ReturnStatement`
    // children automatically; an explicit closure flag would be awkward,
    // so collect the return-value subtree roots first, then walk each.
    visit(body, &mut |n| {
        if let Node::ReturnStatement { value: Some(v), .. } = n {
            visit(v, &mut |inner| {
                if let Node::Identifier { name, .. } = inner {
                    returned.insert(name.as_str());
                }
            });
        }
    });

    (secrets, wiped, returned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn no_secret_binding_skips_check() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn empty_program_returns_ok() {
        let (prog, _) = parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn non_secret_named_binding_skips_check() {
        let src = "fn f() { let plaintext = 42; }\n";
        let (prog, _) = parse(src);
        assert!(
            check(&prog, "test").is_ok(),
            "ordinary bindings must not trigger secret-erasure check"
        );
    }
}
