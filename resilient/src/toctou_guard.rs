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
use crate::uniqueness_walk::{for_each_function, visit};

const CHECK_SUFFIXES: &[&str] = &["_exists", "_is_valid", "_status", "_check"];
const USE_SUFFIXES: &[&str] = &["_open", "_read", "_write", "_delete", "_unlink", "_chmod"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1266 / RES-1917: the typechecker gates this call behind
    // `markers.any_call_ident_with_suffix` with the same CHECK_SUFFIXES.
    // The previous `any_node` pre-scan was redundant — removed.
    for_each_function(program, |fname, _params, body| {
        // RES-2160: borrow fn_name + first-ident-arg directly from the
        // body AST. The events vector lives only inside this callback,
        // and `visit<'a>` already threads the AST lifetime through the
        // closure call. Previously the loop cloned `name` for every
        // events push and cloned `name`/`value` from the first argument
        // — four `String::clone()` per matching CallExpression, all
        // thrown away at the end of the callback iteration.
        let mut events: Vec<(bool, &str, Option<&str>)> = Vec::new();
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
                        Node::Identifier { name, .. } => Some(name.as_str()),
                        Node::StringLiteral { value, .. } => Some(value.as_str()),
                        _ => None,
                    });
                    if CHECK_SUFFIXES.iter().any(|s| name.ends_with(*s)) {
                        events.push((true, name.as_str(), arg));
                    } else if USE_SUFFIXES.iter().any(|s| name.ends_with(*s))
                        && !name.starts_with("atomic_")
                    {
                        events.push((false, name.as_str(), arg));
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
                if *uarg == Some(*carg) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_program_returns_ok() {
        let (prog, _) = crate::parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn program_without_check_call_returns_ok() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn check_suffixes_include_exists() {
        assert!(CHECK_SUFFIXES.contains(&"_exists"));
        assert!(USE_SUFFIXES.contains(&"_open"));
    }
}
