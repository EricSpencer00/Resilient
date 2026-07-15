//! RES-4078 (A-E2): const-generic fixed-array length checking.
//!
//! `[T; N]` type annotations (RES-157a) carry a compile-time length
//! `N`, but until this pass nothing ever compared `N` against an
//! initializer — `let xs: [int; 3] = [1, 2];` and `sum([1, 2])`
//! against a `[int; 3]` parameter both typechecked and only failed
//! (or silently misbehaved) at runtime.
//!
//! This pass rejects **provable** length mismatches, i.e. only where
//! the value is a direct array literal whose length is syntactically
//! known:
//!
//! * `let x: [T; N] = [ ... ];` and `const x: [T; N] = [ ... ];`
//! * direct call arguments paired with a `[T; N]`-typed parameter
//! * `return [ ... ];` against a `[T; N]` fn return type (statement
//!   positions reachable through plain control-flow blocks)
//!
//! Anything that is not a direct array literal — a variable, a call
//! result, a slice, a concatenation — is left alone. Conservative by
//! design: zero false positives, every previously-compiling program
//! keeps compiling. Runtime bounds checks (E0009) remain the backstop
//! for lengths this pass cannot see.

use crate::Node;
use resilient_span::Span;
use std::collections::HashMap;

/// Parse a canonical fixed-size array annotation `[elem; N]` (the
/// exact shape `parse_type_annotation` serializes) into `N`.
/// Returns `None` for every other type string.
fn fixed_len(ty: &str) -> Option<usize> {
    let inner = ty.trim().strip_prefix('[')?.strip_suffix(']')?;
    let (_, len) = inner.rsplit_once(';')?;
    len.trim().parse::<usize>().ok()
}

/// Length of a direct array-literal expression, `None` for any other
/// expression shape (which this pass then ignores).
fn literal_len(value: &Node) -> Option<(usize, Span)> {
    match value {
        Node::ArrayLiteral { items, span } => Some((items.len(), *span)),
        _ => None,
    }
}

fn format_err(source_path: &str, span: Span, msg: &str) -> String {
    if span.start.line == 0 {
        msg.to_string()
    } else {
        format!(
            "{}:{}:{}: {}",
            source_path, span.start.line, span.start.column, msg
        )
    }
}

/// Program-level pass invoked from `<EXTENSION_PASSES>` in
/// `typechecker.rs`.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    // Pass 1: collect fn signatures so call sites can pair positional
    // array-literal arguments with `[T; N]`-typed parameters.
    let mut fns: HashMap<&str, &[(String, String)]> = HashMap::new();
    crate::uniqueness_walk::visit(program, &mut |n| {
        if let Node::Function {
            name, parameters, ..
        } = n
        {
            fns.insert(name.as_str(), parameters.as_slice());
        }
    });

    // Pass 2: check every provable site. `uniqueness_walk::visit` is
    // infallible, so record the first error and surface it after.
    let mut first_err: Option<String> = None;
    crate::uniqueness_walk::visit(program, &mut |n| {
        if first_err.is_some() {
            return;
        }
        match n {
            Node::LetStatement {
                name,
                value,
                type_annot: Some(ty),
                ..
            }
            | Node::Const {
                name,
                value,
                type_annot: Some(ty),
                ..
            } => {
                if let (Some(want), Some((got, span))) = (fixed_len(ty), literal_len(value.as_ref()))
                    && got != want
                {
                    first_err = Some(format_err(
                        source_path,
                        span,
                        &format!(
                            "array literal has {} element(s) but `{}` is declared `{}` (expected {})",
                            got, name, ty, want
                        ),
                    ));
                }
            }
            Node::CallExpression {
                function,
                arguments,
                ..
            } => {
                let Node::Identifier { name: callee, .. } = function.as_ref() else {
                    return;
                };
                let Some(params) = fns.get(callee.as_str()) else {
                    return;
                };
                for (arg, (ptype, pname)) in arguments.iter().zip(params.iter()) {
                    if let (Some(want), Some((got, span))) = (fixed_len(ptype), literal_len(arg))
                        && got != want
                    {
                        first_err = Some(format_err(
                            source_path,
                            span,
                            &format!(
                                "array literal has {} element(s) but parameter `{}` of `{}` is declared `{}` (expected {})",
                                got, pname, callee, ptype, want
                            ),
                        ));
                        return;
                    }
                }
            }
            Node::Function {
                name,
                body,
                return_type: Some(rt),
                ..
            } => {
                if let Some(want) = fixed_len(rt)
                    && let Some((got, span)) = find_return_literal_mismatch(body.as_ref(), want)
                {
                    first_err = Some(format_err(
                        source_path,
                        span,
                        &format!(
                            "array literal has {} element(s) but `{}` returns `{}` (expected {})",
                            got, name, rt, want
                        ),
                    ));
                }
            }
            _ => {}
        }
    });
    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Find a `return [ ... ];` whose literal length differs from `want`,
