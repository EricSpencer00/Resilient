// RES-360: Closed-form invariant checker for live{} recovery bodies.
//
// V2 TLA+ verification requires recovery expressions to be closed-form
// (encodable as state transitions). This pass detects violations early:
// - Opaque FFI calls (malloc, strlen, etc.)
// - Direct recursion
// - Closures capturing from outer scope
//
// V1 emits warnings; V2 will escalate to errors under --v2-strict.

use crate::{ChainAccess, Node};
use std::collections::HashSet;

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    for warning in collect_warnings(program) {
        crate::typechecker::emit_check_warning_plain(warning, source_path, "recovery");
    }
    Ok(())
}

fn collect_warnings(program: &Node) -> Vec<String> {
    // RES-1270 / RES-1916: the typechecker gates this call behind
    // `markers.has_live_block`, so the program is guaranteed to
    // contain at least one `LiveBlock`. Keeping the traversal here
    // independent of the marker also makes this pass testable when a
    // caller constructs a nested live block directly.
    let mut ctx = Context::new();
    ctx.collect_declarations(program);
    ctx.check_live_blocks(program);
    ctx.warnings
}

struct Context {
    extern_fns: HashSet<String>,
    current_fn: Option<String>,
    warnings: Vec<String>,
}

impl Context {
    fn new() -> Self {
        Context {
            extern_fns: HashSet::new(),
            current_fn: None,
            warnings: Vec::new(),
        }
    }

    fn collect_declarations(&mut self, program: &Node) {
        let Node::Program(statements) = program else {
            return;
        };
        // RES-1774: pre-size to the exact extern-decl count. Each
        // Extern statement contributes `decls.len()` inserts, so
        // summing them up front avoids the 0→4→8→… doubling chain
        // as we walk a program with many extern declarations.
        let extern_decl_count: usize = statements
            .iter()
            .filter_map(|s| match &s.node {
                Node::Extern { decls, .. } => Some(decls.len()),
                _ => None,
            })
            .sum();
        self.extern_fns.reserve(extern_decl_count);
        for stmt in statements {
            if let Node::Extern { decls, .. } = &stmt.node {
                for decl in decls {
                    self.extern_fns.insert(decl.resilient_name.clone());
                }
            }
        }
    }

    fn check_live_blocks(&mut self, program: &Node) {
        let Node::Program(statements) = program else {
            return;
        };
        for stmt in statements {
            self.walk_for_live_blocks(&stmt.node);
        }
    }

    fn walk_for_live_blocks(&mut self, node: &Node) {
        match node {
            Node::Function { name, body, .. } => {
                let prev_fn = self.current_fn.take();
                self.current_fn = Some(name.clone());
                self.walk_for_live_blocks(body);
                self.current_fn = prev_fn;
            }
            Node::LiveBlock {
                body, invariants, ..
            } => {
                self.check_node(body);
                for inv in invariants {
                    self.check_node(inv);
                }
            }
            _ => for_each_child(node, &mut |child| self.walk_for_live_blocks(child)),
        }
    }

    fn check_node(&mut self, node: &Node) {
        match node {
            Node::CallExpression { function, .. } => {
                // RES-1511: borrow the callee identifier as `&str` instead
                // of cloning it. `extract_identifier` previously returned
                // an owned `String` so the warning branches could compare
                // against `current_fn`; both lookups
                // (`HashSet<String>::contains` and the equality check
                // against `self.current_fn`) accept a `&str` directly via
                // `Borrow<str>`, so the clone-per-call-site was wasted.
                if let Some(fn_name) = extract_identifier(function) {
                    if self.extern_fns.contains(fn_name) {
                        let plain = format!(
                            "warning: opaque FFI call to '{}' in recovery body \
                             cannot be modeled as TLA+ action",
                            fn_name
                        );
                        self.warnings.push(plain);
                    } else if let Some(current) = self.current_fn.as_deref()
                        && fn_name == current
                    {
                        let plain = format!(
                            "warning: function '{}' recursively calls itself in recovery body",
                            current
                        );
                        self.warnings.push(plain);
                    }
                }
            }
            Node::FunctionLiteral { .. } => {
                let free = crate::free_vars::free_vars(node);
                if !free.is_empty() {
                    let captured: Vec<_> = free.iter().cloned().collect();
                    let plain = format!(
                        "warning: closure in recovery body captures [{}] from outer scope",
                        captured.join(", ")
                    );
                    self.warnings.push(plain);
                }
            }
            _ => {}
        }
        for_each_child(node, &mut |child| self.check_node(child));
    }
}

