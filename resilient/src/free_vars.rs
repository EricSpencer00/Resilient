//! RES-164a: reusable free-variable analysis on the AST.
//!
//! Given a `Node` (typically a `FunctionLiteral`'s body), returns
//! the set of identifier names referenced inside it that aren't
//! bound by a parameter, a local `let`, a `for`-in / `match`
//! binding, or any other scope introduced in the same subtree.
//!
//! This is a pure AST walk — no `Environment`, no `Value`, no
//! stdlib lookup. That's important for RES-164c/d: the JIT needs
//! to know which names to materialize in the captured env
//! *before* any runtime evaluation, and the interpreter's
//! existing free-var notion was tangled with `Rc<RefCell<Env>>`
//! inheritance in `apply_function`, not extractable as a helper.
//!
//! ## What counts as a binding
//!
//! Within a subtree, a name becomes bound when it appears as:
//!
//! - A `Node::LetStatement` / `Node::StaticLet` binder.
//! - A `Node::LetDestructureStruct` local name.
//! - A `Node::ForInStatement` loop variable.
//! - A `Node::Function` / `Node::FunctionLiteral` parameter. The
//!   function's own name (for the top-level `Node::Function`) is
//!   also treated as bound inside its body, matching the
//!   interpreter's recursive-self-bind behaviour.
//! - A `Pattern::Identifier` binder in a `match` arm. The arm's
//!   guard and body see that name; sibling arms do not.
//!
//! ## What counts as a free reference
//!
//! Only `Node::Identifier { name }` reads. Function calls through
//! a callee expression recurse into `function`; the callee slot
//! is where `Node::Identifier` reads surface for calls. Types in
//! parameter / field annotations are strings today (RES-157a),
//! not `Node::Identifier`, so they never count.
//!
//! ## Shadowing
//!
//! A `let x = x + 1;` binds `x` only *after* the RHS runs, so the
//! RHS `x` is still free with respect to this scope. We mirror
//! that: the binder is added to the `bound` set only for statements
//! that come *after* the `let` within the same block.
//!
//! ## What this module is NOT
//!
//! - It doesn't validate scopes (that's the typechecker's job).
//! - It doesn't resolve aliases.
//! - It doesn't follow `use` imports (those are resolved before
//!   this pass runs — RES-073).
//!
//! The public entry point `free_vars` is only consumed from the
//! tests in this module today; RES-164c/d will wire it into the
//! JIT lowering. We `allow(dead_code)` at the module level so the
//! regular build stays warning-clean until then.

#![allow(dead_code)]

use std::collections::BTreeSet;

use crate::{Node, Pattern};

/// Top-level entry: free variables of a single AST node.
///
/// `BTreeSet` keeps the output deterministic for golden-test
/// style assertions.
pub fn free_vars(node: &Node) -> BTreeSet<String> {
    let mut free = BTreeSet::new();
    let mut bound = BTreeSet::new();
    walk(node, &mut bound, &mut free);
    free
}