/// descending only through plain control-flow containers and stopping
/// at nested `Node::Function` boundaries (their own annotation governs
/// their returns).
fn find_return_literal_mismatch(node: &Node, want: usize) -> Option<(usize, Span)> {
    match node {
        Node::ReturnStatement {
            value: Some(v), ..
        } => match literal_len(v.as_ref()) {
            Some((got, span)) if got != want => Some((got, span)),
            _ => None,
        },
        Node::Program(stmts) => stmts
            .iter()
            .find_map(|s| find_return_literal_mismatch(&s.node, want)),
        Node::Block { stmts, .. } => stmts
            .iter()
            .find_map(|s| find_return_literal_mismatch(s, want)),
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => find_return_literal_mismatch(consequence.as_ref(), want).or_else(|| {
            alternative
                .as_ref()
                .and_then(|a| find_return_literal_mismatch(a.as_ref(), want))
        }),
        // `loop { ... }` desugars to `WhileStatement` at parse time.
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            find_return_literal_mismatch(body.as_ref(), want)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Lexer, Parser};

    fn typecheck(src: &str) -> Result<(), String> {
        let lexer = Lexer::new(src);
        let mut parser = Parser::new(lexer);
        let program = parser.parse_program();
        check(&program, "test.rz")
    }

    #[test]
    fn fixed_len_parses_canonical_forms() {
        assert_eq!(fixed_len("[int; 3]"), Some(3));
        assert_eq!(fixed_len("[T; 0]"), Some(0));
        assert_eq!(fixed_len("[float; 12]"), Some(12));
        assert_eq!(fixed_len("int"), None);
        assert_eq!(fixed_len("[int]"), None);
        assert_eq!(fixed_len("array<int>"), None);
        assert_eq!(fixed_len("[int; N]"), None);
    }

    #[test]
    fn let_literal_mismatch_rejected() {
        let err = typecheck("fn main() { let xs: [int; 3] = [1, 2]; }\nmain();\n").unwrap_err();
        assert!(err.contains("array literal has 2 element(s)"), "got: {err}");
        assert!(err.contains("[int; 3]"), "got: {err}");
        assert!(err.contains("test.rz:1:"), "diagnostic must carry line:col, got: {err}");
    }

    #[test]
    fn let_literal_exact_accepted() {
        typecheck("fn main() { let xs: [int; 3] = [1, 2, 3]; }\nmain();\n").unwrap();
    }

    #[test]
    fn let_non_literal_initializer_permissive() {
        // Length not syntactically knowable — must stay accepted.
        typecheck(
            "fn make() { return [1, 2]; }\nfn main() { let xs: [int; 3] = make(); }\nmain();\n",
        )
        .unwrap();
    }

    #[test]
    fn call_arg_literal_mismatch_rejected() {
        let err = typecheck(
            "fn sum(int a, [int; 3] v) -> int { return a; }\nfn main() { sum(1, [1, 2]); }\nmain();\n",
        )
        .unwrap_err();
        assert!(err.contains("parameter `v` of `sum`"), "got: {err}");
        assert!(err.contains("expected 3"), "got: {err}");
    }

    #[test]
    fn call_arg_literal_exact_accepted() {
        typecheck(
            "fn sum(int a, [int; 3] v) -> int { return a; }\nfn main() { sum(1, [1, 2, 3]); }\nmain();\n",
        )
        .unwrap();
    }

    #[test]
    fn call_arg_variable_permissive() {
        typecheck(
            "fn sum(int a, [int; 3] v) -> int { return a; }\nfn main() { let xs = [1, 2]; sum(1, xs); }\nmain();\n",
        )
        .unwrap();
    }

    #[test]
    fn return_literal_mismatch_rejected() {
        let err =
            typecheck("fn make() -> [int; 3] { return [1, 2]; }\nfn main() { make(); }\nmain();\n")
                .unwrap_err();
        assert!(err.contains("`make` returns `[int; 3]`"), "got: {err}");
    }

    #[test]
    fn return_literal_in_if_branch_rejected() {
        let err = typecheck(
            "fn make(int b) -> [int; 2] { if b > 0 { return [1, 2, 3]; } return [1, 2]; }\nfn main() { make(1); }\nmain();\n",
        )
        .unwrap_err();
        assert!(err.contains("3 element(s)"), "got: {err}");
    }

    #[test]
    fn nested_fn_returns_not_attributed_to_outer() {
        // The inner fn's `[int; 2]` governs its own return; the outer
        // fn returning `[int; 3]` must not flag the inner literal.
        typecheck(
            "fn outer() -> [int; 3] {\n    fn inner() -> [int; 2] { return [1, 2]; }\n    return [1, 2, 3];\n}\nfn main() { outer(); }\nmain();\n",
        )
        .unwrap();
    }

    #[test]
    fn zero_length_annotation_checked() {
        let err = typecheck("fn main() { let xs: [int; 0] = [1]; }\nmain();\n").unwrap_err();
        assert!(err.contains("expected 0"), "got: {err}");
        typecheck("fn main() { let xs: [int; 0] = []; }\nmain();\n").unwrap();
    }
}