/// Visit every AST child that can contain executable code.
///
/// Recovery checking used to hand-maintain a short list of statement
/// variants. That made a live block nested in a match arm, tuple, map,
/// closure, or another structured expression invisible to the checker.
/// Keeping the child walk in one place makes new expression containers
/// fail closed by construction: the checker can only miss a new variant
/// when it is deliberately left out of this exhaustive list.
fn for_each_child(node: &Node, visit: &mut impl FnMut(&Node)) {
    match node {
        Node::Program(statements) => {
            for statement in statements {
                visit(&statement.node);
            }
        }
        Node::Function {
            defaults,
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            for default in defaults.iter().flatten() {
                visit(default);
            }
            visit(body);
            for clause in requires {
                visit(clause);
            }
            for clause in ensures {
                visit(clause);
            }
            if let Some(clause) = recovers_to {
                visit(clause);
            }
        }
        Node::LiveBlock {
            body,
            invariants,
            timeout,
            ..
        } => {
            visit(body);
            for invariant in invariants {
                visit(invariant);
            }
            if let Some(timeout) = timeout {
                visit(timeout);
            }
        }
        Node::Assert {
            condition, message, ..
        }
        | Node::Assume {
            condition, message, ..
        } => {
            visit(condition);
            if let Some(message) = message {
                visit(message);
            }
        }
        Node::Block { stmts, .. } => {
            for stmt in stmts {
                visit(stmt);
            }
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::Const { value, .. }
        | Node::Assignment { value, .. } => visit(value),
        Node::ReturnStatement { value, .. } => {
            if let Some(value) = value {
                visit(value);
            }
        }
        Node::LetDestructureStruct { value, .. }
        | Node::LetTupleDestructure { value, .. }
        | Node::NewtypeConstruct { value, .. }
        | Node::DeferStatement { expr: value, .. }
        | Node::BreakWith { value, .. }
        | Node::UnsafeBlock { body: value, .. }
        | Node::BenchBlock { body: value, .. } => visit(value),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            visit(condition);
            visit(consequence);
            if let Some(alternative) = alternative {
                visit(alternative);
            }
        }
        Node::WhileStatement {
            condition,
            body,
            invariants,
            ..
        }
        | Node::ForInStatement {
            iterable: condition,
            body,
            invariants,
            ..
        } => {
            visit(condition);
            visit(body);
            for invariant in invariants {
                visit(invariant);
            }
        }
        Node::ExpressionStatement { expr, .. }
        | Node::TryExpression { expr, .. }
        | Node::FieldAccess { target: expr, .. }
        | Node::TupleIndex { tuple: expr, .. }
        | Node::InvariantStatement { expr, .. } => visit(expr),
        Node::PrefixExpression { right, .. } => visit(right),
        Node::InfixExpression { left, right, .. } => {
            visit(left);
            visit(right);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            visit(function);
            for argument in arguments {
                visit(argument);
            }
        }
        Node::OptionalChain { object, access, .. } => {
            visit(object);
            if let ChainAccess::Method(_, arguments) = access {
                for argument in arguments {
                    visit(argument);
                }
            }
        }
        Node::FunctionLiteral {
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            visit(body);
            for clause in requires {
                visit(clause);
            }
            for clause in ensures {
                visit(clause);
            }
            if let Some(clause) = recovers_to {
                visit(clause);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            visit(scrutinee);
            for (_, guard, body) in arms {
                if let Some(guard) = guard {
                    visit(guard);
                }
                visit(body);
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            if let Some(base) = base {
                visit(base);
            }
            for (_, value) in fields {
                visit(value);
            }
        }
        Node::FieldAssignment { target, value, .. }
        | Node::IndexAssignment { target, value, .. } => {
            visit(target);
            visit(value);
        }
        Node::ArrayLiteral { items, .. }
        | Node::SetLiteral { items, .. }
        | Node::TupleLiteral { items, .. } => {
            for item in items {
                visit(item);
            }
        }
        Node::IndexExpression { target, index, .. } => {
            visit(target);
            visit(index);
        }
        Node::Slice { target, lo, hi, .. } => {
            visit(target);
            if let Some(lo) = lo {
                visit(lo);
            }
            if let Some(hi) = hi {
                visit(hi);
            }
        }
        Node::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                visit(key);
                visit(value);
            }
        }
        Node::TryCatch { body, handlers, .. } => {
            for stmt in body {
                visit(stmt);
            }
            for (_, handler_body) in handlers {
                for stmt in handler_body {
                    visit(stmt);
                }
            }
        }
        Node::Quantifier { range, body, .. } => {
            match range {
                crate::quantifiers::QuantRange::Range { lo, hi } => {
                    visit(lo);
                    visit(hi);
                }
                crate::quantifiers::QuantRange::Iterable(iterable) => visit(iterable),
            }
            visit(body);
        }
        Node::Range { lo, hi, .. } => {
            visit(lo);
            visit(hi);
        }
        Node::NamedArg { value, .. } => visit(value),
        Node::InterpolatedString { parts, .. } => {
            for part in parts {
                if let crate::string_interp::StringPart::Expr(expr) = part {
                    visit(expr);
                }
            }
        }
        Node::ModuleDecl { body, .. } => {
            for item in body {
                visit(item);
            }
        }
        Node::ImplBlock { methods, .. } | Node::BlanketImpl { methods, .. } => {
            for method in methods {
                visit(method);
            }
        }
        Node::Actor {
            state_init,
            concurrent_ensures,
            handlers,
            ..
        } => {
            visit(state_init);
            for clause in concurrent_ensures {
                visit(clause);
            }
            for handler in handlers {
                for clause in &handler.ensures {
                    visit(clause);
                }
                visit(&handler.body);
            }
        }
        Node::ActorDecl {
            state_fields,
            always_clauses,
            eventually_clauses,
            receive_handlers,
            handlers,
            ..
        } => {
            for (_, _, initializer) in state_fields {
                visit(initializer);
            }
            for clause in always_clauses {
                visit(clause);
            }
            for clause in eventually_clauses {
                visit(&clause.post);
            }
            for handler in receive_handlers {
                for clause in &handler.requires {
                    visit(clause);
                }
                for clause in &handler.ensures {
                    visit(clause);
                }
                visit(&handler.body);
            }
            for handler in handlers {
                for clause in &handler.ensures {
                    visit(clause);
                }
                visit(&handler.body);
            }
        }
        Node::ClusterDecl { invariants, .. } => {
            for invariant in invariants {
                visit(invariant);
            }
        }
        Node::StaticAssert { condition, .. } => visit(condition),
        Node::NewtypeDecl { .. }
        | Node::Use { .. }
        | Node::Extern { .. }
        | Node::DurationLiteral { .. }
        | Node::Break { .. }
        | Node::Continue { .. }
        | Node::BreakLabel { .. }
        | Node::ContinueLabel { .. }
        | Node::Identifier { .. }
        | Node::IntegerLiteral { .. }
        | Node::FloatLiteral { .. }
        | Node::StringLiteral { .. }
        | Node::StringInternLiteral { .. }
        | Node::BytesLiteral { .. }
        | Node::CharLiteral { .. }
        | Node::BooleanLiteral { .. }
        | Node::StructDecl { .. }
        | Node::TypeAlias { .. }
        | Node::RegionDecl { .. }
        | Node::TraitDecl { .. }
        | Node::EnumDecl { .. }
        | Node::RegionParam { .. }
        | Node::SupervisorDecl { .. } => {}
    }
}

