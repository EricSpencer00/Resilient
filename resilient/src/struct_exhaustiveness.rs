//! Feature 49/50 — Pattern Exhaustiveness for Structs.
//!
//! When `match` arms destructure a struct (`StructName { field1, field2 }`),
//! the analyzer verifies that every reachable variant of the struct's
//! field domain is covered. Initial coverage: bool fields (must
//! cover both true and false) and integer fields with explicit
//! literal patterns (must include a wildcard arm).
//!
//! A match is considered non-exhaustive when ALL of these hold:
//!   1. Every arm uses a `Pattern::Struct` destructure.
//!   2. No arm is an unguarded "catch-all": a wildcard `_`, an
//!      identifier binding, a struct pattern with `..` (`has_rest`),
//!      or a struct pattern whose every field sub-pattern is a wildcard
//!      or identifier (both always succeed without constraint).

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;

#[derive(Debug, Clone)]
pub struct ExhaustivenessWarning {
    pub function: String,
    /// RES-2022: `&'static str` because the sole push site populates
    /// this from a string literal. The previous `String` shape forced
    /// a `.into()` allocation per push for content that already lived
    /// in `.rodata`. Sibling fix to RES-2020 for `coverage_warnings`.
    pub message: &'static str,
}

/// Returns true if the sub-pattern inside a struct field binding
/// cannot fail (i.e., it always matches any value).
fn is_irrefutable_sub_pattern(p: &crate::Pattern) -> bool {
    matches!(p, crate::Pattern::Wildcard | crate::Pattern::Identifier(_))
}

/// Returns true if `pattern` is an unguarded catch-all arm — one that
/// matches any struct value without constraint.
fn struct_arm_is_unguarded_catch_all(
    pattern: &crate::Pattern,
    guard: &Option<crate::Node>,
) -> bool {
    if guard.is_some() {
        return false;
    }
    match pattern {
        crate::Pattern::Wildcard | crate::Pattern::Identifier(_) => true,
        crate::Pattern::Struct {
            fields, has_rest, ..
        } => *has_rest || fields.iter().all(|(_, fp)| is_irrefutable_sub_pattern(fp)),
        _ => false,
    }
}

pub fn analyze(program: &Node) -> Vec<ExhaustivenessWarning> {
    let mut out = Vec::new();
    let Node::Program(stmts) = program else {
        return out;
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            walk(body, name, &mut out);
        }
    }
    out
}

fn walk(node: &Node, fn_name: &str, out: &mut Vec<ExhaustivenessWarning>) {
    match node {
        Node::Match { arms, .. } => {
            let all_struct = arms
                .iter()
                .all(|(p, _, _)| matches!(p, crate::Pattern::Struct { .. }));
            let has_cover = arms
                .iter()
                .any(|(p, g, _)| struct_arm_is_unguarded_catch_all(p, g));
            if all_struct && !arms.is_empty() && !has_cover {
                out.push(ExhaustivenessWarning {
                    function: fn_name.to_string(),
                    message: "Non-exhaustive match on struct — add a wildcard arm \
                              (`_`, an identifier, or `StructName { .. }`)",
                });
            }
            for (_, _, body) in arms {
                walk(body, fn_name, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk(s, fn_name, out);
            }
        }
        Node::ExpressionStatement { expr, .. } => walk(expr, fn_name, out),
        Node::LetStatement { value, .. } => walk(value, fn_name, out),
        Node::ReturnStatement { value: Some(v), .. } => walk(v, fn_name, out),
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            walk(consequence, fn_name, out);
            if let Some(a) = alternative {
                walk(a, fn_name, out);
            }
        }
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            walk(body, fn_name, out);
        }
        _ => {}
    }
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let warnings = analyze(program);
    if warnings.is_empty() {
        return Ok(());
    }
    let w = &warnings[0];
    Err(format!(
        "{}: error: in fn `{}`: {}",
        source_path, w.function, w.message
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_program_no_warnings() {
        let p = Node::Program(vec![]);
        assert!(analyze(&p).is_empty());
    }

    #[test]
    fn check_always_returns_ok_without_match() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn function_without_struct_no_warnings() {
        let src = "fn g(int x) -> int { return x * 2; }\n";
        let (prog, _) = crate::parse(src);
        assert!(analyze(&prog).is_empty());
    }

    /// A struct match where ALL arms are specific literal-field patterns
    /// and no arm is a catch-all is non-exhaustive — analysis fires a warning.
    #[test]
    fn analyze_detects_missing_catch_all_arm() {
        let src = r#"
struct Point { int x, int y }
fn locate(Point p) -> int {
    return match p {
        Point { x: 0, y: 0 } => 0,
        Point { x: 1, y: 1 } => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let warnings = analyze(&prog);
        assert!(
            !warnings.is_empty(),
            "expected at least one exhaustiveness warning"
        );
        assert!(
            warnings[0].message.contains("Non-exhaustive"),
            "warning must mention Non-exhaustive: {}",
            warnings[0].message
        );
    }

    /// A struct match with `StructName { .. }` (`has_rest = true`) as its
    /// last arm is exhaustive — no warning should fire.
    #[test]
    fn analyze_ok_when_rest_arm_is_present() {
        let src = r#"
struct Point { int x, int y }
fn locate(Point p) -> int {
    return match p {
        Point { x: 0, y: 0 } => 0,
        Point { .. } => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let warnings = analyze(&prog);
        assert!(
            warnings.is_empty(),
            "expected no warnings for match with `..` catch-all; got: {:?}",
            warnings
        );
    }

    /// A struct match where the last arm binds every field as an identifier
    /// (no constraint) is exhaustive — no warning should fire.
    #[test]
    fn analyze_ok_when_identifier_catch_all_arm_is_present() {
        let src = r#"
struct Point { int x, int y }
fn locate(Point p) -> int {
    return match p {
        Point { x: 0, y: 0 } => 0,
        Point { x, y } => x + y,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let warnings = analyze(&prog);
        assert!(
            warnings.is_empty(),
            "expected no warnings for match with identifier catch-all arm; got: {:?}",
            warnings
        );
    }

    /// `check()` must surface the diagnostic as an error for programs
    /// with non-exhaustive struct matches.
    #[test]
    fn check_errors_on_nonexhaustive_struct_match() {
        let src = r#"
struct Event { int code }
fn handle(Event e) -> int {
    return match e {
        Event { code: 0 } => 0,
        Event { code: 1 } => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let result = check(&prog, "test.rz");
        assert!(
            result.is_err(),
            "expected check to fail for non-exhaustive struct match"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("Non-exhaustive match on struct"),
            "error must contain 'Non-exhaustive match on struct': {msg}"
        );
    }

    /// `check()` returns `Ok` for a match with an identifier-catch-all arm.
    #[test]
    fn check_ok_for_exhaustive_struct_match() {
        let src = r#"
struct Event { int code }
fn handle(Event e) -> int {
    return match e {
        Event { code: 0 } => 0,
        Event { code } => code,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test.rz").is_ok());
    }
}
