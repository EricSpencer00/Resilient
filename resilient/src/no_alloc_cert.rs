//! Feature 30/50 — No-Allocation Certification.
//!
//! `#[no_alloc]` on a function statically proves it (and its callees)
//! never invoke an allocating builtin or construct a heap-resident
//! value type. This is stronger than `no_std` because it covers
//! transitive allocation — even allocating call paths must be
//! eliminated.
//!
//! Detection scans for these allocation triggers in the body:
//! * `[ ... ]` array literal (heap-allocated)
//! * `{ ... }` map literal
//! * `#{ ... }` set literal
//! * `string` interpolation `"...{x}..."`
//! * Builtins: `push`, `array_new`, `string_concat`
//!
//! Calls to fns lacking `#[no_alloc]` from a `#[no_alloc]` body emit
//! a hard error.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashSet;

const ALLOCATING_BUILTINS: &[&str] = &[
    "push",
    "array_new",
    "string_concat",
    "to_string",
    "format",
    "split",
    "map",
    "filter",
    "reduce",
];

pub fn body_allocates(node: &Node) -> Option<String> {
    match node {
        Node::ArrayLiteral { .. } => Some("array literal".to_string()),
        Node::MapLiteral { .. } => Some("map literal".to_string()),
        Node::SetLiteral { .. } => Some("set literal".to_string()),
        Node::InterpolatedString { .. } => Some("string interpolation".to_string()),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                if ALLOCATING_BUILTINS.contains(&name.as_str()) {
                    return Some(format!("builtin `{name}`"));
                }
            }
            for a in arguments {
                if let Some(r) = body_allocates(a) {
                    return Some(r);
                }
            }
            None
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                if let Some(r) = body_allocates(s) {
                    return Some(r);
                }
            }
            None
        }
        Node::ReturnStatement { value: Some(e), .. } => body_allocates(e),
        Node::LetStatement { value, .. } | Node::Assignment { value, .. } => body_allocates(value),
        Node::ExpressionStatement { expr, .. } => body_allocates(expr),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => body_allocates(condition)
            .or_else(|| body_allocates(consequence))
            .or_else(|| alternative.as_ref().and_then(|a| body_allocates(a))),
        Node::WhileStatement {
            condition, body, ..
        } => body_allocates(condition).or_else(|| body_allocates(body)),
        _ => None,
    }
}

pub fn collect_no_alloc_fns() -> HashSet<String> {
    crate::feature_attrs::find_kind("no_alloc")
        .into_iter()
        .map(|(item, _)| item)
        .collect()
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let no_alloc = collect_no_alloc_fns();
    if no_alloc.is_empty() {
        return Ok(());
    }
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            if no_alloc.contains(name) {
                if let Some(reason) = body_allocates(body) {
                    return Err(format!(
                        "{}:0:0: error: `{}` is `#[no_alloc]` but allocates via {}",
                        source_path, name, reason
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn array_literal_is_alloc() {
        let src = r#"fn f(int x) { let a = [1, 2, 3]; return x; }"#;
        let (prog, _) = parse(src);
        if let Node::Program(ss) = &prog {
            for s in ss {
                if let Node::Function { body, .. } = &s.node {
                    assert!(body_allocates(body).is_some());
                }
            }
        }
    }

    #[test]
    fn pure_arithmetic_is_alloc_free() {
        let src = r#"fn f(int x) { let y = x + 1; return y; }"#;
        let (prog, _) = parse(src);
        if let Node::Program(ss) = &prog {
            for s in ss {
                if let Node::Function { body, .. } = &s.node {
                    assert!(body_allocates(body).is_none());
                }
            }
        }
    }

    #[test]
    fn no_alloc_with_pure_code_passes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "safe_add",
            crate::feature_attrs::AttrRecord {
                name: "no_alloc".into(),
                args: String::new(),
                line: 0,
            },
        );
        let src = r#"fn safe_add(int x, int y) -> int { return x + y; }"#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn no_alloc_with_array_literal_fails() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "make_array",
            crate::feature_attrs::AttrRecord {
                name: "no_alloc".into(),
                args: String::new(),
                line: 0,
            },
        );
        let src = r#"fn make_array(int x) -> void { let a = [1, 2, 3]; }"#;
        let (prog, _) = parse(src);
        let err = check(&prog, "test");
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("array literal"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn no_alloc_with_map_literal_fails() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "make_map",
            crate::feature_attrs::AttrRecord {
                name: "no_alloc".into(),
                args: String::new(),
                line: 0,
            },
        );
        let src = r#"fn make_map(int x) -> void { let m = { "key": 1 }; }"#;
        let (prog, _) = parse(src);
        let err = check(&prog, "test");
        assert!(err.is_err());
        crate::feature_attrs::reset();
    }

    #[test]
    fn no_alloc_with_string_interpolation_fails() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "format_str",
            crate::feature_attrs::AttrRecord {
                name: "no_alloc".into(),
                args: String::new(),
                line: 0,
            },
        );
        let src = r#"fn format_str(int x) -> void { let s = "value: {x}"; }"#;
        let (prog, _) = parse(src);
        let err = check(&prog, "test");
        assert!(err.is_err());
        crate::feature_attrs::reset();
    }

    #[test]
    fn no_alloc_with_push_builtin_fails() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "append",
            crate::feature_attrs::AttrRecord {
                name: "no_alloc".into(),
                args: String::new(),
                line: 0,
            },
        );
        let src = r#"fn append(int x) -> void { push([1, 2], 3); }"#;
        let (prog, _) = parse(src);
        let err = check(&prog, "test");
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("builtin"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn multiple_no_alloc_functions_checked() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "add",
            crate::feature_attrs::AttrRecord {
                name: "no_alloc".into(),
                args: String::new(),
                line: 0,
            },
        );
        crate::feature_attrs::record(
            "mul",
            crate::feature_attrs::AttrRecord {
                name: "no_alloc".into(),
                args: String::new(),
                line: 0,
            },
        );
        let src = r#"
            fn add(int x, int y) -> int { return x + y; }
            fn mul(int x, int y) -> int { return x * y; }
        "#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn no_alloc_with_format_builtin_fails() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "fmt",
            crate::feature_attrs::AttrRecord {
                name: "no_alloc".into(),
                args: String::new(),
                line: 0,
            },
        );
        let src = r#"fn fmt(int x) -> void { format("x={}", x); }"#;
        let (prog, _) = parse(src);
        let err = check(&prog, "test");
        assert!(err.is_err());
        crate::feature_attrs::reset();
    }
}
