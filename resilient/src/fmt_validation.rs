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

/// Extract the compile-time template string from a node.
///
/// Returns `Some(text)` for `StringLiteral` and for
/// `InterpolatedString` where all parts are literals (i.e. the string
/// contained `\{` escapes but no live `{expr}` interpolations). The
/// latter is the common form for `format()` templates, which escape
/// `{` as `\{` to avoid Resilient's string-interpolation syntax.
///
/// Returns `None` for dynamic templates — runtime values that cannot
/// be validated at compile time.
fn static_template(node: &Node) -> Option<String> {
    match node {
        Node::StringLiteral { value, .. } => Some(value.clone()),
        Node::InterpolatedString { parts, .. } => {
            let mut buf = String::new();
            for p in parts {
                match p {
                    crate::string_interp::StringPart::Literal(s) => buf.push_str(s),
                    crate::string_interp::StringPart::Expr(_) => return None,
                }
            }
            Some(buf)
        }
        _ => None,
    }
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
                    if let Some(tmpl) = static_template(&arguments[0]) {
                        match crate::format_builtin::parse_template(&tmpl) {
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
                                // The runtime `format(template, args)` signature
                                // accepts EITHER individual positional args or a
                                // single array literal.  Check both conventions:
                                //  - Array arg:  `format("t", [a, b])` → count array items
                                //  - Individual: `format("t", a, b)`   → count extra args
                                let got = if arguments.len() == 2 {
                                    if let Node::ArrayLiteral { items, .. } = &arguments[1] {
                                        items.len()
                                    } else {
                                        arguments.len() - 1
                                    }
                                } else {
                                    arguments.len() - 1
                                };
                                if got != need {
                                    errs.push(format!(
                                        "in `{}`: format string has {} placeholder(s) but {} arg(s) were passed",
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
    // RES-1284 / RES-1917: the typechecker gates this call behind
    // `markers.call_idents.contains("format")`, so the program is
    // guaranteed to contain at least one `format` call. The previous
    // `any_node` pre-scan was redundant — removed.
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

    #[test]
    fn array_convention_single_arg_ok() {
        // format("tmpl {}", [42]) — one placeholder, one-element array → OK
        let src = r#"fn f(int x) { format("val: {}", [x]); return 0; }"#;
        let (prog, _) = parse(src);
        assert!(
            analyze(&prog).is_empty(),
            "single-placeholder array call should pass"
        );
    }

    #[test]
    fn array_convention_two_placeholders_two_elems_ok() {
        // format("{}, {}", [a, b]) — 2 placeholders, 2-element array → OK
        let src = r#"fn f(int x, int y) { format("{}, {}", [x, y]); return 0; }"#;
        let (prog, _) = parse(src);
        assert!(
            analyze(&prog).is_empty(),
            "two-placeholder two-element array call should pass"
        );
    }

    #[test]
    fn array_convention_count_mismatch_errors() {
        // format("{}, {}", [x]) — 2 placeholders, 1-element array → error
        let src = r#"fn f(int x) { format("{}, {}", [x]); return 0; }"#;
        let (prog, _) = parse(src);
        let errs = analyze(&prog);
        assert!(!errs.is_empty(), "expected mismatch error");
        assert!(
            errs[0].contains("2") && errs[0].contains("1"),
            "error must mention placeholder count and arg count: {}",
            errs[0]
        );
    }

    #[test]
    fn interp_string_template_is_checked() {
        // Template uses \{ escape — the parser produces InterpolatedString
        // (all-literal parts). The validator must still check placeholder count.
        // String `"val: \{}"` in source → InterpolatedString with Literal "val: {}".
        // One placeholder, but zero individual non-array args → mismatch.
        let src = r#"fn f(int x) { format("val: \{}", [x, x]); return 0; }"#;
        let (prog, _) = parse(src);
        let errs = analyze(&prog);
        assert!(
            !errs.is_empty(),
            "InterpolatedString template with too many array args should error; \
             errs: {:?}",
            errs
        );
    }
}
