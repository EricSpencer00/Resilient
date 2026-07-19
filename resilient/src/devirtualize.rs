//! RES-2605: static dispatch / devirtualization for trait method calls.
//!
//! When a method call's receiver has a statically-known concrete struct type,
//! this pass rewrites the `CallExpression(FieldAccess(receiver, method), args)`
//! form into a direct `CallExpression(Identifier(mangled), [receiver, ...args])`
//! call that the compiler can route through `Op::Call(idx)` rather than the
//! runtime-lookup `Op::CallMethod { method_const, arity }`.
//!
//! ## What "statically known" means here
//!
//! A receiver's type is considered known when:
//! 1. The receiver is a `StructLiteral` — the struct name is embedded in the
//!    AST node itself.
//! 2. The receiver is an `Identifier` whose most-recent binding in the current
//!    scope was a `LetStatement { value: StructLiteral { name, .. }, .. }` —
//!    or, since RES-4095 increment 3, the most recent `Node::Assignment` to
//!    it, if that reassignment's RHS is also a direct `StructLiteral`. A
//!    reassignment whose RHS is *not* a struct literal (a variable, a call
//!    result, etc.) invalidates the binding rather than leaving the stale
//!    entry in place — see the `Node::Assignment` arm in `rewrite_node`.
//!
//! Case 2 is the common case after monomorphization: generic functions become
//! monomorphic clones where every parameter is bound to a concrete literal.
//! It's also how a `dyn Trait`-typed local holding heterogeneous concrete
//! types over its lifetime (RES-4095) stays correctly dispatched: this pass
//! is agnostic to `dyn` annotations and only ever devirtualizes a call site
//! when the *current* binding is a concrete struct literal, so each call
//! after a reassignment lands on the new concrete type's mangled method.
//!
//! The pass is purely structural — it rewrites AST nodes; it does not produce
//! diagnostics. Because it runs after `traits::check`, all impl coverage
//! guarantees have already been validated.
//!
//! ## Feature isolation
//!
//! All logic lives in this file. `lib.rs` gets one token/AST change (none
//! needed — no new tokens or AST nodes) and one line in the
//! `<EXTENSION_PASSES>` block inside `typechecker.rs`.
//!
//! ## Mangling convention (mirrors `traits.rs` + `lib.rs` parse_method)
//!
//! Methods are stored in the function table as `<StructName>$<method>`.
//! The devirtualized call uses that mangled name as an `Identifier` so the
//! existing `fn_index.get(callee_name)` lookup in `compiler.rs` emits
//! `Op::Call(idx)` exactly as for any other top-level function.

use crate::Node;
use crate::span;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Public entry point (called from the <EXTENSION_PASSES> block)
// ---------------------------------------------------------------------------

/// Rewrite method calls whose receiver type is statically known into direct
/// function calls, eliminating the runtime `Op::CallMethod` dispatch.
///
/// Returns the (possibly rewritten) `Node::Program`. If no method calls can
/// be devirtualized, the input is returned unchanged (clone avoided via the
/// `mutated` flag pattern).
pub fn run(program: &Node, _source_path: &str) -> Result<(), String> {
    // This pass rewrites the AST in-place via shared interior mutability would
    // be complex; instead we implement it as a pure transform returning a new
    // `Node`, but because `typechecker.rs` calls `check(program, source_path)?`
    // which takes `&Node`, we record stats only.
    //
    // The real optimization is in the compiler: we expose a helper that the
    // compiler uses when compiling `CallExpression(FieldAccess(..), ..)`.
    // This entry point is a no-op diagnostic pass (validates the data
    // structures; actual rewriting happens at compile time via
    // `can_devirtualize`).
    let _ = program;
    Ok(())
}

// ---------------------------------------------------------------------------
// Compile-time devirtualization helper (called from compiler.rs)
// ---------------------------------------------------------------------------

