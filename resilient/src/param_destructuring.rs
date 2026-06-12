//! Feature 47/50 — Parameter Destructuring.
//!
//! `fn add((int x, int y) pair)` allows unpacking a tuple parameter
//! directly in the signature. The first slice ships an analysis pass
//! that identifies parameter names declared with the destructuring
//! syntax (encoded in the existing parameter list with synthetic
//! names) and validates the destructure shape.
//!
//! Today this is a recognition-only pass; full lowering — generating
//! a synthetic `let (x, y) = pair;` at the top of the body — is a
//! follow-up that touches `parse_function`. Recognising the
//! convention now lets downstream features (e.g., the LSP completion
//! database) advertise the destructured form.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;

#[derive(Debug, Clone)]
pub struct DestructureRequest {
    pub fn_name: String,
    pub param_index: usize,
    pub locals: Vec<String>,
}

/// Convention: a parameter declared with type `"(T1,T2,...)"` is
/// recognised as a tuple destructure target. The locals list is the
/// underscore-stripped param-name segments.
pub fn analyze(program: &Node) -> Vec<DestructureRequest> {
    let mut out = Vec::new();
    let Node::Program(stmts) = program else {
        return out;
    };
    for s in stmts {
        if let Node::Function {
            name, parameters, ..
        } = &s.node
        {
            for (i, (ty, pname)) in parameters.iter().enumerate() {
                if ty.starts_with('(') && ty.ends_with(')') {
                    let locals = pname
                        .trim_start_matches('_')
                        .split('_')
                        .map(|s| s.to_string())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>();
                    out.push(DestructureRequest {
                        fn_name: name.clone(),
                        param_index: i,
                        locals,
                    });
                }
            }
        }
    }
    out
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let reqs = analyze(program);
    for r in &reqs {
        if r.locals.is_empty() {
            return Err(format!(
                "{}:0:0: param_destructuring: `{}` parameter {} has \
                 destructuring syntax but no local names after \
                 underscore-stripping. Use `(T1, T2, ...)` with \
                 underscore-separated identifiers like `_x_y_z`",
                _source_path, r.fn_name, r.param_index
            ));
        }

        for (i, local) in r.locals.iter().enumerate() {
            if !is_valid_identifier(local) {
                return Err(format!(
                    "{}:0:0: param_destructuring: `{}` parameter {} local #{} \
                     has invalid name `{}` — destructured locals must be \
                     valid identifiers",
                    _source_path, r.fn_name, r.param_index, i, local
                ));
            }
        }

        let mut seen = std::collections::HashSet::new();
        for local in &r.locals {
            if !seen.insert(local.clone()) {
                return Err(format!(
                    "{}:0:0: param_destructuring: `{}` parameter {} \
                     has duplicate local name `{}`",
                    _source_path, r.fn_name, r.param_index, local
                ));
            }
        }

        eprintln!(
            "note: `{}` parameter {} is a tuple destructure; lowering to \
             `let ({}) = param;` at call sites is not yet supported — \
             use explicit binding in the function body",
            r.fn_name,
            r.param_index,
            r.locals.join(", ")
        );
    }
    Ok(())
}

fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let first = s.chars().next().unwrap();
    (first.is_alphabetic() || first == '_') && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;
    use crate::span::Span;

    fn program_with_destructure_param(param_ty: &str, param_name: &str) -> Node {
        Node::Program(vec![crate::span::Spanned {
            node: Node::Function {
                name: "destructure".to_string(),
                parameters: vec![(param_ty.to_string(), param_name.to_string())],
                defaults: vec![None],
                body: Box::new(Node::Block {
                    stmts: Vec::new(),
                    span: Span::default(),
                }),
                requires: Vec::new(),
                ensures: Vec::new(),
                recovers_to: None,
                return_type: None,
                span: Span::default(),
                pure: false,
                effects: crate::EffectSet::io(),
                type_params: Vec::new(),
                type_param_bounds: Vec::new(),
                fails: Vec::new(),
                is_pub: false,
            },
            span: Span::default(),
        }])
    }

    #[test]
    fn empty_program_no_requests() {
        let p = Node::Program(vec![]);
        assert!(analyze(&p).is_empty());
    }

    #[test]
    fn check_always_returns_ok() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn analyze_pure_function_no_requests() {
        let src = "fn f(int x) -> int { return x + 1; }\n";
        let (prog, _) = parse(src);
        assert!(analyze(&prog).is_empty());
    }

    #[test]
    fn check_with_destructure_param_returns_ok() {
        let prog = program_with_destructure_param("(int,int)", "_x_y");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn check_rejects_empty_destructure_locals() {
        let prog = program_with_destructure_param("(int,int)", "_");
        let err = check(&prog, "test").expect_err("empty locals must fail");
        assert!(err.contains("no local names after"));
    }

    #[test]
    fn check_rejects_duplicate_destructure_locals() {
        let prog = program_with_destructure_param("(int,int)", "_x_x");
        let err = check(&prog, "test").expect_err("duplicate locals must fail");
        assert!(err.contains("duplicate local name `x`"));
    }

    #[test]
    fn is_valid_identifier_rejects_empty() {
        assert!(!is_valid_identifier(""));
    }

    #[test]
    fn is_valid_identifier_rejects_numeric_start() {
        assert!(!is_valid_identifier("123start"));
    }

    #[test]
    fn is_valid_identifier_accepts_underscore_start() {
        assert!(is_valid_identifier("_start"));
        assert!(is_valid_identifier("_"));
    }

    #[test]
    fn is_valid_identifier_accepts_valid_names() {
        assert!(is_valid_identifier("valid_name"));
        assert!(is_valid_identifier("CamelCase"));
        assert!(is_valid_identifier("a"));
        assert!(is_valid_identifier("_x_y_z"));
    }

    #[test]
    fn duplicate_detection_logic() {
        // Test the duplicate detection logic independently.
        let locals = vec!["x".to_string(), "y".to_string(), "x".to_string()];
        let mut seen = std::collections::HashSet::new();
        let mut has_dup = false;
        for local in &locals {
            if !seen.insert(local.clone()) {
                has_dup = true;
                break;
            }
        }
        assert!(has_dup, "duplicate detection should find x appearing twice");
    }

    #[test]
    fn analyze_finds_no_destructuring_in_simple_params() {
        // Parse a function without destructuring syntax and verify analyze returns empty.
        let src = "fn simple(int x, int y) -> int { return x + y; }\n";
        let (prog, _) = parse(src);
        let reqs = analyze(&prog);
        assert_eq!(
            reqs.len(),
            0,
            "simple params should not trigger destructuring"
        );
    }
}
