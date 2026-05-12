//! Ralph-Loop Uniqueness #25 — TOCTOU (time-of-check-to-time-of-use) guard.
//!
//! Filesystem and resource APIs across Unix/Windows are riddled with
//! TOCTOU races: `if (file_exists(p)) { open(p) }` is broken because
//! something else can replace the file between the two calls. Static
//! analyzers like `cppcheck` and `splint` look for narrow patterns;
//! Rust prevents some via the borrow checker; no language *requires*
//! that any check-then-use pair on the same file/key be replaced with
//! an atomic operation.
//!
//! Resilient detects the canonical TOCTOU pattern: a function body that
//! contains, in textual order, a call to a `*_exists`/`*_is_valid`/
//! `*_status` "check" function with a literal-or-identifier argument,
//! followed later by a call to a "use" function (`*_open`/`*_read`/
//! `*_write`/`*_delete`) with the same argument, *without* the use
//! call's name being prefixed `atomic_` (an opt-in for atomic APIs).

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::{any_node, for_each_function, visit};

const CHECK_SUFFIXES: &[&str] = &["_exists", "_is_valid", "_status", "_check"];
const USE_SUFFIXES: &[&str] = &["_open", "_read", "_write", "_delete", "_unlink", "_chmod"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1266: fast-reject. The TOCTOU detector pairs a `*_exists` /
    // `*_is_valid` / `*_status` / `*_check` call with a later non-atomic
    // `*_open` / `*_read` / … on the same argument. Without a *check*
    // call in the function, there can never be a pair, so the per-function
    // event collection is dead work for the overwhelming majority of
    // programs (every fixture in `examples/`, every test). Pre-scan once
    // for any check-suffixed CallExpression via the early-terminating
    // `any_node` (RES-1238) and skip the loop entirely when none exist.
    let has_check_call = any_node(program, |n| match n {
        Node::CallExpression { function, .. } => match function.as_ref() {
            Node::Identifier { name, .. } => CHECK_SUFFIXES.iter().any(|s| name.ends_with(*s)),
            _ => false,
        },
        _ => false,
    });
    if !has_check_call {
        return Ok(());
    }
    for_each_function(program, |fname, _params, body| {
        let mut events: Vec<(bool, String, Option<String>)> = Vec::new();
        // (is_check, fn_name, first_ident_arg)
        visit(body, &mut |n| {
            if let Node::CallExpression {
                function,
                arguments,
                ..
            } = n
            {
                if let Node::Identifier { name, .. } = function.as_ref() {
                    let arg = arguments.first().and_then(|a| match a {
                        Node::Identifier { name, .. } => Some(name.clone()),
                        Node::StringLiteral { value, .. } => Some(value.clone()),
                        _ => None,
                    });
                    if CHECK_SUFFIXES.iter().any(|s| name.ends_with(*s)) {
                        events.push((true, name.clone(), arg));
                    } else if USE_SUFFIXES.iter().any(|s| name.ends_with(*s))
                        && !name.starts_with("atomic_")
                    {
                        events.push((false, name.clone(), arg));
                    }
                }
            }
        });
        // Find pairs: a check followed later by a non-atomic use on same arg.
        for (i, (is_check, cname, carg)) in events.iter().enumerate() {
            if !*is_check {
                continue;
            }
            let Some(carg) = carg else { continue };
            for (is_check2, uname, uarg) in events.iter().skip(i + 1) {
                if *is_check2 {
                    continue;
                }
                if uarg.as_deref() == Some(carg) {
                    eprintln!(
                        "warning: in '{fname}', TOCTOU: '{cname}({carg})' followed \
                         by '{uname}({carg})' — use atomic_{uname} or wrap both \
                         in a single-step API"
                    );
                    break;
                }
            }
        }
    });
    Ok(())
}
