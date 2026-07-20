//! RES-4197 Phase 3: `E0012` enforcement for `let const` bindings.
//!
//! Phase 2 (`lib.rs::parse_let_statement`) added the opt-in
//! `let const NAME = expr;` form — a `Node::LetStatement` whose
//! `is_const` field is `true`. This module is the typechecker pass
//! that gives that flag teeth: any later bare reassignment of `NAME`
//! (`NAME = ...`, or any compound assign — `+=`, `-=`, etc., which the
//! parser already desugars into a plain `Node::Assignment`, see
//! `Token::PlusAssign`'s doc comment in `lib.rs`) is rejected with the
//! registry's `E0012` code.
//!
//! ## Enforcement scope (see docs/IMMUTABILITY.md Phase 3)
//!
//! - **Same-function.** Each `Node::Function` body (and each
//!   `Node::ImplBlock` method body) is walked with its own, fresh
//!   scope stack — a `let const` in one function has no effect on
//!   any other function, and top-level statements form their own
//!   implicit scope.
//! - **Path-insensitive.** We do not model control flow: a
//!   reassignment inside an `if`/`while`/`for`/`match` arm is flagged
//!   exactly like one at the top of the function, regardless of
//!   whether both branches are actually reachable together.
//! - **Provable-only.** The only thing we prove is "this name was
//!   declared `let const` in an enclosing lexical scope and this
//!   statement writes to it with no closer non-const declaration of
//!   the same name in between." We do not attempt aliasing, field
//!   mutation through a struct, or any other indirect-mutation
//!   analysis — those are out of scope for RES-4197.
//! - **Shadowing is allowed.** A new `let NAME = ...;` (const or not)
//!   in the same or a nested block replaces the tracked binding for
//!   `NAME` from that point forward; assignments after the shadow
//!   target the new (non-const, unless it re-declares `let const`)
//!   binding and are not rejected.

use crate::Node;
use crate::span::Span;
use std::collections::HashMap;

/// RES-4197: gate for the `E0012` diagnostic label, mirroring
/// `typechecker.rs`'s `rich_diag_enabled` / `bounds_check.rs`'s. The
/// default message carries no bracketed code (this is a brand new
/// diagnostic — there is no legacy golden output to preserve); setting
/// `RESILIENT_RICH_DIAG=1` switches the level label to
/// `error[E0012]`, consistent with every other registry code in
/// `diag.rs`.
fn rich_diag_enabled() -> bool {
    static ENABLED: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var("RESILIENT_RICH_DIAG").as_deref() == Ok("1"));
    *ENABLED
}

fn format_e0012(source_path: &str, span: Span, name: &str) -> String {
    let level = if rich_diag_enabled() {
        "error[E0012]"
    } else {
        "error"
    };
    let loc = if span.start.line > 0 {
        format!(
            "{}:{}:{}: ",
            source_path, span.start.line, span.start.column
        )
    } else if source_path.is_empty() {
        String::new()
    } else {
        format!("{}: ", source_path)
    };
    format!(
        "{loc}{level}: cannot reassign `{name}` — it was declared `let const` and is immutable \
         for the rest of this function"
    )
}

/// Per-block scope frame: name -> "is this binding currently const".
/// A fresh `let` (const or not) overwrites the entry for its name in
/// the innermost frame, which is exactly lexical shadowing.
type ScopeStack = Vec<HashMap<String, bool>>;

fn is_const_active(scopes: &ScopeStack, name: &str) -> bool {
    for frame in scopes.iter().rev() {
        if let Some(is_const) = frame.get(name) {
            return *is_const;
        }
    }
    false
}