/// Shared walker. `bound` is the set of names currently in scope
/// AT THIS POINT in the walk; `free` is the set of names that
/// have been observed as reads while unbound.
///
/// Each recursive call must restore `bound` to its entry value
/// before returning (we model scope exit by snapshotting the
/// bound-set length and truncating back).
fn walk(node: &Node, bound: &mut BTreeSet<String>, free: &mut BTreeSet<String>) {
    match node {
        // ---- Leaves ----
        Node::Identifier { name, .. } => {
            if !bound.contains(name) {
                free.insert(name.clone());
            }
        }
        Node::IntegerLiteral { .. }
        | Node::FloatLiteral { .. }
        | Node::StringLiteral { .. }
        | Node::BytesLiteral { .. }
        | Node::BooleanLiteral { .. }
        | Node::DurationLiteral { .. } => {}

        // ---- Top-level shape ----
        Node::Program(stmts) => {
            // Top-level `fn` / `let` / `static let` declarations
            // are all visible to each other (forward reference is
            // allowed — the interpreter does a prepass). Collect
            // their names up front, then walk each statement.
            let snapshot = bound.len();
            for s in stmts {
                collect_top_level_binder(&s.node, bound);
            }
            for s in stmts {
                walk(&s.node, bound, free);
            }
            truncate_to(bound, snapshot);
        }
        Node::Use { .. } => {}
        // FFI v1: extern blocks don't introduce Resilient bindings
        // at the source level; driver resolves them separately.
        Node::Extern { .. } => {}

        // ---- Blocks introduce scope ----
        Node::Block { stmts, .. } => {
            let snapshot = bound.len();
            for s in stmts {
                walk(s, bound, free);
                // Let / for-in introduce a binder that extends
                // into subsequent statements within the same block.
                // Note: we add the binder AFTER walking the stmt
                // itself so the RHS sees the outer scope.
                match s {
                    Node::LetStatement { name, .. } | Node::StaticLet { name, .. } => {
                        bound.insert(name.clone());
                    }
                    Node::LetDestructureStruct { fields, .. } => {
                        for (_, local) in fields {
                            bound.insert(local.clone());
                        }
                    }
                    _ => {}
                }
            }
            truncate_to(bound, snapshot);
        }

        // ---- Declarations ----
        Node::Function {
            name,
            parameters,
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            let snapshot = bound.len();
            // The fn's own name is visible inside the body (for
            // recursion) and in `requires` / `ensures` (for
            // contract recursion). Parameter names are in scope
            // for all three.
            bound.insert(name.clone());
            for (_, param_name) in parameters {
                bound.insert(param_name.clone());
            }
            for clause in requires {
                walk(clause, bound, free);
            }
            walk(body, bound, free);
            // `result` is a magic name visible in `ensures`
            // clauses — treat it as bound for those walks.
            let pre_ensures = bound.len();
            bound.insert("result".into());
            for clause in ensures {
                walk(clause, bound, free);
            }
            // RES-392: `recovers_to` binds in the same env as
            // `ensures` — `result` and the parameters are in scope.
            if let Some(rec) = recovers_to {
                walk(rec, bound, free);
            }
            truncate_to(bound, pre_ensures);
            truncate_to(bound, snapshot);
        }
        Node::FunctionLiteral {
            parameters,
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            let snapshot = bound.len();
            for (_, param_name) in parameters {
                bound.insert(param_name.clone());
            }
            for clause in requires {
                walk(clause, bound, free);
            }
            walk(body, bound, free);
            let pre_ensures = bound.len();
            bound.insert("result".into());
            for clause in ensures {
                walk(clause, bound, free);
            }
            if let Some(rec) = recovers_to {
                walk(rec, bound, free);
            }
            truncate_to(bound, pre_ensures);
            truncate_to(bound, snapshot);
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                walk(m, bound, free);
            }
        }
        Node::StructDecl { .. } | Node::TypeAlias { .. } | Node::RegionDecl { .. } => {}
        // RES-386: Actor declarations are verifier-only.
        Node::Actor { .. } => {}
        // RES-390: ClusterDecl introduces no bindings.
        Node::ClusterDecl { .. } => {}
        // RES-388/RES-390: ActorDecl walks state field initializers,
        // always invariants, and each handler body.
        Node::ActorDecl {
            state_fields,
            always_clauses,
            receive_handlers,
            ..
        } => {
            let snapshot = bound.len();
            for (_, field, init) in state_fields {
                walk(init, bound, free);
                bound.insert(field.clone());
            }
            for clause in always_clauses {
                walk(clause, bound, free);
            }
            for handler in receive_handlers {
                let handler_snapshot = bound.len();
                for (_, pname) in &handler.parameters {
                    bound.insert(pname.clone());
                }
                for r in &handler.requires {
                    walk(r, bound, free);
                }
                walk(&handler.body, bound, free);
                bound.insert("result".into());
                for e in &handler.ensures {
                    walk(e, bound, free);
                }
                truncate_to(bound, handler_snapshot);
            }
            truncate_to(bound, snapshot);
        }

        // ---- Statements ----
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => {
            // RHS evaluated in outer scope; the binder is added
            // by the enclosing block after this walk returns.
            walk(value, bound, free);
        }
        Node::LetDestructureStruct { value, .. } => {
            walk(value, bound, free);
        }
        Node::Assignment { name, value, .. } => {
            // LHS is a read of `name` in Resilient's scoping model —
            // you can't assign to a name you haven't declared.
            if !bound.contains(name) {
                free.insert(name.clone());
            }
            walk(value, bound, free);
        }
        Node::ReturnStatement { value, .. } => {
            if let Some(v) = value {
                walk(v, bound, free);
            }
        }
        Node::ExpressionStatement { expr, .. } => walk(expr, bound, free),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk(condition, bound, free);
            walk(consequence, bound, free);
            if let Some(a) = alternative {
                walk(a, bound, free);
            }
        }
        Node::WhileStatement {
            condition,
            body,
            invariants,
            ..
        } => {
            walk(condition, bound, free);
            for inv in invariants {
                walk(inv, bound, free);
            }
            walk(body, bound, free);
        }
        Node::ForInStatement {
            name,
            iterable,
            body,
            invariants,
            ..
        } => {
            walk(iterable, bound, free);
            let snapshot = bound.len();
            bound.insert(name.clone());
            for inv in invariants {
                walk(inv, bound, free);
            }
            walk(body, bound, free);
            truncate_to(bound, snapshot);
        }
        Node::LiveBlock {
            body, invariants, ..
        } => {
            walk(body, bound, free);
            for inv in invariants {
                walk(inv, bound, free);
            }
        }
        Node::Assert { condition, .. } => walk(condition, bound, free),
        Node::Assume { condition, .. } => walk(condition, bound, free),

        // ---- Expressions ----
        Node::PrefixExpression { right, .. } => walk(right, bound, free),
        Node::InfixExpression { left, right, .. } => {
            walk(left, bound, free);
            walk(right, bound, free);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            walk(function, bound, free);
            for a in arguments {
                walk(a, bound, free);
            }
        }
        Node::TryExpression { expr, .. } => walk(expr, bound, free),
        Node::IndexExpression { target, index, .. } => {
            walk(target, bound, free);
            walk(index, bound, free);
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            walk(target, bound, free);
            walk(index, bound, free);
            walk(value, bound, free);
        }
        Node::FieldAccess { target, .. } => walk(target, bound, free),
        Node::FieldAssignment { target, value, .. } => {
            walk(target, bound, free);
            walk(value, bound, free);
        }
        Node::ArrayLiteral { items, .. } => {
            for i in items {
                walk(i, bound, free);
            }
        }
        // RES-352: tuple literal — walk each element.
        Node::TupleLiteral { items, .. } => {
            for i in items {
                walk(i, bound, free);
            }
        }
        // RES-352: tuple destructuring — walk the RHS value; bind each
        // name in the pattern so downstream uses are not counted free.
        Node::LetDestructureTuple { names, value, .. } => {
            walk(value, bound, free);
            for n in names {
                bound.insert(n.clone());
            }
        }
        Node::StructLiteral { fields, .. } => {
            for (_, v) in fields {
                walk(v, bound, free);
            }
        }
        Node::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                walk(k, bound, free);
                walk(v, bound, free);
            }
        }
        Node::SetLiteral { items, .. } => {
            for i in items {
                walk(i, bound, free);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            walk(scrutinee, bound, free);
            for (pat, guard, body) in arms {
                let snapshot = bound.len();
                bind_pattern(pat, bound);
                if let Some(g) = guard {
                    walk(g, bound, free);
                }
                walk(body, bound, free);
                truncate_to(bound, snapshot);
            }
        }
    }
}

