// RES-360: Closed-form invariant checker for live{} recovery bodies.
//
// V2 TLA+ verification requires recovery expressions to be closed-form
// (encodable as state transitions). This pass detects violations early:
// - Opaque FFI calls (malloc, strlen, etc.)
// - Direct recursion
// - Closures capturing from outer scope
//
// V1 emits warnings; V2 will escalate to errors under --v2-strict.

use crate::Node;
use std::collections::HashSet;

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1270 / RES-1916: the typechecker gates this call behind
    // `markers.has_live_block`, so the program is guaranteed to
    // contain at least one `LiveBlock`. The previous `any_node`
    // pre-scan was redundant — removed.
    let mut ctx = Context::new();
    ctx.collect_declarations(program);
    ctx.check_live_blocks(program);
    Ok(())
}

/// RES-2092: `extern_fns` and `current_fn` borrow `&'a str` directly from the
/// program AST instead of cloning. The two-pass design (`collect_declarations`
/// then `check_live_blocks`) walks the same `program` reference twice;
/// every name stored in the context already lives in the AST for the
/// duration of `check`. The previous shape allocated one `String` per
/// extern declaration plus one per visited `Function` node — wasted
/// work for a pass whose entire lifetime is bounded by a single
/// `&'a Node` argument.
struct Context<'a> {
    extern_fns: HashSet<&'a str>,
    current_fn: Option<&'a str>,
}

impl<'a> Context<'a> {
    fn new() -> Self {
        Context {
            extern_fns: HashSet::new(),
            current_fn: None,
        }
    }

    fn collect_declarations(&mut self, program: &'a Node) {
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
                    self.extern_fns.insert(decl.resilient_name.as_str());
                }
            }
        }
    }

    fn check_live_blocks(&mut self, program: &'a Node) {
        let Node::Program(statements) = program else {
            return;
        };
        for stmt in statements {
            self.walk_for_live_blocks(&stmt.node);
        }
    }

    fn walk_for_live_blocks(&mut self, node: &'a Node) {
        match node {
            Node::Function { name, body, .. } => {
                let prev_fn = self.current_fn.take();
                self.current_fn = Some(name.as_str());
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
            Node::Block { stmts, .. } => {
                for stmt in stmts {
                    self.walk_for_live_blocks(stmt);
                }
            }
            Node::IfStatement {
                condition,
                consequence,
                alternative,
                ..
            } => {
                self.walk_for_live_blocks(condition);
                self.walk_for_live_blocks(consequence);
                if let Some(alt) = alternative {
                    self.walk_for_live_blocks(alt);
                }
            }
            Node::WhileStatement {
                condition, body, ..
            } => {
                self.walk_for_live_blocks(condition);
                self.walk_for_live_blocks(body);
            }
            Node::ForInStatement { iterable, body, .. } => {
                self.walk_for_live_blocks(iterable);
                self.walk_for_live_blocks(body);
            }
            _ => {}
        }
    }

    fn check_node(&mut self, node: &'a Node) {
        match node {
            Node::CallExpression {
                function,
                arguments,
                ..
            } => {
                // RES-1511: borrow the callee identifier as `&str` instead
                // of cloning it. `extract_identifier` previously returned
                // an owned `String` so the warning branches could compare
                // against `current_fn`; both lookups
                // (`HashSet<String>::contains` and the equality check
                // against `self.current_fn`) accept a `&str` directly via
                // `Borrow<str>`, so the clone-per-call-site was wasted.
                if let Some(fn_name) = extract_identifier(function) {
                    if self.extern_fns.contains(fn_name) {
                        eprintln!(
                            "warning: opaque FFI call to '{}' in recovery body \
                            cannot be modeled as TLA+ action",
                            fn_name
                        );
                    } else if let Some(current) = self.current_fn
                        && fn_name == current
                    {
                        eprintln!(
                            "warning: function '{}' recursively calls itself in recovery body",
                            current
                        );
                    }
                }
                for arg in arguments {
                    self.check_node(arg);
                }
            }
            Node::FunctionLiteral { .. } => {
                let free = crate::free_vars::free_vars(node);
                if !free.is_empty() {
                    let captured: Vec<_> = free.iter().cloned().collect();
                    eprintln!(
                        "warning: closure in recovery body captures [{}] from outer scope",
                        captured.join(", ")
                    );
                }
            }
            Node::Block { stmts, .. } => {
                for stmt in stmts {
                    self.check_node(stmt);
                }
            }
            Node::LetStatement { value, .. } => {
                self.check_node(value);
            }
            Node::Assignment { value, .. } => {
                self.check_node(value);
            }
            Node::Assert { condition, .. } => {
                self.check_node(condition);
            }
            Node::Assume { condition, .. } => {
                self.check_node(condition);
            }
            Node::IfStatement {
                condition,
                consequence,
                alternative,
                ..
            } => {
                self.check_node(condition);
                self.check_node(consequence);
                if let Some(alt) = alternative {
                    self.check_node(alt);
                }
            }
            Node::WhileStatement {
                condition, body, ..
            } => {
                self.check_node(condition);
                self.check_node(body);
            }
            Node::ForInStatement { iterable, body, .. } => {
                self.check_node(iterable);
                self.check_node(body);
            }
            Node::InfixExpression { left, right, .. } => {
                self.check_node(left);
                self.check_node(right);
            }
            Node::PrefixExpression { right, .. } => {
                self.check_node(right);
            }
            Node::IndexExpression { target, index, .. } => {
                self.check_node(target);
                self.check_node(index);
            }
            Node::FieldAccess { target, .. } => {
                self.check_node(target);
            }
            Node::ArrayLiteral { items, .. } => {
                for elem in items {
                    self.check_node(elem);
                }
            }
            Node::ExpressionStatement { expr, .. } => {
                self.check_node(expr);
            }
            _ => {}
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
}