/// RES-4197: walk the whole program and reject any reassignment of a
/// `let const` binding. Entry point wired into
/// `typechecker.rs`'s `<EXTENSION_PASSES>` block.
pub fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let stmts = match program {
        Node::Program(s) => s,
        _ => return Ok(()),
    };
    // RES-4197: top-level statements share one implicit function-like
    // scope (mirrors how the interpreter treats top-level `let`s —
    // there is no enclosing `fn` to reset the scope stack at).
    let mut top_scopes: ScopeStack = vec![HashMap::new()];
    for s in stmts {
        check_top_level(&s.node, source_path, &mut top_scopes)?;
    }
    Ok(())
}

/// Dispatch a top-level statement: recurse into function bodies (each
/// with its own fresh scope stack — "same-function" enforcement) and
/// impl-block methods, otherwise walk it as an ordinary statement in
/// the shared top-level scope.
fn check_top_level(
    node: &Node,
    source_path: &str,
    top_scopes: &mut ScopeStack,
) -> Result<(), String> {
    match node {
        Node::Function { body, .. } => {
            let mut scopes: ScopeStack = vec![HashMap::new()];
            walk_stmt(body, source_path, &mut scopes)
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                if let Node::Function { body, .. } = m {
                    let mut scopes: ScopeStack = vec![HashMap::new()];
                    walk_stmt(body, source_path, &mut scopes)?;
                }
            }
            Ok(())
        }
        Node::ModuleDecl { body, .. } => {
            for s in body {
                check_top_level(s, source_path, top_scopes)?;
            }
            Ok(())
        }
        other => walk_stmt(other, source_path, top_scopes),
    }
}