/// RES-1511: borrow the identifier name out of the AST node instead of
/// cloning it. The previous shape returned `Option<String>` so every
/// call site paid a `to_string()` for a value only used as a lookup key
/// or for a single `eprintln!`.
fn extract_identifier(node: &Node) -> Option<&str> {
    match node {
        Node::Identifier { name, .. } => Some(name.as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn no_live_block_returns_ok() {
        let src = "fn f(int x) -> int { return x + 1; }\nf(5);\n";
        let (prog, _) = parse(src);
        assert!(
            check(&prog, "test").is_ok(),
            "check must return Ok for programs with no live blocks"
        );
    }

    #[test]
    fn live_block_with_simple_arithmetic_passes() {
        let src = "fn f(int x) -> int {\n    live {\n        let y = x + 1;\n        return y;\n    }\n}\nf(5);\n";
        let (prog, _) = parse(src);
        assert!(
            check(&prog, "test").is_ok(),
            "live block with pure arithmetic should pass (V1 only warns)"
        );
    }

    #[test]
    fn empty_program_returns_ok() {
        let (prog, _) = parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn live_block_with_extern_call_still_returns_ok() {
        // V1: only warns, does not return Err
        let src = "extern { fn malloc(int n) -> int; }\nfn f(int x) -> int {\n    live { return malloc(x); }\n}\nf(5);\n";
        let (prog, _) = parse(src);
        assert!(
            check(&prog, "test").is_ok(),
            "V1 checker emits warnings but always returns Ok"
        );
    }

    #[test]
    fn live_block_with_direct_recursion_still_returns_ok() {
        let src = "fn f(int n) -> int {\n    live { if (n > 0) { return f(n - 1); } }\n}\nf(5);\n";
        let (prog, _) = parse(src);
        assert!(
            check(&prog, "test").is_ok(),
            "V1 checker only warns on recursion in recovery"
        );
    }

    #[test]
    fn live_block_with_closure_capture_still_returns_ok() {
        let src = "fn f(int x) -> int {\n    live {\n        let g = fn(int y) { return y + x; };\n        return g(1);\n    }\n}\nf(5);\n";
        let (prog, _) = parse(src);
        assert!(
            check(&prog, "test").is_ok(),
            "V1 warns on captured closures but returns Ok"
        );
    }

    #[test]
    fn multiple_extern_calls_all_warned() {
        let src = "extern { fn malloc(int n) -> int; fn free(int p) -> int; }\nfn f() {\n    live {\n        let p = malloc(10);\n        let _ = free(p);\n    }\n}\nf();\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn nested_match_live_block_is_checked() {
        let src = r#"extern "test" { fn malloc(int n) -> int; }
fn f(int x) -> int {
    return match x {
        0 => { live { return malloc(1); } return 0; },
        _ => 0,
    };
}
f(0);
"#;
        let (prog, errors) = parse(src);
        assert!(errors.is_empty(), "unexpected parse errors: {errors:?}");

        let warnings = collect_warnings(&prog);
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("opaque FFI call to 'malloc'")),
            "nested live block was not checked: {warnings:?}"
        );
    }
}