/// Add every `Pattern::Identifier(name)` inside `pat` to `bound`.
/// `Wildcard` and `Literal` don't bind. `Or` branches are required
/// to bind the same set of names (enforced elsewhere — RES-160), so
/// we just union what each branch introduces.
fn bind_pattern(pat: &Pattern, bound: &mut BTreeSet<String>) {
    match pat {
        Pattern::Identifier(name) => {
            bound.insert(name.clone());
        }
        Pattern::Wildcard | Pattern::Literal(_) => {}
        Pattern::Or(branches) => {
            for b in branches {
                bind_pattern(b, bound);
            }
        }
        // RES-161a: outer name + whatever the inner pattern binds.
        Pattern::Bind(outer, inner) => {
            bound.insert(outer.clone());
            bind_pattern(inner, bound);
        }
        Pattern::Struct { fields, .. } => {
            for (_, sub) in fields {
                bind_pattern(sub.as_ref(), bound);
            }
        }
    }
}

/// Prepass over `Program` statements: register every top-level
/// declaration name so sibling statements can forward-reference.
/// Mirrors what the interpreter does in its hoisting pass.
fn collect_top_level_binder(node: &Node, bound: &mut BTreeSet<String>) {
    match node {
        Node::Function { name, .. } => {
            bound.insert(name.clone());
        }
        Node::StructDecl { name, .. } => {
            bound.insert(name.clone());
        }
        Node::TypeAlias { name, .. } => {
            bound.insert(name.clone());
        }
        Node::RegionDecl { name, .. } => {
            // RES-391: region declarations introduce a compile-time
            // name (consumed by the borrow checker). No runtime
            // binding, but treat it like other declarations for the
            // scoping walk so sibling statements see the name.
            bound.insert(name.clone());
        }
        Node::ActorDecl { name, .. } => {
            bound.insert(name.clone());
        }
        Node::ClusterDecl { name, .. } => {
            bound.insert(name.clone());
        }
        Node::LetStatement { name, .. } | Node::StaticLet { name, .. } => {
            bound.insert(name.clone());
        }
        Node::LetDestructureStruct { fields, .. } => {
            for (_, local) in fields {
                bound.insert(local.clone());
            }
        }
        _ => {}
    }
}