/// Recursively walk a statement (or nested block-bearing construct),
/// tracking `let const` declarations and flagging reassignments.
fn walk_stmt(node: &Node, source_path: &str, scopes: &mut ScopeStack) -> Result<(), String> {
    match node {
        Node::Block { stmts, .. } => {
            scopes.push(HashMap::new());
            let result = (|| {
                for s in stmts {
                    walk_stmt(s, source_path, scopes)?;
                }
                Ok(())
            })();
            scopes.pop();
            result
        }
        Node::LetStatement { name, is_const, .. } => {
            if let Some(frame) = scopes.last_mut() {
                frame.insert(name.clone(), *is_const);
            }
            Ok(())
        }
        Node::Assignment { name, span, .. } => {
            if is_const_active(scopes, name) {
                return Err(format_e0012(source_path, *span, name));
            }
            Ok(())
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            walk_stmt(consequence, source_path, scopes)?;
            if let Some(alt) = alternative {
                walk_stmt(alt, source_path, scopes)?;
            }
            Ok(())
        }
        Node::WhileStatement { body, .. } => walk_stmt(body, source_path, scopes),
        Node::ForInStatement { body, .. } => walk_stmt(body, source_path, scopes),
        Node::LiveBlock { body, .. } => walk_stmt(body, source_path, scopes),
        Node::TryCatch { body, handlers, .. } => {
            scopes.push(HashMap::new());
            let mut result = Ok(());
            for s in body {
                if let Err(e) = walk_stmt(s, source_path, scopes) {
                    result = Err(e);
                    break;
                }
            }
            scopes.pop();
            result?;
            for (_, hstmts) in handlers {
                scopes.push(HashMap::new());
                let mut hresult = Ok(());
                for s in hstmts {
                    if let Err(e) = walk_stmt(s, source_path, scopes) {
                        hresult = Err(e);
                        break;
                    }
                }
                scopes.pop();
                hresult?;
            }
            Ok(())
        }
        Node::Match { arms, .. } => {
            for (_, _, body) in arms {
                walk_stmt(body, source_path, scopes)?;
            }
            Ok(())
        }
        Node::Function { body, .. } => {
            // RES-4197: a nested/local `fn` (if the grammar ever
            // allows one at statement position) gets its own fresh
            // scope — same "same-function" rule as top-level fns.
            let mut fn_scopes: ScopeStack = vec![HashMap::new()];
            walk_stmt(body, source_path, &mut fn_scopes)
        }
        // Every other node kind is either a leaf, an expression with
        // no statement-level `let`/`Assignment` inside it that this
        // pass needs to see, or a construct outside RES-4197 Phase 3's
        // scope (see module docs — "provable-only").
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Node {
        let lexer = crate::Lexer::new(src);
        let mut parser = crate::Parser::new(lexer);
        parser.parse_program()
    }

    fn with_rich_diag<T>(enabled: bool, f: impl FnOnce() -> T) -> T {
        // RES-4197: `rich_diag_enabled` caches via `LazyLock`, same as
        // every other gate in the codebase — tests only assert on
        // whichever branch the ambient env is already in (mirrors
        // `bounds_check.rs`'s `out_of_bounds_error_carries_e0009_when_gated`).
        let _ = enabled;
        f()
    }

    #[test]
    fn plain_let_reassignment_is_accepted() {
        let src = r#"
fn main() {
    let x = 1;
    x = 2;
}
main();
"#;
        let program = parse(src);
        assert!(check(&program, "<test>").is_ok());
    }

    #[test]
    fn let_const_reassignment_is_rejected() {
        let src = r#"
fn main() {
    let const x = 1;
    x = 2;
}
main();
"#;
        let program = parse(src);
        let err = check(&program, "<test>").expect_err("expected E0012");
        assert!(err.contains("cannot reassign"), "got: {err}");
        assert!(err.contains('x'), "got: {err}");
    }

    #[test]
    fn let_const_compound_assign_is_rejected() {
        let src = r#"
fn main() {
    let const total = 0;
    total += 1;
}
main();
"#;
        let program = parse(src);
        assert!(check(&program, "<test>").is_err());
    }

    #[test]
    fn let_const_never_reassigned_is_accepted() {
        let src = r#"
fn main() {
    let const x = 1;
    let y = x + 1;
}
main();
"#;
        let program = parse(src);
        assert!(check(&program, "<test>").is_ok());
    }

    #[test]
    fn shadowing_with_new_let_permits_reassignment() {
        let src = r#"
fn main() {
    let const x = 1;
    if true {
        let x = 2;
        x = 3;
    }
}
main();
"#;
        let program = parse(src);
        assert!(
            check(&program, "<test>").is_ok(),
            "an inner `let` re-declaration should shadow the outer `let const`"
        );
    }

    #[test]
    fn let_const_in_one_fn_does_not_leak_into_another() {
        let src = r#"
fn a() {
    let const x = 1;
}
fn b() {
    let x = 1;
    x = 2;
}
a();
b();
"#;
        let program = parse(src);
        assert!(check(&program, "<test>").is_ok());
    }

    #[test]
    fn let_const_inside_if_branch_is_rejected_path_insensitively() {
        let src = r#"
fn main() {
    let const x = 1;
    if true {
        x = 2;
    }
}
main();
"#;
        let program = parse(src);
        assert!(check(&program, "<test>").is_err());
    }

    #[test]
    fn e0012_carries_line_and_column() {
        let src = "fn main() {\n    let const x = 1;\n    x = 2;\n}\nmain();\n";
        let program = parse(src);
        let err = check(&program, "example.rz").expect_err("expected E0012");
        assert!(err.starts_with("example.rz:3:"), "got: {err}");
    }

    #[test]
    fn e0012_is_gated_by_rich_diag_env_var() {
        with_rich_diag(true, || {
            let src = r#"
fn main() {
    let const x = 1;
    x = 2;
}
main();
"#;
            let program = parse(src);
            let err = check(&program, "<test>").expect_err("expected E0012");
            if std::env::var("RESILIENT_RICH_DIAG").as_deref() == Ok("1") {
                assert!(err.contains("error[E0012]"), "got: {err}");
            } else {
                assert!(err.contains("error:"), "got: {err}");
                assert!(!err.contains("[E0012]"), "code leaked into default: {err}");
            }
        });
    }
}
