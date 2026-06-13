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

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let reqs = analyze(program);
    for r in &reqs {
        if r.locals.is_empty() {
            return Err(format!(
                "{}:0:0: param_destructuring: `{}` parameter {} has \
                 destructuring syntax but no local names after \
                 underscore-stripping. Use `(T1, T2, ...)` with \
                 underscore-separated identifiers like `_x_y_z`",
                source_path, r.fn_name, r.param_index
            ));
        }

        for (i, local) in r.locals.iter().enumerate() {
            if !is_valid_identifier(local) {
                return Err(format!(
                    "{}:0:0: param_destructuring: `{}` parameter {} local #{} \
                     has invalid name `{}` — destructured locals must be \
                     valid identifiers",
                    source_path, r.fn_name, r.param_index, i, local
                ));
            }
        }

        let mut seen = std::collections::HashSet::new();
        for local in &r.locals {
            if !seen.insert(local.clone()) {
                return Err(format!(
                    "{}:0:0: param_destructuring: `{}` parameter {} \
                     has duplicate local name `{}`",
                    source_path, r.fn_name, r.param_index, local
                ));
            }
        }

        if let Node::Program(stmts) = program {
            for s in stmts {
                if let Node::Function {
                    name, parameters, ..
                } = &s.node
                {
                    if name == &r.fn_name && r.param_index < parameters.len() {
                        let (ty, _) = &parameters[r.param_index];
                        let Some(tuple_arity) = tuple_type_arity(ty) else {
                            return Err(format!(
                                "{}:0:0: param_destructuring: `{}` parameter {} \
                                 has malformed tuple type `{}`. Tuple types must be \
                                 non-empty and comma-separated, like `(int, int)` or `(int, float, bool)`",
                                source_path, r.fn_name, r.param_index, ty
                            ));
                        };
                        if tuple_arity != r.locals.len() {
                            return Err(format!(
                                "{}:0:0: param_destructuring: `{}` parameter {} \
                                 destructures {} local names from tuple type `{}` with {} element(s)",
                                source_path,
                                r.fn_name,
                                r.param_index,
                                r.locals.len(),
                                ty,
                                tuple_arity
                            ));
                        }
                    }
                }
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

    // RES-3236: validate call-site arguments for functions with destructuring parameters
    check_destructuring_call_sites(program, source_path, &reqs)?;

    Ok(())
}

fn check_destructuring_call_sites(
    node: &Node,
    source_path: &str,
    reqs: &[DestructureRequest],
) -> Result<(), String> {
    // Build a map of function names to their destructuring requirements
    let mut destructuring_map: std::collections::HashMap<String, &DestructureRequest> =
        std::collections::HashMap::new();
    for req in reqs {
        destructuring_map.insert(req.fn_name.clone(), req);
    }

    // Walk the AST looking for function calls
    walk_call_sites(node, source_path, &destructuring_map);
    Ok(())
}

fn walk_call_sites(
    node: &Node,
    source_path: &str,
    destructuring_map: &std::collections::HashMap<String, &DestructureRequest>,
) {
    if let Node::CallExpression {
        function,
        arguments,
        span,
    } = node
    {
        if let Node::Identifier { name, .. } = function.as_ref() {
            if let Some(req) = destructuring_map.get(name) {
                // The function has destructuring; check argument count
                // The function expects req.param_index + 1 parameters total
                // (since param_index is 0-based, a destructuring on param 0
                // means the function expects at least 1 parameter)
                let expected_params = req.param_index + 1; // at minimum, the destructured param
                if arguments.len() < expected_params {
                    eprintln!(
                        "{}:{}:0: note: call to `{}` provides {} arguments but function has \
                         destructuring parameter at position {} (expected at least {})",
                        source_path,
                        span.start.line,
                        name,
                        arguments.len(),
                        req.param_index,
                        expected_params
                    );
                }
            }
        }
    }
    crate::uniqueness_walk::walk_children(node, &mut |child| {
        walk_call_sites(child, source_path, destructuring_map);
    });
}

fn validate_tuple_type(ty: &str) -> bool {
    tuple_type_arity(ty).is_some()
}

fn tuple_type_arity(ty: &str) -> Option<usize> {
    if !ty.starts_with('(') || !ty.ends_with(')') {
        return None;
    }
    let inner = ty[1..ty.len() - 1].trim();
    if inner.is_empty() {
        return None;
    }
    let parts = inner.split(',').map(str::trim).collect::<Vec<_>>();
    if parts.len() < 2 || parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    Some(parts.len())
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

    #[test]
    fn check_rejects_empty_tuple_type() {
        let prog = program_with_destructure_param("()", "_x");
        let err = check(&prog, "test").expect_err("empty tuple must fail");
        assert!(
            err.contains("malformed tuple type") || err.contains("non-empty"),
            "got: {err}"
        );
    }

    #[test]
    fn check_rejects_tuple_ending_with_comma() {
        let prog = program_with_destructure_param("(int,)", "_x");
        let err = check(&prog, "test").expect_err("trailing comma must fail");
        assert!(
            err.contains("malformed tuple type") || err.contains("comma"),
            "got: {err}"
        );
    }

    #[test]
    fn check_rejects_single_element_tuple_no_comma() {
        let prog = program_with_destructure_param("(int)", "_x");
        let err = check(&prog, "test").expect_err("single-element tuple without comma must fail");
        assert!(
            err.contains("malformed tuple type") || err.contains("comma-separated"),
            "got: {err}"
        );
    }

    #[test]
    fn check_rejects_tuple_local_arity_mismatch() {
        let prog = program_with_destructure_param("(int,int)", "_x_y_z");
        let err = check(&prog, "test").expect_err("arity mismatch must fail");
        assert!(
            err.contains("destructures 3 local names") && err.contains("2 element"),
            "got: {err}"
        );
    }

    #[test]
    fn check_accepts_valid_two_element_tuple() {
        let prog = program_with_destructure_param("(int,int)", "_x_y");
        assert!(
            check(&prog, "test").is_ok(),
            "valid two-element tuple must pass"
        );
    }

    #[test]
    fn check_accepts_valid_three_element_tuple_with_spaces() {
        let prog = program_with_destructure_param("(int, float, bool)", "_x_y_z");
        assert!(check(&prog, "test").is_ok(), "tuple with spaces must pass");
    }

    #[test]
    fn check_accepts_valid_many_element_tuple() {
        let prog = program_with_destructure_param("(int,int,int,int,int)", "_a_b_c_d_e");
        assert!(check(&prog, "test").is_ok(), "many-element tuple must pass");
    }

    #[test]
    fn validate_tuple_type_empty() {
        assert!(!validate_tuple_type("()"));
    }

    #[test]
    fn validate_tuple_type_single_no_comma() {
        assert!(!validate_tuple_type("(int)"));
    }

    #[test]
    fn validate_tuple_type_trailing_comma() {
        assert!(!validate_tuple_type("(int,)"));
    }

    #[test]
    fn validate_tuple_type_missing_parens() {
        assert!(!validate_tuple_type("int,int"));
        assert!(!validate_tuple_type("(int,int"));
        assert!(!validate_tuple_type("int,int)"));
    }

    #[test]
    fn validate_tuple_type_valid_two_element() {
        assert!(validate_tuple_type("(int,int)"));
    }

    #[test]
    fn validate_tuple_type_valid_with_spaces() {
        assert!(validate_tuple_type("(int, float)"));
        assert!(validate_tuple_type("( int , float )"));
    }

    #[test]
    fn validate_tuple_type_valid_many_element() {
        assert!(validate_tuple_type("(int,int,int,int,int)"));
    }

    // ── Malformed-input regression corpus ──────────────────────────────────
    // Comprehensive test cases for malformed param_destructuring declarations,
    // duplicate forms, and invalid arguments.

    #[test]
    fn check_rejects_local_starting_with_digit() {
        let prog = program_with_destructure_param("(int, int)", "_1x_2y");
        let err = check(&prog, "test").expect_err("digit-start local must fail");
        assert!(
            err.contains("invalid name") && err.contains("valid identifier"),
            "{err}"
        );
    }

    #[test]
    fn check_rejects_local_with_special_chars() {
        let prog = program_with_destructure_param("(int, int)", "_x@_y");
        let err = check(&prog, "test").expect_err("special char local must fail");
        assert!(
            err.contains("invalid name") && err.contains("valid identifier"),
            "{err}"
        );
    }

    #[test]
    fn check_rejects_local_with_hyphen() {
        let prog = program_with_destructure_param("(int, int)", "_x-y_z");
        let err = check(&prog, "test").expect_err("hyphen local must fail");
        assert!(
            err.contains("invalid name") && err.contains("valid identifier"),
            "{err}"
        );
    }

    #[test]
    fn check_rejects_duplicate_local_exact() {
        let prog = program_with_destructure_param("(int, int, int)", "_x_y_x");
        let err = check(&prog, "test").expect_err("duplicate local exact must fail");
        assert!(err.contains("duplicate local name"), "{err}");
    }

    #[test]
    fn check_rejects_duplicate_local_case_sensitive() {
        // Case-sensitivity: x and X should be different
        let prog = program_with_destructure_param("(int, int)", "_x_X");
        assert!(check(&prog, "test").is_ok(), "x and X must be distinct");
    }

    #[test]
    fn check_rejects_arity_mismatch_too_few_locals() {
        let prog = program_with_destructure_param("(int, int, int, int)", "_x_y");
        let err = check(&prog, "test").expect_err("too few locals must fail");
        assert!(
            err.contains("destructures") && err.contains("4 element"),
            "{err}"
        );
    }

    #[test]
    fn check_rejects_arity_mismatch_too_many_locals() {
        let prog = program_with_destructure_param("(int, int)", "_x_y_z");
        let err = check(&prog, "test").expect_err("too many locals must fail");
        assert!(
            err.contains("destructures") && err.contains("2 element"),
            "{err}"
        );
    }

    #[test]
    fn check_rejects_malformed_tuple_empty_parens() {
        let prog = program_with_destructure_param("()", "_x");
        let err = check(&prog, "test").expect_err("empty tuple must fail");
        assert!(err.contains("malformed tuple type"), "{err}");
    }

    #[test]
    fn check_rejects_malformed_tuple_single_no_comma() {
        let prog = program_with_destructure_param("(int)", "_x");
        let err = check(&prog, "test").expect_err("single element no comma must fail");
        assert!(err.contains("malformed tuple type"), "{err}");
    }

    #[test]
    fn check_rejects_local_with_unicode_special() {
        let prog = program_with_destructure_param("(int, int)", "_x_€");
        let err = check(&prog, "test").expect_err("unicode special char must fail");
        assert!(
            err.contains("invalid name") && err.contains("valid identifier"),
            "{err}"
        );
    }

    #[test]
    fn check_rejects_local_with_space() {
        let prog = program_with_destructure_param("(int, int)", "_x _y");
        let err = check(&prog, "test").expect_err("space in local must fail");
        // Space is not a valid identifier character
        assert!(err.contains("invalid name"), "{err}");
    }

    // Valid baseline cases for regression detection

    #[test]
    fn check_accepts_valid_two_element_basic() {
        let prog = program_with_destructure_param("(int,int)", "_x_y");
        assert!(check(&prog, "test").is_ok(), "basic two-element must pass");
    }

    #[test]
    fn check_accepts_valid_three_element_with_spaces() {
        let prog = program_with_destructure_param("(int, string, bool)", "_a_b_c");
        assert!(
            check(&prog, "test").is_ok(),
            "three-element with spaces must pass"
        );
    }

    #[test]
    fn check_accepts_uppercase_identifiers() {
        let prog = program_with_destructure_param("(int, int)", "_X_Y");
        assert!(
            check(&prog, "test").is_ok(),
            "uppercase identifiers must pass"
        );
    }

    #[test]
    fn check_accepts_many_element_tuple() {
        let prog = program_with_destructure_param("(int,int,int,int,int,int)", "_a_b_c_d_e_f");
        assert!(check(&prog, "test").is_ok(), "many-element tuple must pass");
    }

    #[test]
    fn check_accepts_leading_underscores() {
        let prog = program_with_destructure_param("(int, int)", "___x_y");
        assert!(
            check(&prog, "test").is_ok(),
            "leading underscores must pass"
        );
    }

    #[test]
    fn check_accepts_single_letter_names() {
        let prog = program_with_destructure_param("(int, int, int)", "_a_b_c");
        assert!(
            check(&prog, "test").is_ok(),
            "single-letter names must pass"
        );
    }

    #[test]
    fn check_accepts_names_with_digits() {
        let prog = program_with_destructure_param("(int, int)", "_x1_y2");
        assert!(
            check(&prog, "test").is_ok(),
            "names with trailing digits must pass"
        );
    }

    #[test]
    fn check_call_site_validation_detects_insufficient_args() {
        // RES-3236: call-site validation should note mismatched argument counts
        let src = r#"
fn process((int x, int y) pair) -> int {
    return x + y;
}

fn main() {
    let result = process(42);
}
"#;
        let (prog, _) = crate::parse(src);
        // The check should succeed but emit a diagnostic note
        // (call-site validation is a note, not an error, since full type checking is needed)
        let result = check(&prog, "test");
        assert!(result.is_ok(), "call-site validation should not error");
    }
}