/// `BTreeSet` has no `truncate`; we rebuild by draining the
/// stable-ordered set into a Vec, keeping the first `len` entries,
/// and reinserting. The sets are small (scope depth * per-frame
/// binders) so the cost is negligible.
fn truncate_to(set: &mut BTreeSet<String>, len: usize) {
    if set.len() <= len {
        return;
    }
    let drained: Vec<String> = set.iter().cloned().collect();
    set.clear();
    for name in drained.into_iter().take(len) {
        set.insert(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    fn ident(name: &str) -> Node {
        Node::Identifier {
            name: name.into(),
            span: Span::default(),
        }
    }

    fn int_lit(v: i64) -> Node {
        Node::IntegerLiteral {
            value: v,
            span: Span::default(),
        }
    }

    fn bool_lit(v: bool) -> Node {
        Node::BooleanLiteral {
            value: v,
            span: Span::default(),
        }
    }

    fn parse_program(src: &str) -> Node {
        let lexer = crate::Lexer::new(src.to_string());
        let mut parser = crate::Parser::new(lexer);
        parser.parse_program()
    }

    fn as_set<I: IntoIterator<Item = &'static str>>(names: I) -> BTreeSet<String> {
        names.into_iter().map(str::to_string).collect()
    }

    #[test]
    fn empty_program_has_no_free_vars() {
        let p = parse_program("");
        assert_eq!(free_vars(&p), BTreeSet::new());
    }

    #[test]
    fn literal_only_program_has_no_free_vars() {
        let p = parse_program("let x = 42;");
        assert_eq!(free_vars(&p), BTreeSet::new());
    }

    #[test]
    fn bare_identifier_reference_is_free() {
        // Hand-built AST: just `n` in expression position.
        let node = ident("n");
        assert_eq!(free_vars(&node), as_set(["n"]));
    }

    #[test]
    fn identifier_bound_by_let_is_not_free_downstream() {
        // Block: `let n = 5; n`
        let block = Node::Block {
            stmts: vec![
                Node::LetStatement {
                    name: "n".into(),
                    value: Box::new(int_lit(5)),
                    type_annot: None,
                    span: Span::default(),
                },
                Node::ExpressionStatement {
                    expr: Box::new(ident("n")),
                    span: Span::default(),
                },
            ],
            span: Span::default(),
        };
        assert_eq!(free_vars(&block), BTreeSet::new());
    }

    #[test]
    fn let_rhs_sees_outer_scope_not_the_binder_itself() {
        // `let x = x + 1;` — the RHS `x` is still free (outer x).
        let block = Node::Block {
            stmts: vec![Node::LetStatement {
                name: "x".into(),
                value: Box::new(Node::InfixExpression {
                    left: Box::new(ident("x")),
                    operator: "+".into(),
                    right: Box::new(int_lit(1)),
                    span: Span::default(),
                }),
                type_annot: None,
                span: Span::default(),
            }],
            span: Span::default(),
        };
        assert_eq!(free_vars(&block), as_set(["x"]));
    }

    #[test]
    fn fn_literal_captures_outer_name() {
        // `fn(x) { return x + n; }` — `n` is free.
        let lit = Node::FunctionLiteral {
            parameters: vec![("int".into(), "x".into())],
            body: Box::new(Node::Block {
                stmts: vec![Node::ReturnStatement {
                    value: Some(Box::new(Node::InfixExpression {
                        left: Box::new(ident("x")),
                        operator: "+".into(),
                        right: Box::new(ident("n")),
                        span: Span::default(),
                    })),
                    span: Span::default(),
                }],
                span: Span::default(),
            }),
            requires: Vec::new(),
            ensures: Vec::new(),
            recovers_to: None,
            return_type: None,
            span: Span::default(),
        };
        assert_eq!(free_vars(&lit), as_set(["n"]));
    }

    #[test]
    fn fn_literal_parameters_are_not_free_in_body() {
        // `fn(x) { return x; }` — nothing free.
        let lit = Node::FunctionLiteral {
            parameters: vec![("int".into(), "x".into())],
            body: Box::new(Node::Block {
                stmts: vec![Node::ReturnStatement {
                    value: Some(Box::new(ident("x"))),
                    span: Span::default(),
                }],
                span: Span::default(),
            }),
            requires: Vec::new(),
            ensures: Vec::new(),
            recovers_to: None,
            return_type: None,
            span: Span::default(),
        };
        assert_eq!(free_vars(&lit), BTreeSet::new());
    }

    #[test]
    fn fn_literal_captures_two_names() {
        // `fn(x) { return x + a + b; }` — `a`, `b` free.
        let lit = Node::FunctionLiteral {
            parameters: vec![("int".into(), "x".into())],
            body: Box::new(Node::Block {
                stmts: vec![Node::ReturnStatement {
                    value: Some(Box::new(Node::InfixExpression {
                        left: Box::new(Node::InfixExpression {
                            left: Box::new(ident("x")),
                            operator: "+".into(),
                            right: Box::new(ident("a")),
                            span: Span::default(),
                        }),
                        operator: "+".into(),
                        right: Box::new(ident("b")),
                        span: Span::default(),
                    })),
                    span: Span::default(),
                }],
                span: Span::default(),
            }),
            requires: Vec::new(),
            ensures: Vec::new(),
            recovers_to: None,
            return_type: None,
            span: Span::default(),
        };
        assert_eq!(free_vars(&lit), as_set(["a", "b"]));
    }

    #[test]
    fn for_in_binder_masks_loop_variable() {
        // `for x in xs { sum = sum + x; }` — `xs`, `sum` free; `x` bound.
        let stmt = Node::ForInStatement {
            name: "x".into(),
            iterable: Box::new(ident("xs")),
            body: Box::new(Node::Block {
                stmts: vec![Node::Assignment {
                    name: "sum".into(),
                    value: Box::new(Node::InfixExpression {
                        left: Box::new(ident("sum")),
                        operator: "+".into(),
                        right: Box::new(ident("x")),
                        span: Span::default(),
                    }),
                    span: Span::default(),
                }],
                span: Span::default(),
            }),
            invariants: Vec::new(),
            span: Span::default(),
        };
        assert_eq!(free_vars(&stmt), as_set(["sum", "xs"]));
    }

    #[test]
    fn match_identifier_pattern_binds_in_arm() {
        // `match n { y => y + outer }` — `n`, `outer` free; `y` bound.
        let m = Node::Match {
            scrutinee: Box::new(ident("n")),
            arms: vec![(
                Pattern::Identifier("y".into()),
                None,
                Node::InfixExpression {
                    left: Box::new(ident("y")),
                    operator: "+".into(),
                    right: Box::new(ident("outer")),
                    span: Span::default(),
                },
            )],
            span: Span::default(),
        };
        assert_eq!(free_vars(&m), as_set(["n", "outer"]));
    }

    #[test]
    fn match_wildcard_does_not_bind_scrutinee() {
        // `match n { _ => n }` — the `_` pattern doesn't introduce
        // a name, so `n` is referenced both as scrutinee and in
        // the arm body, and is free both times.
        let m = Node::Match {
            scrutinee: Box::new(ident("n")),
            arms: vec![(Pattern::Wildcard, None, ident("n"))],
            span: Span::default(),
        };
        assert_eq!(free_vars(&m), as_set(["n"]));
    }

    #[test]
    fn nested_closure_sees_both_scopes() {
        // `fn(a) { return fn(b) { return a + b + c; }; }`
        // Outer fn binds `a`; inner fn binds `b`; `c` is free.
        let inner = Node::FunctionLiteral {
            parameters: vec![("int".into(), "b".into())],
            body: Box::new(Node::Block {
                stmts: vec![Node::ReturnStatement {
                    value: Some(Box::new(Node::InfixExpression {
                        left: Box::new(Node::InfixExpression {
                            left: Box::new(ident("a")),
                            operator: "+".into(),
                            right: Box::new(ident("b")),
                            span: Span::default(),
                        }),
                        operator: "+".into(),
                        right: Box::new(ident("c")),
                        span: Span::default(),
                    })),
                    span: Span::default(),
                }],
                span: Span::default(),
            }),
            requires: Vec::new(),
            ensures: Vec::new(),
            recovers_to: None,
            return_type: None,
            span: Span::default(),
        };
        let outer = Node::FunctionLiteral {
            parameters: vec![("int".into(), "a".into())],
            body: Box::new(Node::Block {
                stmts: vec![Node::ReturnStatement {
                    value: Some(Box::new(inner)),
                    span: Span::default(),
                }],
                span: Span::default(),
            }),
            requires: Vec::new(),
            ensures: Vec::new(),
            recovers_to: None,
            return_type: None,
            span: Span::default(),
        };
        assert_eq!(free_vars(&outer), as_set(["c"]));
    }

    #[test]
    fn named_fn_body_can_reference_itself_without_being_free() {
        // `fn fact(int n) { return fact(n); }` — `fact` is not free.
        let src = r#"
            fn fact(int n) -> int {
                return fact(n);
            }
        "#;
        let p = parse_program(src);
        assert_eq!(free_vars(&p), BTreeSet::new());
    }

    #[test]
    fn assignment_to_unbound_name_counts_as_free() {
        // `sum = sum + 1;` — with no outer let, `sum` is free.
        let stmt = Node::Assignment {
            name: "sum".into(),
            value: Box::new(Node::InfixExpression {
                left: Box::new(ident("sum")),
                operator: "+".into(),
                right: Box::new(int_lit(1)),
                span: Span::default(),
            }),
            span: Span::default(),
        };
        assert_eq!(free_vars(&stmt), as_set(["sum"]));
    }

    #[test]
    fn condition_branches_walked_independently_of_bindings() {
        // `if flag { let x = 1; } else { let y = 2; }` — only
        // `flag` is free. x and y don't escape their arms.
        let stmt = Node::IfStatement {
            condition: Box::new(ident("flag")),
            consequence: Box::new(Node::Block {
                stmts: vec![Node::LetStatement {
                    name: "x".into(),
                    value: Box::new(int_lit(1)),
                    type_annot: None,
                    span: Span::default(),
                }],
                span: Span::default(),
            }),
            alternative: Some(Box::new(Node::Block {
                stmts: vec![Node::LetStatement {
                    name: "y".into(),
                    value: Box::new(int_lit(2)),
                    type_annot: None,
                    span: Span::default(),
                }],
                span: Span::default(),
            })),
            span: Span::default(),
        };
        assert_eq!(free_vars(&stmt), as_set(["flag"]));
    }

    #[test]
    fn parsed_fn_literal_with_capture_surfaces_it() {
        // End-to-end: feed real source through the parser, then
        // assert the captured set. Exercises the `use crate::Lexer`
        // + `crate::Parser` plumbing, not just hand-built nodes.
        let src = r#"
            fn make_adder(int n) -> int {
                let add = fn(int x) { return x + n; };
                return add(1);
            }
        "#;
        let program = parse_program(src);
        // Drill into the fn body → block → let → fn literal.
        let body = match &program {
            Node::Program(stmts) => match &stmts[0].node {
                Node::Function { body, .. } => &**body,
                other => panic!("expected fn, got {:?}", other),
            },
            _ => panic!("expected Program"),
        };
        let lit = match body {
            Node::Block { stmts, .. } => match &stmts[0] {
                Node::LetStatement { value, .. } => &**value,
                other => panic!("expected let, got {:?}", other),
            },
            other => panic!("expected Block, got {:?}", other),
        };
        // The inner literal captures `n`.
        let captures = free_vars(lit);
        assert_eq!(captures, as_set(["n"]));
    }

    #[test]
    fn free_vars_is_deterministic_across_runs() {
        // BTreeSet gives us stable iteration. The test checks the
        // *iteration order* matters: we build two different Node
        // trees that reference the same three names in different
        // orders and expect identical Vec<String> output.
        let a = Node::InfixExpression {
            left: Box::new(Node::InfixExpression {
                left: Box::new(ident("gamma")),
                operator: "+".into(),
                right: Box::new(ident("alpha")),
                span: Span::default(),
            }),
            operator: "+".into(),
            right: Box::new(ident("beta")),
            span: Span::default(),
        };
        let b = Node::InfixExpression {
            left: Box::new(Node::InfixExpression {
                left: Box::new(ident("alpha")),
                operator: "+".into(),
                right: Box::new(ident("beta")),
                span: Span::default(),
            }),
            operator: "+".into(),
            right: Box::new(ident("gamma")),
            span: Span::default(),
        };
        let fa: Vec<String> = free_vars(&a).into_iter().collect();
        let fb: Vec<String> = free_vars(&b).into_iter().collect();
        assert_eq!(fa, fb);
        assert_eq!(fa, vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn struct_literal_field_values_are_walked() {
        // `new Point { x: a, y: b }` — both `a` and `b` free.
        let lit = Node::StructLiteral {
            name: "Point".into(),
            fields: vec![("x".into(), ident("a")), ("y".into(), ident("b"))],
            span: Span::default(),
        };
        assert_eq!(free_vars(&lit), as_set(["a", "b"]));
    }

    #[test]
    fn invariants_on_while_are_walked() {
        // `while cond invariant (p >= 0) { ... }` — `cond`, `p`
        // both free. Regression check that RES-132a's new field
        // is actually visited.
        let stmt = Node::WhileStatement {
            condition: Box::new(ident("cond")),
            body: Box::new(Node::Block {
                stmts: Vec::new(),
                span: Span::default(),
            }),
            invariants: vec![Node::InfixExpression {
                left: Box::new(ident("p")),
                operator: ">=".into(),
                right: Box::new(int_lit(0)),
                span: Span::default(),
            }],
            span: Span::default(),
        };
        assert_eq!(free_vars(&stmt), as_set(["cond", "p"]));
    }

    #[test]
    fn bool_literal_alone_has_no_free_vars_sanity() {
        // Guard against regressions where leaf variants accidentally
        // get classified as identifiers.
        assert_eq!(free_vars(&bool_lit(true)), BTreeSet::new());
    }
}