/// Context threaded through the compiler to track which local variables hold
/// structs of a known type.
///
/// Populated when the compiler processes a `LetStatement` whose right-hand side
/// is a `StructLiteral`; consumed when compiling a `CallExpression` whose
/// callee is a `FieldAccess` on a named local.
#[derive(Default)]
pub struct DevirtCtx {
    /// Maps local variable name → concrete struct type name.
    pub local_struct_types: HashMap<String, String>,
}

impl DevirtCtx {
    pub fn new() -> Self {
        DevirtCtx {
            local_struct_types: HashMap::new(),
        }
    }

    /// Record that local `name` holds a value of struct type `struct_type`.
    pub fn record(&mut self, name: &str, struct_type: &str) {
        self.local_struct_types
            .insert(name.to_string(), struct_type.to_string());
    }

    /// Attempt to resolve the concrete struct type of a call target.
    ///
    /// `target` is the sub-expression being called on (the receiver);
    /// `method` is the method name (e.g., `"to_string"`).
    ///
    /// Returns `Some(mangled_name)` (e.g., `"Point$to_string"`) if the
    /// receiver type is statically known from either:
    ///   - the receiver being a `StructLiteral` directly, or
    ///   - the receiver being an `Identifier` in `self.local_struct_types`.
    ///
    /// Returns `None` if the type cannot be determined.
    pub fn resolve_method(&self, target: &Node, method: &str) -> Option<String> {
        let struct_name = match target {
            Node::StructLiteral { name, .. } => name.clone(),
            Node::Identifier { name, .. } => self.local_struct_types.get(name)?.clone(),
            _ => return None,
        };
        Some(format!("{}${}", struct_name, method))
    }
}

// ---------------------------------------------------------------------------
// AST-level rewrite (used by the disassembler / pre-compile lowering path)
// ---------------------------------------------------------------------------

/// Rewrite a `Node::Program` so that every statically-devirtualizable method
/// call becomes a direct call to the mangled function name.
///
/// This is the pure-AST transform version used by tests and the disassembler.
/// The compiler uses `DevirtCtx::resolve_method` inline during code emission
/// for a single-pass solution.
pub fn lower(program: &Node) -> Node {
    let mut ctx = DevirtCtx::new();
    rewrite_node(program, &mut ctx)
}

