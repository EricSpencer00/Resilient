// RES-360: Closed-form invariant checker for live{} recovery bodies.
//
// V2 TLA+ verification requires recovery expressions to be closed-form
// (encodable as state transitions). This pass detects violations early:
// - Opaque FFI calls (malloc, strlen, etc.)
// - Direct recursion
// - Closures capturing from outer scope
//
// V1 emits warnings; V2 will escalate to errors under --v2-strict.

use crate::{Node, Pattern};
use std::collections::HashSet;

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    // RES-1270 / RES-1916: the typechecker gates this call behind
    // `markers.has_live_block`, so the program is guaranteed to
    // contain at least one `LiveBlock`. The previous `any_node`
    // pre-scan was redundant — removed.
    let mut ctx = Context::new(source_path);
    ctx.collect_declarations(program);
    ctx.check_live_blocks(program);
    Ok(())
}

struct Context {
    source_path: String,
    extern_fns: HashSet<String>,
    current_fn: Option<String>,
}

impl Context {
    fn new(source_path: &str) -> Self {
        Context {
            source_path: source_path.to_string(),
            extern_fns: HashSet::new(),
            current_fn: None,
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
            Node::Function { name, .. } => {
                let prev_fn = self.current_fn.take();
                self.current_fn = Some(name.clone());
                Self::for_each_child(node, |child| self.walk_for_live_blocks(child));
                self.current_fn = prev_fn;
            }
            Node::LiveBlock {
                body,
                invariants,
                timeout,
                ..
            } => {
                self.check_node(body);
                for inv in invariants {
                    self.check_node(inv);
                }
                if let Some(timeout) = timeout {
                    self.check_node(timeout);
                }
            }
            _ => Self::for_each_child(node, |child| self.walk_for_live_blocks(child)),
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
                        crate::typechecker::emit_check_warning_plain(
                            plain,
                            &self.source_path,
                            "recovery",
                        );
                    } else if let Some(current) = self.current_fn.as_deref()
                        && fn_name == current
                    {
                        let plain = format!(
                            "warning: function '{}' recursively calls itself in recovery body",
                            current
                        );
                        crate::typechecker::emit_check_warning_plain(
                            plain,
                            &self.source_path,
                            "recovery",
                        );
                    }
                }
                Self::for_each_child(node, |child| self.check_node(child));
            }
            Node::FunctionLiteral { .. } => {
                let free = crate::free_vars::free_vars(node);
                if !free.is_empty() {
                    let captured: Vec<_> = free.iter().cloned().collect();
                    let plain = format!(
                        "warning: closure in recovery body captures [{}] from outer scope",
                        captured.join(", ")
                    );
                    crate::typechecker::emit_check_warning_plain(
                        plain,
                        &self.source_path,
                        "recovery",
                    );
                }
                Self::for_each_child(node, |child| self.check_node(child));
            }
            Node::Function { name, .. } => {
                let prev_fn = self.current_fn.take();
                self.current_fn = Some(name.clone());
                Self::for_each_child(node, |child| self.check_node(child));
                self.current_fn = prev_fn;
            }
            _ => Self::for_each_child(node, |child| self.check_node(child)),
        }
    }

    /// Visit every AST child that can contain an expression or statement.
    ///
    /// The recovery checker used to hand-list only blocks, conditionals,
    /// loops, calls, and a few literals. That made safety warnings depend on
    /// syntax shape: a call hidden in a match arm or a defer body was silently
    /// omitted. Keeping the child enumeration in one place makes new
    /// expression-bearing containers visible to both the live-block finder
    /// and the closed-form checker.
    fn for_each_child(node: &Node, mut visit: impl FnMut(&Node)) {
        match node {
            Node::Program(statements) => {
                for statement in statements {
                    visit(&statement.node);
                }
            }
            Node::Extern { decls, .. } => {
                for decl in decls {
                    for expr in &decl.requires {
                        visit(expr);
                    }
                    for expr in &decl.ensures {
                        visit(expr);
                    }
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
                for expr in requires {
                    visit(expr);
                }
                for expr in ensures {
                    visit(expr);
                }
                if let Some(expr) = recovers_to {
                    visit(expr);
                }
            }
            Node::LiveBlock {
                body,
                invariants,
                timeout,
                ..
            } => {
                visit(body);
                for expr in invariants {
                    visit(expr);
                }
                if let Some(expr) = timeout {
                    visit(expr);
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
            | Node::Assignment { value, .. }
            | Node::BreakWith { value, .. }
            | Node::NamedArg { value, .. }
            | Node::NewtypeConstruct { value, .. } => visit(value),
            Node::ReturnStatement { value, .. } => {
                if let Some(value) = value {
                    visit(value);
                }
            }
            Node::DeferStatement { expr, .. }
            | Node::InvariantStatement { expr, .. }
            | Node::TryExpression { expr, .. } => visit(expr),
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
            | Node::UnsafeBlock { body: expr, .. }
            | Node::BenchBlock { body: expr, .. } => visit(expr),
            Node::PrefixExpression { right, .. }
            | Node::FieldAccess { target: right, .. }
            | Node::TupleIndex { tuple: right, .. } => visit(right),
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
                if let crate::ChainAccess::Method(_, arguments) = access {
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
                for expr in requires {
                    visit(expr);
                }
                for expr in ensures {
                    visit(expr);
                }
                if let Some(expr) = recovers_to {
                    visit(expr);
                }
            }
            Node::Match {
                scrutinee, arms, ..
            } => {
                visit(scrutinee);
                for (pattern, guard, body) in arms {
                    Self::for_each_pattern_child(pattern, &mut visit);
                    if let Some(guard) = guard {
                        visit(guard);
                    }
                    visit(body);
                }
            }
            Node::LetDestructureStruct { value, .. } | Node::LetTupleDestructure { value, .. } => {
                visit(value)
            }
            Node::StructLiteral { fields, base, .. } => {
                for (_, value) in fields {
                    visit(value);
                }
                if let Some(base) = base {
                    visit(base);
                }
            }
            Node::FieldAssignment { target, value, .. } => {
                visit(target);
                visit(value);
            }
            Node::IndexAssignment {
                target,
                index,
                value,
                ..
            } => {
                visit(target);
                visit(index);
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
                for expr in concurrent_ensures {
                    visit(expr);
                }
                for handler in handlers {
                    for expr in &handler.ensures {
                        visit(expr);
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
                for expr in always_clauses {
                    visit(expr);
                }
                for clause in eventually_clauses {
                    visit(&clause.post);
                }
                for handler in receive_handlers {
                    for expr in &handler.requires {
                        visit(expr);
                    }
                    for expr in &handler.ensures {
                        visit(expr);
                    }
                    visit(&handler.body);
                }
                for handler in handlers {
                    for expr in &handler.ensures {
                        visit(expr);
                    }
                    visit(&handler.body);
                }
            }
            Node::ClusterDecl { invariants, .. } => {
                for invariant in invariants {
                    visit(invariant);
                }
            }
            Node::TryCatch { body, handlers, .. } => {
                for stmt in body {
                    visit(stmt);
                }
                for (_, stmts) in handlers {
                    for stmt in stmts {
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
            Node::StaticAssert { condition, .. } => visit(condition),
            _ => {}
        }
    }

    fn for_each_pattern_child(pattern: &Pattern, visit: &mut impl FnMut(&Node)) {
        match pattern {
            Pattern::Literal(node) => visit(node),
            Pattern::Or(patterns)
            | Pattern::Tuple(patterns)
            | Pattern::TupleStruct {
                fields: patterns, ..
            } => {
                for pattern in patterns {
                    Self::for_each_pattern_child(pattern, visit);
                }
            }
            Pattern::Bind(_, pattern)
            | Pattern::Some(pattern)
            | Pattern::Ok(pattern)
            | Pattern::Err(pattern) => Self::for_each_pattern_child(pattern, visit),
            Pattern::Struct { fields, .. } => {
                for (_, pattern) in fields {
                    Self::for_each_pattern_child(pattern, visit);
                }
            }
            Pattern::EnumVariant { payload, .. } => match payload {
                crate::EnumPatternPayload::None => {}
                crate::EnumPatternPayload::Named(fields) => {
                    for (_, pattern) in fields {
                        Self::for_each_pattern_child(pattern, visit);
                    }
                }
                crate::EnumPatternPayload::Tuple(patterns) => {
                    for pattern in patterns {
                        Self::for_each_pattern_child(pattern, visit);
                    }
                }
            },
            Pattern::Identifier(_) | Pattern::Wildcard | Pattern::Range { .. } | Pattern::None => {}
        }
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

    fn recovery_diagnostics(src: &str) -> Vec<crate::typechecker::CheckDiagnostic> {
        let (prog, errors) = parse(src);
        assert!(errors.is_empty(), "source must parse: {errors:?}");
        let (result, diagnostics) =
            crate::typechecker::collect_check_diagnostics(|| check(&prog, "test"));
        assert!(result.is_ok());
        diagnostics
    }

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
    fn opaque_call_inside_match_arm_is_reported() {
        let src = "extern { fn malloc(int n) -> int; }\nfn f(int x) -> int {\n    live { return match x { 0 => malloc(x), _ => x, }; }\n}\nf(5);\n";
        let diagnostics = recovery_diagnostics(src);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.plain.contains("opaque FFI call to 'malloc'"))
        );
    }

    #[test]
    fn recursive_call_inside_try_handler_is_reported() {
        let src = "fn f(int x) -> int {\n    live {\n        try { return x; } catch Timeout { return f(x); }\n    }\n}\nf(5);\n";
        let diagnostics = recovery_diagnostics(src);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .plain
                .contains("function 'f' recursively calls itself")
        }));
    }
}
