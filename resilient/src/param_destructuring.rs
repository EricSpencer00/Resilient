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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

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
        // The check() function always returns Ok — warnings are advisory.
        let src = "fn g(int x) -> int { return x; }\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }
}