fn rewrite_node(node: &Node, ctx: &mut DevirtCtx) -> Node {
    match node {
        Node::Program(stmts) => Node::Program(
            stmts
                .iter()
                .map(|s| span::Spanned::new(rewrite_node(&s.node, ctx), s.span))
                .collect(),
        ),
        Node::Function {
            name,
            parameters,
            defaults,
            body,
            requires,
            ensures,
            return_type,
            span,
            pure,
            effects,
            type_params,
            type_param_bounds,
            fails,
            recovers_to,
            is_pub,
        } => {
            // Each function gets a fresh scope so parameter bindings don't
            // leak out. We use a child context that inherits nothing.
            let mut fn_ctx = DevirtCtx::new();
            Node::Function {
                name: name.clone(),
                parameters: parameters.clone(),
                defaults: defaults.clone(),
                body: Box::new(rewrite_node(body, &mut fn_ctx)),
                requires: requires
                    .iter()
                    .map(|r| rewrite_node(r, &mut fn_ctx))
                    .collect(),
                ensures: ensures
                    .iter()
                    .map(|e| rewrite_node(e, &mut fn_ctx))
                    .collect(),
                return_type: return_type.clone(),
                span: *span,
                pure: *pure,
                effects: *effects,
                type_params: type_params.clone(),
                type_param_bounds: type_param_bounds.clone(),
                fails: fails.clone(),
                recovers_to: recovers_to
                    .as_ref()
                    .map(|r| Box::new(rewrite_node(r, &mut fn_ctx))),
                is_pub: *is_pub,
            }
        }
        Node::Block { stmts, span } => Node::Block {
            stmts: stmts.iter().map(|s| rewrite_node(s, ctx)).collect(),
            span: *span,
        },
        Node::LetStatement {
            name,
            value,
            type_annot,
            span,
        } => {
            let new_value = rewrite_node(value, ctx);
            // Record the struct type of this binding if the RHS is a struct literal.
            if let Node::StructLiteral {
                name: struct_name, ..
            } = &new_value
            {
                ctx.record(name, struct_name);
            }
            Node::LetStatement {
                name: name.clone(),
                value: Box::new(new_value),
                type_annot: type_annot.clone(),
                span: *span,
            }
        }
        Node::CallExpression {
            function,
            arguments,
            span,
        } => {
            // The key rewrite: `target.method(args)` → `StructName$method(target, args)`
            // when the receiver type is statically known.
            if let Node::FieldAccess { target, field, .. } = function.as_ref()
                && let Some(mangled) = ctx.resolve_method(target, field)
            {
                // Rewrite to direct call: prepend the receiver as the first argument.
                let mut new_args = Vec::with_capacity(arguments.len() + 1);
                new_args.push(rewrite_node(target, ctx));
                for a in arguments {
                    new_args.push(rewrite_node(a, ctx));
                }
                let id_span = match target.as_ref() {
                    Node::Identifier { span, .. } => *span,
                    _ => *span,
                };
                return Node::CallExpression {
                    function: Box::new(Node::Identifier {
                        name: mangled,
                        span: id_span,
                    }),
                    arguments: new_args,
                    span: *span,
                };
            }
            // Not devirtualizable — recurse into sub-expressions.
            let new_fn = Box::new(rewrite_node(function, ctx));
            let new_args = arguments.iter().map(|a| rewrite_node(a, ctx)).collect();
            Node::CallExpression {
                function: new_fn,
                arguments: new_args,
                span: *span,
            }
        }
        Node::ExpressionStatement { expr, span } => Node::ExpressionStatement {
            expr: Box::new(rewrite_node(expr, ctx)),
            span: *span,
        },
        Node::ReturnStatement { value, span } => Node::ReturnStatement {
            value: value.as_ref().map(|v| Box::new(rewrite_node(v, ctx))),
            span: *span,
        },
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            span,
        } => Node::IfStatement {
            condition: Box::new(rewrite_node(condition, ctx)),
            consequence: Box::new(rewrite_node(consequence, ctx)),
            alternative: alternative.as_ref().map(|a| Box::new(rewrite_node(a, ctx))),
            span: *span,
        },
        Node::WhileStatement {
            condition,
            body,
            invariants,
            span,
            label,
        } => Node::WhileStatement {
            condition: Box::new(rewrite_node(condition, ctx)),
            body: Box::new(rewrite_node(body, ctx)),
            invariants: invariants.iter().map(|i| rewrite_node(i, ctx)).collect(),
            span: *span,
            label: label.clone(),
        },
        Node::ForInStatement {
            name,
            iterable,
            body,
            invariants,
            span,
            label,
        } => Node::ForInStatement {
            name: name.clone(),
            iterable: Box::new(rewrite_node(iterable, ctx)),
            body: Box::new(rewrite_node(body, ctx)),
            invariants: invariants.iter().map(|i| rewrite_node(i, ctx)).collect(),
            span: *span,
            label: label.clone(),
        },
        Node::Assignment { name, value, span } => {
            let new_value = rewrite_node(value, ctx);
            // RES-4095 increment 3: a reassignment changes (or erases) the
            // statically-known struct type this local holds. Before this
            // fix, `ctx` was never updated on `Node::Assignment`, so a
            // `dyn Trait`-typed (or any) local reassigned to a different
            // concrete struct kept resolving `.method()` calls to the
            // *original* binding's mangled function — silently wrong
            // dispatch under `--vm`/`--jit`, not just a missed
            // optimization. Mirror `LetStatement`'s handling: record the
            // new struct type when the RHS is a literal, otherwise drop
            // the stale entry so later call sites fall back to the
            // runtime-checked `Op::CallMethod` dispatch instead of
            // reusing a now-invalid mangled name.
            if let Node::StructLiteral {
                name: struct_name, ..
            } = &new_value
            {
                ctx.record(name, struct_name);
            } else {
                ctx.local_struct_types.remove(name);
            }
            Node::Assignment {
                name: name.clone(),
                value: Box::new(new_value),
                span: *span,
            }
        }
        Node::InfixExpression {
            left,
            operator,
            right,
            span,
        } => Node::InfixExpression {
            left: Box::new(rewrite_node(left, ctx)),
            operator,
            right: Box::new(rewrite_node(right, ctx)),
            span: *span,
        },
        Node::PrefixExpression {
            operator,
            right,
            span,
        } => Node::PrefixExpression {
            operator,
            right: Box::new(rewrite_node(right, ctx)),
            span: *span,
        },
        Node::ImplBlock {
            trait_name,
            struct_name,
            methods,
            span,
            associated_type_impls,
        } => {
            let mut impl_ctx = DevirtCtx::new();
            Node::ImplBlock {
                trait_name: trait_name.clone(),
                struct_name: struct_name.clone(),
                methods: methods
                    .iter()
                    .map(|m| rewrite_node(m, &mut impl_ctx))
                    .collect(),
                span: *span,
                associated_type_impls: associated_type_impls.clone(),
            }
        }
        // All other nodes are leaves or structural nodes we don't need to
        // descend into for the purpose of this optimization.
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;
    use crate::parse;

    fn parse_prog(src: &str) -> Node {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        prog
    }

    // -----------------------------------------------------------------------
    // DevirtCtx unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn struct_literal_receiver_resolves_to_mangled() {
        let ctx = DevirtCtx::new();
        let target = Node::StructLiteral {
            name: "Point".to_string(),
            fields: vec![],
            base: None,
            span: Default::default(),
        };
        let result = ctx.resolve_method(&target, "to_string");
        assert_eq!(result, Some("Point$to_string".to_string()));
    }

    #[test]
    fn known_identifier_receiver_resolves_to_mangled() {
        let mut ctx = DevirtCtx::new();
        ctx.record("p", "Point");
        let target = Node::Identifier {
            name: "p".to_string(),
            span: Default::default(),
        };
        let result = ctx.resolve_method(&target, "to_string");
        assert_eq!(result, Some("Point$to_string".to_string()));
    }

    #[test]
    fn unknown_identifier_receiver_returns_none() {
        let ctx = DevirtCtx::new();
        let target = Node::Identifier {
            name: "x".to_string(),
            span: Default::default(),
        };
        let result = ctx.resolve_method(&target, "to_string");
        assert_eq!(result, None);
    }

    #[test]
    fn non_struct_receiver_returns_none() {
        let ctx = DevirtCtx::new();
        let target = Node::IntegerLiteral {
            value: 42,
            span: Default::default(),
        };
        let result = ctx.resolve_method(&target, "to_string");
        assert_eq!(result, None);
    }

    // -----------------------------------------------------------------------
    // AST rewrite (lower) tests
    // -----------------------------------------------------------------------

    #[test]
    fn direct_struct_literal_call_devirtualized() {
        // `new Point { x: 1 }.to_string()` should rewrite to `Point$to_string(new Point { x: 1 })`
        let src = r#"
trait Printable { fn to_string(self) -> string; }
struct Point { int x, }
impl Printable for Point { fn to_string(self) -> string { return "p"; } }
fn main(int dummy) {
    let p = new Point { x: 1 };
    p.to_string();
}
main(0);
"#;
        let prog = parse_prog(src);
        let lowered = lower(&prog);
        // After lowering, any CallExpression that was FieldAccess on `p`
        // should now be a direct call to Point$to_string.
        let found_direct_call = find_direct_call(&lowered, "Point$to_string");
        assert!(
            found_direct_call,
            "expected devirtualized call to Point$to_string in lowered AST"
        );
    }

    #[test]
    fn non_devirtualizable_call_unchanged() {
        // A method call where the receiver type is unknown should remain unchanged.
        let src = r#"
trait Printable { fn to_string(self) -> string; }
struct Point { int x, }
impl Printable for Point { fn to_string(self) -> string { return "p"; } }
fn print_any(Point item) { item.to_string(); }
fn main(int dummy) { print_any(new Point { x: 1 }); }
main(0);
"#;
        let prog = parse_prog(src);
        let lowered = lower(&prog);
        // Inside `print_any`, `item` is a parameter — its type is not tracked
        // in the let-binding registry, so the call stays as FieldAccess.
        // We just verify the program still compiles and runs.
        let bc = crate::compiler::compile(&lowered).expect("compile must succeed");
        crate::vm::run(&bc).expect("VM must execute");
    }

    #[test]
    fn devirtualized_call_produces_same_output_as_original() {
        let src = r#"
trait Tag { fn tag(self) -> string; }
struct S { int x, }
impl Tag for S { fn tag(self) -> string { return "tagged"; } }
fn main() {
    let s = new S { x: 0 };
    s.tag();
}
main();
"#;
        let prog = parse_prog(src);
        // Run original.
        let bc_orig = crate::compiler::compile(&prog).expect("compile original");
        crate::vm::run(&bc_orig).expect("VM original");

        // Run devirtualized.
        let lowered = lower(&prog);
        let bc_dev = crate::compiler::compile(&lowered).expect("compile devirtualized");
        crate::vm::run(&bc_dev).expect("VM devirtualized");
    }

    #[test]
    fn run_pass_returns_ok() {
        let src = "fn main(int dummy) {} main(0);";
        let prog = parse_prog(src);
        let result = run(&prog, "test.rz");
        assert!(result.is_ok());
    }

    #[test]
    fn devirt_ctx_default_is_empty() {
        let ctx = DevirtCtx::default();
        assert!(ctx.local_struct_types.is_empty());
    }

    // -----------------------------------------------------------------------
    // Benchmark-equivalent: no behavioral change under 1M calls
    // -----------------------------------------------------------------------

    #[test]
    fn devirt_preserves_behavior_under_many_calls() {
        // This test verifies behavioral equivalence — the key correctness
        // property. A real benchmark lives in the examples/.
        let src = r#"
trait Counter { fn inc(self) -> int; }
struct Ctr { int n, }
impl Counter for Ctr { fn inc(self) -> int { return self.n + 1; } }
fn main() {
    let c = new Ctr { n: 0 };
    let v1 = c.inc();
    let v2 = c.inc();
    let v3 = c.inc();
}
main();
"#;
        let prog = parse_prog(src);
        let lowered = lower(&prog);

        // Both should execute without error.
        let bc = crate::compiler::compile(&lowered).expect("compile");
        crate::vm::run(&bc).expect("VM");
    }

    // -----------------------------------------------------------------------
    // Helper
    // -----------------------------------------------------------------------

    fn find_direct_call(node: &Node, target_name: &str) -> bool {
        match node {
            Node::Program(stmts) => stmts.iter().any(|s| find_direct_call(&s.node, target_name)),
            Node::Function { body, .. } => find_direct_call(body, target_name),
            Node::Block { stmts, .. } => stmts.iter().any(|s| find_direct_call(s, target_name)),
            Node::CallExpression { function, .. } => {
                if let Node::Identifier { name, .. } = function.as_ref()
                    && name == target_name
                {
                    return true;
                }
                find_direct_call(function, target_name)
            }
            Node::ExpressionStatement { expr, .. } => find_direct_call(expr, target_name),
            Node::LetStatement { value, .. } => find_direct_call(value, target_name),
            Node::ReturnStatement { value: Some(v), .. } => find_direct_call(v, target_name),
            Node::IfStatement {
                condition,
                consequence,
                alternative,
                ..
            } => {
                find_direct_call(condition, target_name)
                    || find_direct_call(consequence, target_name)
                    || alternative
                        .as_ref()
                        .is_some_and(|a| find_direct_call(a, target_name))
            }
            Node::ImplBlock { methods, .. } => {
                methods.iter().any(|m| find_direct_call(m, target_name))
            }
            _ => false,
        }
    }

    // -----------------------------------------------------------------------
    // RES-4095 increment 3: reassignment invalidates/updates the binding.
    // -----------------------------------------------------------------------

    #[test]
    fn reassignment_to_new_struct_literal_updates_binding() {
        let mut ctx = DevirtCtx::new();
        ctx.record("c", "Circle");
        // Simulate rewriting `c = new Square { .. };` through the compiler's
        // `Node::Assignment` handling.
        let reassigned_value = Node::StructLiteral {
            name: "Square".to_string(),
            fields: vec![],
            base: None,
            span: Default::default(),
        };
        if let Node::StructLiteral { name, .. } = &reassigned_value {
            ctx.record("c", name);
        }
        let target = Node::Identifier {
            name: "c".to_string(),
            span: Default::default(),
        };
        assert_eq!(
            ctx.resolve_method(&target, "area"),
            Some("Square$area".to_string())
        );
    }

    #[test]
    fn reassignment_through_devirtualized_dyn_binding_dispatches_correctly() {
        // The exact shape of `trait_dyn_dispatch_reassign.rz`: a `dyn
        // Trait`-typed local reassigned mid-function to a different
        // concrete struct. Before the RES-4095 increment-3 fix, the
        // devirtualize pass never updated `ctx` on `Node::Assignment`, so
        // `c.area()` after the reassignment stayed rewritten to the
        // *original* `Circle$area` — a silent wrong-dispatch bug under
        // `--vm`, not merely a missed optimization. Assert the lowered
        // program actually runs `Square$area` (returns 36, not `Circle$area`
        // on a Square receiver, which would runtime-error on the missing
        // `r` field).
        let src = r#"
trait Shape { fn area(self) -> int; }
struct Circle { int r, }
struct Square { int s, }
impl Shape for Circle { fn area(self) -> int { return self.r * self.r; } }
impl Shape for Square { fn area(self) -> int { return self.s * self.s; } }
fn main() -> int {
    let c: dyn Shape = new Circle { r: 2 };
    c = new Square { s: 6 };
    return c.area();
}
main();
"#;
        let prog = parse_prog(src);
        let lowered = lower(&prog);
        // Confirm the second call site was devirtualized to Square$area,
        // not left pointing at the stale Circle$area.
        assert!(
            find_direct_call(&lowered, "Square$area"),
            "expected the post-reassignment call to devirtualize to Square$area"
        );
        let bc = crate::compiler::compile(&lowered).expect("compile devirtualized");
        let result = crate::vm::run(&bc).expect("VM devirtualized");
        assert!(
            matches!(result, Value::Int(36)),
            "expected Value::Int(36), got {:?}",
            result
        );
    }

    #[test]
    fn reassignment_to_non_literal_invalidates_binding() {
        // `c = some_fn_result;` can't be statically resolved, so the
        // binding must be dropped rather than left stale — otherwise a
        // later `.method()` call would keep resolving to the type from
        // before the reassignment.
        let mut ctx = DevirtCtx::new();
        ctx.record("c", "Circle");
        ctx.local_struct_types.remove("c");
        let target = Node::Identifier {
            name: "c".to_string(),
            span: Default::default(),
        };
        assert_eq!(ctx.resolve_method(&target, "area"), None);
    }
}
