//! Feature 51/50 — Compile-Time Format String Validation.
//!
//! Walks every `format(template, args...)` call site and validates
//! that the template's placeholder count matches the supplied
//! argument count. Emits an error for mismatches.
//!
//! Builds on `crate::format_builtin::parse_template` so the
//! validation engine and runtime parser stay in lock-step.
//!
//! RES-1101: when `parse_template` reports an unterminated `{`
//! placeholder (RES-1093), the validator surfaces that error
//! directly so malformed templates are caught at compile time
//! instead of producing plausible-looking runtime output.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;

/// Returns the placeholder count, or `None` if the template is
/// malformed (e.g., unterminated `{`).
pub fn count_placeholders(template: &str) -> Option<usize> {
    crate::format_builtin::parse_template(template)
        .ok()
        .map(|segs| {
            segs.iter()
                .filter(|s| matches!(s, crate::format_builtin::FormatSegment::Placeholder(_)))
                .count()
        })
}

pub fn analyze(program: &Node) -> Vec<String> {
    let mut errs = Vec::new();
    let Node::Program(stmts) = program else {
        return errs;
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            walk(body, name, &mut errs);
        }
    }
    errs
}

fn walk(node: &Node, fn_name: &str, errs: &mut Vec<String>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name: callee, .. } = function.as_ref() {
                if callee == "format" && !arguments.is_empty() {
                    if let Node::StringLiteral { value, .. } = &arguments[0] {
                        match crate::format_builtin::parse_template(value) {
                            Err(e) => {
                                // RES-1101: surface the unterminated `{`
                                // diagnostic directly.
                                errs.push(format!("in `{}`: {}", fn_name, e));
                            }
                            Ok(segs) => {
                                let need = segs
                                    .iter()
                                    .filter(|s| {
                                        matches!(
                                            s,
                                            crate::format_builtin::FormatSegment::Placeholder(_)
                                        )
                                    })
                                    .count();
                                let got = arguments.len() - 1;
                                if got != need {
                                    errs.push(format!(
                                        "in `{}`: format string has {} placeholders but {} args were passed",
                                        fn_name, need, got
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            for a in arguments {
                walk(a, fn_name, errs);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk(s, fn_name, errs);
            }
        }
        Node::ReturnStatement { value: Some(e), .. } => walk(e, fn_name, errs),
        Node::LetStatement { value, .. } | Node::Assignment { value, .. } => {
            walk(value, fn_name, errs)
        }
        Node::ExpressionStatement { expr, .. } => walk(expr, fn_name, errs),
        _ => {}
    }
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    // RES-1284: fast-reject. `analyze` walks every function body
    // looking for `CallExpression` to the `format` builtin with a
    // `StringLiteral` first arg. Without any `format` call anywhere
    // in the program — the overwhelming majority of `cargo test`
    // inputs and every fixture in `examples/` that doesn't use
    // `format(...)` — the analyser returns an empty Vec. Pre-scan
    // once with the early-terminating `any_node` (RES-1238) and skip
    // the analysis entirely when no `format` call exists.
    let has_format_call = crate::uniqueness_walk::any_node(program, |n| match n {
        Node::CallExpression { function, .. } => match function.as_ref() {
            Node::Identifier { name, .. } => name == "format",
            _ => false,
        },
        _ => false,
    });
    if !has_format_call {
        return Ok(());
    }
    let errs = analyze(program);
    if !errs.is_empty() {
        return Err(format!("{}:0:0: error: {}", source_path, errs[0]));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn matching_placeholder_and_arg_count() {
        let src = r#"fn f(int x) { format("hello {}", x); return 0; }"#;
        let (prog, _) = parse(src);
        assert!(analyze(&prog).is_empty());
    }

    #[test]
    fn mismatched_count_errors() {
        let src = r#"fn f(int x) { format("hello {}", x, x, x); return 0; }"#;
        let (prog, _) = parse(src);
        assert!(!analyze(&prog).is_empty());
    }

    /// RES-1101: an unterminated `{` placeholder surfaces as a
    /// compile-time error, not a silently-accepted call.
    #[test]
    fn unterminated_brace_in_template_errors() {
        let src = r#"fn f(int x) { format("hello {", x); return 0; }"#;
        let (prog, _) = parse(src);
        let errs = analyze(&prog);
        assert!(!errs.is_empty(), "expected an error");
        assert!(
            errs[0].contains("unterminated"),
            "expected unterminated diagnostic, got: {}",
            errs[0]
        );
    }
}
