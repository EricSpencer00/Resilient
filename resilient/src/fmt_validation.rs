//! Feature 51/50 — Compile-Time Format String Validation.
//!
//! Walks every `format(template, args...)` call site and validates:
//! - The template's placeholder count matches the supplied argument count
//! - Each argument's type is compatible with its format specifier (RES-3789)
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
///
/// RES-2248: returns `Cow<str>` so the StringLiteral arm (the common
/// case — `format("hello {}", x)` style) can borrow directly from the
/// AST instead of cloning the template into a fresh `String`. The
/// InterpolatedString arm still allocates (we need to concatenate the
/// literal parts), but `Cow::Owned` wraps the produced `String`
/// transparently for the caller. The caller's only consumer is
/// `parse_template(&tmpl)`, which takes `&str` — works identically
/// for both `Cow::Borrowed` and `Cow::Owned` via `Deref`.
fn static_template(node: &Node) -> Option<std::borrow::Cow<'_, str>> {
    match node {
        Node::StringLiteral { value, .. } => Some(std::borrow::Cow::Borrowed(value.as_str())),
        // RES-2612: interned strings can be validated as templates.
        Node::StringInternLiteral { content, .. } => {
            Some(std::borrow::Cow::Borrowed(content.as_str()))
        }
        Node::InterpolatedString { parts, .. } => {
            let mut buf = String::new();
            for p in parts {
                match p {
                    crate::string_interp::StringPart::Literal(s) => buf.push_str(s),
                    crate::string_interp::StringPart::Expr(_) => return None,
                }
            }
            Some(std::borrow::Cow::Owned(buf))
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
                                let (got, args_to_check) = if arguments.len() == 2 {
                                    if let Node::ArrayLiteral { items, .. } = &arguments[1] {
                                        (items.len(), items.clone())
                                    } else {
                                        (arguments.len() - 1, arguments[1..].to_vec())
                                    }
                                } else {
                                    (arguments.len() - 1, arguments[1..].to_vec())
                                };
                                if got != need {
                                    errs.push(format!(
                                        "in `{}`: format string has {} placeholder(s) but {} arg(s) were passed",
                                        fn_name, need, got
                                    ));
                                } else {
                                    // RES-3789: validate each argument against its format specifier
                                    let mut arg_idx = 0;
                                    for seg in &segs {
                                        if let crate::format_builtin::FormatSegment::Placeholder(
                                            spec,
                                        ) = seg
                                        {
                                            if arg_idx < args_to_check.len() {
                                                let arg = &args_to_check[arg_idx];
                                                let arg_kind =
                                                    crate::format_builtin::infer_arg_kind(arg);
                                                if arg_kind != crate::format_builtin::FormatArgumentKind::Unknown {
                                                    let requires_int = crate::format_builtin::spec_requires_integer(spec);
                                                    let requires_float = crate::format_builtin::spec_requires_float(spec);

                                                    let type_err = match (arg_kind, requires_int, requires_float) {
                                                        (crate::format_builtin::FormatArgumentKind::String, true, _) => {
                                                            Some(format!("in `{}`: format specifier `{{{}}}` requires integer argument, got string", fn_name, spec))
                                                        }
                                                        (crate::format_builtin::FormatArgumentKind::String, false, true) => {
                                                            Some(format!("in `{}`: format specifier `{{{}}}` requires float argument, got string", fn_name, spec))
                                                        }
                                                        (crate::format_builtin::FormatArgumentKind::Boolean, true, _) => {
                                                            Some(format!("in `{}`: format specifier `{{{}}}` requires integer argument, got boolean", fn_name, spec))
                                                        }
                                                        (crate::format_builtin::FormatArgumentKind::Boolean, false, true) => {
                                                            Some(format!("in `{}`: format specifier `{{{}}}` requires float argument, got boolean", fn_name, spec))
                                                        }
                                                        (crate::format_builtin::FormatArgumentKind::Integer, false, true) => {
                                                            Some(format!("in `{}`: format specifier `{{{}}}` requires float argument, got integer", fn_name, spec))
                                                        }
                                                        (crate::format_builtin::FormatArgumentKind::Float, true, false) => {
                                                            Some(format!("in `{}`: format specifier `{{{}}}` requires integer argument, got float", fn_name, spec))
                                                        }
                                                        _ => None,
                                                    };
                                                    if let Some(err) = type_err {
                                                        errs.push(err);
                                                    }
                                                }
                                            }
                                            arg_idx += 1;
                                        }
                                    }
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

    #[test]
    fn format_specifier_type_mismatch_string_to_int_errors() {
        // RES-3789: format("{:d}", "text") should error (string literal to :d specifier)
        let src = r#"fn f() { format("{:d}", "text"); return 0; }"#;
        let (prog, _) = parse(src);
        let errs = analyze(&prog);
        assert!(
            !errs.is_empty(),
            "string literal to :d specifier should error"
        );
        assert!(
            errs[0].contains(":d") && errs[0].contains("integer") && errs[0].contains("string"),
            "error should mention :d specifier, integer requirement, and string type: {}",
            errs[0]
        );
    }

    #[test]
    fn format_specifier_type_mismatch_int_to_float_errors() {
        // RES-3789: format("{:.2f}", 1) should error (integer literal to float specifier)
        let src = r#"fn f() { format("{:.2f}", 1); return 0; }"#;
        let (prog, _) = parse(src);
        let errs = analyze(&prog);
        assert!(
            !errs.is_empty(),
            "integer literal to :.2f specifier should error"
        );
        assert!(
            errs[0].contains(".2f") && errs[0].contains("float") && errs[0].contains("integer"),
            "error should mention .2f specifier, float requirement, and integer type: {}",
            errs[0]
        );
    }

    #[test]
    fn format_specifier_valid_int_to_int_ok() {
        // format("{:d}", 42) should pass (integer literal to :d specifier)
        let src = r#"fn f() { format("{:d}", 42); return 0; }"#;
        let (prog, _) = parse(src);
        assert!(
            analyze(&prog).is_empty(),
            "integer literal to :d specifier should pass"
        );
    }

    #[test]
    fn format_specifier_valid_float_to_float_ok() {
        // format("{:.2f}", 3.14) should pass (float literal to :.2f specifier)
        let src = r#"fn f() { format("{:.2f}", 3.14); return 0; }"#;
        let (prog, _) = parse(src);
        assert!(
            analyze(&prog).is_empty(),
            "float literal to :.2f specifier should pass"
        );
    }

    #[test]
    fn format_specifier_string_to_default_ok() {
        // format("{}", "text") should pass (no specifier)
        let src = r#"fn f() { format("{}", "text"); return 0; }"#;
        let (prog, _) = parse(src);
        assert!(
            analyze(&prog).is_empty(),
            "string literal to default specifier should pass"
        );
    }

    #[test]
    fn format_specifier_type_mismatch_array_convention() {
        // RES-3789: format("{:d}", ["text"]) should error (string literal in array to :d specifier)
        let src = r#"fn f() { format("{:d}", ["text"]); return 0; }"#;
        let (prog, _) = parse(src);
        let errs = analyze(&prog);
        assert!(
            !errs.is_empty(),
            "string literal in array to :d specifier should error"
        );
        assert!(
            errs[0].contains(":d") && errs[0].contains("integer") && errs[0].contains("string"),
            "error should mention :d specifier and type mismatch: {}",
            errs[0]
        );
    }

    #[test]
    fn format_specifier_bool_to_int_errors() {
        // format("{:x}", true) should error (boolean literal to :x specifier)
        let src = r#"fn f() { format("{:x}", true); return 0; }"#;
        let (prog, _) = parse(src);
        let errs = analyze(&prog);
        assert!(
            !errs.is_empty(),
            "boolean literal to :x specifier should error"
        );
        assert!(
            errs[0].contains(":x") && errs[0].contains("integer") && errs[0].contains("boolean"),
            "error should mention :x specifier and type mismatch: {}",
            errs[0]
        );
    }
}
