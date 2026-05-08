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
use crate::uniqueness_walk::{any_node, for_each_function};

const SECRET_NAME_PREFIXES: &[&str] = &["secret_", "key_", "priv_", "password", "nonce_"];
const SECRET_TYPE_PREFIXES: &[&str] = &["Secret", "&Secret", "&mut Secret"];
const WIPE_FNS: &[&str] = &["zeroize", "zero_out", "wipe", "scrub"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    for_each_function(program, |fname, _params, body| {
        let mut leaks = Vec::new();
        collect_local_secrets(body, &mut leaks);
        for var in leaks {
            if !is_wiped(body, &var) && !is_returned(body, &var) {
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

fn collect_local_secrets(body: &Node, out: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    let mut sink = |n: &Node| {
        let (name, type_annot) = match n {
            Node::LetStatement {
                name, type_annot, ..
            } => (name, type_annot.as_deref()),
            Node::StaticLet { name, .. } => (name, None),
            _ => return,
        };
        if !seen.insert(name.clone()) {
            return;
        }
        let by_name = SECRET_NAME_PREFIXES.iter().any(|p| name.starts_with(*p));
        let by_type = type_annot.map(is_secret_type).unwrap_or(false);
        if by_name || by_type {
            out.push(name.clone());
        }
    };
    crate::uniqueness_walk::visit(body, &mut sink);
}

fn is_secret_type(ty: &str) -> bool {
    SECRET_TYPE_PREFIXES.iter().any(|p| ty.starts_with(*p))
}

fn is_wiped(body: &Node, var: &str) -> bool {
    any_node(body, |n| match n {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => match function.as_ref() {
            Node::Identifier { name, .. } if WIPE_FNS.contains(&name.as_str()) => arguments
                .iter()
                .any(|a| matches!(a, Node::Identifier { name, .. } if name == var)),
            _ => false,
        },
        _ => false,
    })
}

fn is_returned(body: &Node, var: &str) -> bool {
    any_node(body, |n| match n {
        Node::ReturnStatement { value: Some(v), .. } => contains_ident(v, var),
        _ => false,
    })
}

fn contains_ident(node: &Node, var: &str) -> bool {
    any_node(
        node,
        |n| matches!(n, Node::Identifier { name, .. } if name == var),
    )
}
