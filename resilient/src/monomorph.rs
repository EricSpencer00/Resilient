//! RES-405 PR 3: post-typecheck monomorphization pass for the bytecode VM.
//!
//! Transforms a `Node::Program` by:
//! 1. Collecting all call sites that invoke generic functions with
//!    inferrable concrete type arguments (literals only — variables
//!    require a full type-inference pass that lives in a future PR).
//! 2. For each unique `(fn_name, Vec<Type>)` instantiation, cloning
//!    the generic function body with type-parameter names substituted
//!    and mangling the clone's name to `fn_name$T1$T2`.
//! 3. Rewriting call sites that target a generic function with a
//!    monomorphizable argument list to call the specialized clone.
//!
//! Generic functions are **kept** in the output alongside their
//! specialized clones so that call sites whose argument types cannot
//! be inferred at the AST level (variables, nested calls) continue
//! to work — the tree walker handles those via erasure, and the VM
//! compiler will compile the generic body as an untyped fallback.
//!
//! The only user of this module today is the VM / JIT driver path
//! in `lib.rs`.  The tree-walker uses `active_subst` on the
//! `Interpreter` instead (see PR 2 of RES-405).
//!
//! ## Mangling convention
//!
//! Specialized clones are named `fn_name$T1` or `fn_name$T1$T2` where
//! each `Ti` is the capitalized primitive type name (`Int`, `Float`,
//! `String`, `Bool`, `Bytes`).  The `$` separator cannot appear in
//! user-written identifiers, so there is no collision risk.

#![allow(dead_code)]

use crate::Node;
use crate::span;
use crate::typechecker::Type;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Monomorphize all generic functions reachable from literal-typed call sites.
///
/// Returns a (possibly new) `Node::Program`.  If the program contains no
/// generic functions the original clone is returned unchanged.
pub fn lower(program: &Node) -> Node {
    let stmts = match program {
        Node::Program(s) => s,
        _ => return program.clone(),
    };

    // Phase 1: map generic function names → their AST nodes.
    let generic_fns = collect_generic_fns(stmts);
    if generic_fns.is_empty() {
        return program.clone();
    }

    // Phase 2: collect unique instantiations from literal call sites.
    let mut instantiations: HashMap<String, Vec<Vec<Type>>> = HashMap::new();
    for spanned in stmts {
        collect_in_node(&spanned.node, &generic_fns, &mut instantiations);
    }

    // Phase 3: assemble the lowered program.
    let mut new_stmts: Vec<span::Spanned<Node>> = Vec::new();

    // Copy every non-generic statement with call sites rewritten.
    // Keep generic functions too (erasure fallback for non-literal call sites).
    for spanned in stmts {
        let rewritten = rewrite_node(&spanned.node, &generic_fns, &instantiations);
        new_stmts.push(span::Spanned::new(rewritten, spanned.span));
    }

    // Append specialized monomorphic clones after the existing declarations.
    for (fn_name, instances) in &instantiations {
        if let Some(fn_node) = generic_fns.get(fn_name) {
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            for type_args in instances {
                let mangled = mangle_name(fn_name, type_args);
                if seen.insert(mangled.clone()) {
                    let specialized = specialize_fn(fn_node, &mangled, type_args);
                    new_stmts.push(span::Spanned::new(specialized, span::Span::default()));
                }
            }
        }
    }

    Node::Program(new_stmts)
}

// ---------------------------------------------------------------------------
// Phase 1: collect generic function declarations
// ---------------------------------------------------------------------------

fn collect_generic_fns(stmts: &[span::Spanned<Node>]) -> HashMap<String, Node> {
    let mut map = HashMap::new();
    for spanned in stmts {
        if let Node::Function {
            name, type_params, ..
        } = &spanned.node
            && !type_params.is_empty()
        {
            map.insert(name.clone(), spanned.node.clone());
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Phase 2: collect instantiations
// ---------------------------------------------------------------------------

fn collect_in_node(
    node: &Node,
    generic_fns: &HashMap<String, Node>,
    out: &mut HashMap<String, Vec<Vec<Type>>>,
) {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                collect_in_node(&s.node, generic_fns, out);
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            collect_in_node(body, generic_fns, out);
            for r in requires {
                collect_in_node(r, generic_fns, out);
            }
            for e in ensures {
                collect_in_node(e, generic_fns, out);
            }
            if let Some(r) = recovers_to {
                collect_in_node(r, generic_fns, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_in_node(s, generic_fns, out);
            }
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            // Recurse into arguments first so nested generic calls are collected.
            for arg in arguments {
                collect_in_node(arg, generic_fns, out);
            }
            collect_in_node(function, generic_fns, out);
            // Try to record this call site as an instantiation.
            if let Node::Identifier { name, .. } = function.as_ref()
                && let Some(type_args) = try_infer_call(name, arguments, generic_fns)
            {
                out.entry(name.clone()).or_default().push(type_args);
            }
        }
        Node::LetStatement { value, .. } => collect_in_node(value, generic_fns, out),
        Node::StaticLet { value, .. } => collect_in_node(value, generic_fns, out),
        Node::Const { value, .. } => collect_in_node(value, generic_fns, out),
        Node::Assignment { value, .. } => collect_in_node(value, generic_fns, out),
        Node::ReturnStatement { value: Some(v), .. } => collect_in_node(v, generic_fns, out),
        Node::ReturnStatement { value: None, .. } => {}
        Node::ExpressionStatement { expr, .. } => collect_in_node(expr, generic_fns, out),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_in_node(condition, generic_fns, out);
            collect_in_node(consequence, generic_fns, out);
            if let Some(alt) = alternative {
                collect_in_node(alt, generic_fns, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_in_node(condition, generic_fns, out);
            collect_in_node(body, generic_fns, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            collect_in_node(iterable, generic_fns, out);
            collect_in_node(body, generic_fns, out);
        }
        Node::InfixExpression { left, right, .. } => {
            collect_in_node(left, generic_fns, out);
            collect_in_node(right, generic_fns, out);
        }
        Node::PrefixExpression { right, .. } => collect_in_node(right, generic_fns, out),
        _ => {}
    }
}

/// Try to infer the concrete type arguments for a call to a generic function.
///
/// Returns `None` if any required type parameter cannot be inferred from
/// the argument expressions (e.g., the argument is a variable).
fn try_infer_call(
    fn_name: &str,
    arguments: &[Node],
    generic_fns: &HashMap<String, Node>,
) -> Option<Vec<Type>> {
    let fn_node = generic_fns.get(fn_name)?;
    let (type_params, parameters) = match fn_node {
        Node::Function {
            type_params,
            parameters,
            ..
        } => (type_params, parameters),
        _ => return None,
    };
    if parameters.len() != arguments.len() {
        return None;
    }
    let tp_set: std::collections::HashSet<&str> = type_params.iter().map(String::as_str).collect();
    let mut mapping: HashMap<&str, Type> = HashMap::new();
    for ((param_ty, _), arg) in parameters.iter().zip(arguments.iter()) {
        if tp_set.contains(param_ty.as_str()) {
            let inferred = infer_literal_type(arg)?;
            match mapping.get(param_ty.as_str()) {
                Some(existing) if existing != &inferred => return None,
                _ => {
                    mapping.insert(param_ty.as_str(), inferred);
                }
            }
        }
    }
    // Build result in type_params order for deterministic mangling.
    type_params
        .iter()
        .map(|tp| mapping.get(tp.as_str()).cloned())
        .collect()
}

/// Infer `Type` from a literal AST node.  Returns `None` for non-literals.
fn infer_literal_type(node: &Node) -> Option<Type> {
    match node {
        Node::IntegerLiteral { .. } => Some(Type::Int),
        Node::FloatLiteral { .. } => Some(Type::Float),
        Node::StringLiteral { .. } => Some(Type::String),
        Node::BooleanLiteral { .. } => Some(Type::Bool),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Phase 3a: rewrite call sites
// ---------------------------------------------------------------------------

/// Deep-clone `node` with call sites to generic functions rewritten to use
/// their specialized mangled counterparts wherever the argument types can be
/// inferred from literals.
fn rewrite_node(
    node: &Node,
    generic_fns: &HashMap<String, Node>,
    instantiations: &HashMap<String, Vec<Vec<Type>>>,
) -> Node {
    match node {
        Node::Program(stmts) => Node::Program(
            stmts
                .iter()
                .map(|s| {
                    span::Spanned::new(rewrite_node(&s.node, generic_fns, instantiations), s.span)
                })
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
        } => Node::Function {
            name: name.clone(),
            parameters: parameters.clone(),
            defaults: defaults.clone(),
            body: Box::new(rewrite_node(body, generic_fns, instantiations)),
            requires: requires
                .iter()
                .map(|r| rewrite_node(r, generic_fns, instantiations))
                .collect(),
            ensures: ensures
                .iter()
                .map(|e| rewrite_node(e, generic_fns, instantiations))
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
                .map(|r| Box::new(rewrite_node(r, generic_fns, instantiations))),
        },
        Node::Block { stmts, span } => Node::Block {
            stmts: stmts
                .iter()
                .map(|s| rewrite_node(s, generic_fns, instantiations))
                .collect(),
            span: *span,
        },
        Node::CallExpression {
            function,
            arguments,
            span,
        } => {
            let rewritten_args: Vec<Node> = arguments
                .iter()
                .map(|a| rewrite_node(a, generic_fns, instantiations))
                .collect();
            // If this is a call to a known generic function with inferrable
            // argument types, redirect to the specialized clone.
            let new_fn = if let Node::Identifier {
                name,
                span: id_span,
            } = function.as_ref()
            {
                if instantiations.contains_key(name.as_str()) {
                    if let Some(type_args) = try_infer_call(name, arguments, generic_fns) {
                        let mangled = mangle_name(name, &type_args);
                        Box::new(Node::Identifier {
                            name: mangled,
                            span: *id_span,
                        })
                    } else {
                        function.clone()
                    }
                } else {
                    function.clone()
                }
            } else {
                Box::new(rewrite_node(function, generic_fns, instantiations))
            };
            Node::CallExpression {
                function: new_fn,
                arguments: rewritten_args,
                span: *span,
            }
        }
        Node::LetStatement {
            name,
            value,
            type_annot,
            span,
        } => Node::LetStatement {
            name: name.clone(),
            value: Box::new(rewrite_node(value, generic_fns, instantiations)),
            type_annot: type_annot.clone(),
            span: *span,
        },
        Node::StaticLet { name, value, span } => Node::StaticLet {
            name: name.clone(),
            value: Box::new(rewrite_node(value, generic_fns, instantiations)),
            span: *span,
        },
        Node::Const {
            name,
            value,
            type_annot,
            span,
        } => Node::Const {
            name: name.clone(),
            value: Box::new(rewrite_node(value, generic_fns, instantiations)),
            type_annot: type_annot.clone(),
            span: *span,
        },
        Node::Assignment { name, value, span } => Node::Assignment {
            name: name.clone(),
            value: Box::new(rewrite_node(value, generic_fns, instantiations)),
            span: *span,
        },
        Node::ReturnStatement { value, span } => Node::ReturnStatement {
            value: value
                .as_ref()
                .map(|v| Box::new(rewrite_node(v, generic_fns, instantiations))),
            span: *span,
        },
        Node::ExpressionStatement { expr, span } => Node::ExpressionStatement {
            expr: Box::new(rewrite_node(expr, generic_fns, instantiations)),
            span: *span,
        },
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            span,
        } => Node::IfStatement {
            condition: Box::new(rewrite_node(condition, generic_fns, instantiations)),
            consequence: Box::new(rewrite_node(consequence, generic_fns, instantiations)),
            alternative: alternative
                .as_ref()
                .map(|a| Box::new(rewrite_node(a, generic_fns, instantiations))),
            span: *span,
        },
        Node::WhileStatement {
            condition,
            body,
            invariants,
            span,
        } => Node::WhileStatement {
            condition: Box::new(rewrite_node(condition, generic_fns, instantiations)),
            body: Box::new(rewrite_node(body, generic_fns, instantiations)),
            invariants: invariants
                .iter()
                .map(|i| rewrite_node(i, generic_fns, instantiations))
                .collect(),
            span: *span,
        },
        Node::ForInStatement {
            name,
            iterable,
            body,
            invariants,
            span,
        } => Node::ForInStatement {
            name: name.clone(),
            iterable: Box::new(rewrite_node(iterable, generic_fns, instantiations)),
            body: Box::new(rewrite_node(body, generic_fns, instantiations)),
            invariants: invariants
                .iter()
                .map(|i| rewrite_node(i, generic_fns, instantiations))
                .collect(),
            span: *span,
        },
        Node::InfixExpression {
            left,
            operator,
            right,
            span,
        } => Node::InfixExpression {
            left: Box::new(rewrite_node(left, generic_fns, instantiations)),
            operator: operator.clone(),
            right: Box::new(rewrite_node(right, generic_fns, instantiations)),
            span: *span,
        },
        Node::PrefixExpression {
            operator,
            right,
            span,
        } => Node::PrefixExpression {
            operator: operator.clone(),
            right: Box::new(rewrite_node(right, generic_fns, instantiations)),
            span: *span,
        },
        // Leaves and unsupported structural nodes: clone as-is.
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Phase 3b: build a specialized function clone
// ---------------------------------------------------------------------------

/// Clone `fn_node` (a generic `Node::Function`) with `type_params` cleared,
/// the name replaced with `mangled`, and the type-parameter names in
/// `parameters` substituted with their concrete equivalents.
fn specialize_fn(fn_node: &Node, mangled: &str, type_args: &[Type]) -> Node {
    match fn_node {
        Node::Function {
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
            type_param_bounds: _,
            fails,
            recovers_to,
            ..
        } => {
            // Substitute type-parameter names in the parameter type strings.
            let specialized_params: Vec<(String, String)> = parameters
                .iter()
                .map(|(ty, name)| {
                    (
                        substitute_type_str(ty, type_params, type_args),
                        name.clone(),
                    )
                })
                .collect();
            // Substitute in the return type annotation (advisory, but good practice).
            let specialized_return = return_type
                .as_ref()
                .map(|rt| substitute_type_str(rt, type_params, type_args));
            Node::Function {
                name: mangled.to_string(),
                parameters: specialized_params,
                defaults: defaults.clone(),
                body: body.clone(),
                requires: requires.clone(),
                ensures: ensures.clone(),
                return_type: specialized_return,
                span: *span,
                pure: *pure,
                effects: *effects,
                // Specialized clone is monomorphic — no more type parameters.
                type_params: vec![],
                type_param_bounds: vec![],
                fails: fails.clone(),
                recovers_to: recovers_to.clone(),
            }
        }
        other => other.clone(),
    }
}

/// Replace occurrences of type-parameter name strings in `s` with their
/// concrete type string equivalents.  E.g. `"T"` → `"int"` when `T=Int`.
fn substitute_type_str(s: &str, type_params: &[String], type_args: &[Type]) -> String {
    for (param, arg) in type_params.iter().zip(type_args.iter()) {
        if s == param.as_str() {
            return format!("{}", arg); // uses Display impl ("int", "string", …)
        }
    }
    s.to_string()
}

// ---------------------------------------------------------------------------
// Mangling helpers
// ---------------------------------------------------------------------------

/// Produce the mangled name for a generic instantiation.
/// `identity` + `[Int]` → `"identity$Int"`.
/// `first` + `[Int, String]` → `"first$Int$String"`.
pub fn mangle_name(fn_name: &str, type_args: &[Type]) -> String {
    let mut s = fn_name.to_string();
    for ty in type_args {
        s.push('$');
        s.push_str(type_mangle_str(ty));
    }
    s
}

/// Capitalized type name used in mangled identifiers.
fn type_mangle_str(ty: &Type) -> &'static str {
    match ty {
        Type::Int => "Int",
        Type::Float => "Float",
        Type::String => "String",
        Type::Bool => "Bool",
        Type::Bytes => "Bytes",
        // For other types we fall back to a stable tag; these won't arise
        // from literal inference today.
        Type::Int8 => "Int8",
        Type::Int16 => "Int16",
        Type::Int32 => "Int32",
        Type::UInt8 => "UInt8",
        Type::UInt16 => "UInt16",
        Type::UInt32 => "UInt32",
        Type::UInt64 => "UInt64",
        _ => "Unknown",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn disasm_lowered(src: &str) -> String {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let lowered = lower(&prog);
        let bc = crate::compiler::compile(&lowered).expect("compile must succeed");
        let mut out = String::new();
        crate::disasm::disassemble(&bc, &mut out).unwrap();
        out
    }

    fn lower_src(src: &str) -> Node {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        lower(&prog)
    }

    /// Count functions with a given name prefix in the lowered program.
    fn count_fns_with_prefix(program: &Node, prefix: &str) -> usize {
        let stmts = match program {
            Node::Program(s) => s,
            _ => return 0,
        };
        stmts
            .iter()
            .filter(|s| matches!(&s.node, Node::Function { name, .. } if name.starts_with(prefix)))
            .count()
    }

    #[test]
    fn non_generic_program_passes_through_unchanged() {
        let src = "fn add(int x, int y) -> int { return x + y; }";
        let prog = lower_src(src);
        assert_eq!(count_fns_with_prefix(&prog, "add"), 1);
        assert_eq!(count_fns_with_prefix(&prog, "add$"), 0);
    }

    #[test]
    fn identity_fn_gets_two_specializations() {
        let src = r#"
fn identity<T>(T x) -> T { return x; }
fn main() {
    identity(42);
    identity("hello");
}
main();
"#;
        let prog = lower_src(src);
        // Original generic + Int clone + String clone.
        assert!(
            count_fns_with_prefix(&prog, "identity") >= 3,
            "expected >=3 identity* fns"
        );
        assert_eq!(count_fns_with_prefix(&prog, "identity$Int"), 1);
        assert_eq!(count_fns_with_prefix(&prog, "identity$String"), 1);
    }

    #[test]
    fn mangle_name_single_param() {
        assert_eq!(mangle_name("identity", &[Type::Int]), "identity$Int");
        assert_eq!(mangle_name("identity", &[Type::String]), "identity$String");
    }

    #[test]
    fn mangle_name_two_params() {
        assert_eq!(
            mangle_name("first", &[Type::Int, Type::String]),
            "first$Int$String"
        );
    }

    #[test]
    fn vm_disasm_shows_specialized_chunks() {
        let src = r#"
fn identity<T>(T x) -> T { return x; }
fn main() {
    identity(42);
    identity("hello");
}
main();
"#;
        let disasm = disasm_lowered(src);
        assert!(
            disasm.contains("identity$Int"),
            "disasm should contain 'identity$Int': {}",
            disasm
        );
        assert!(
            disasm.contains("identity$String"),
            "disasm should contain 'identity$String': {}",
            disasm
        );
    }

    #[test]
    fn monomorphized_program_executes_in_vm() {
        let src = r#"
fn identity<T>(T x) -> T { return x; }
fn main() {
    identity(42);
    identity("hello");
}
main();
"#;
        let (prog, errs) = parse(src);
        assert!(errs.is_empty());
        let lowered = lower(&prog);
        let bc = crate::compiler::compile(&lowered)
            .expect("compile after monomorphization must succeed");
        // VM should run without error.
        crate::vm::run(&bc).expect("VM should execute monomorphized program");
    }

    #[test]
    fn call_sites_rewritten_to_mangled_names() {
        let src = r#"
fn identity<T>(T x) -> T { return x; }
fn main() {
    identity(42);
    identity("hello");
}
main();
"#;
        let disasm = disasm_lowered(src);
        // The main / fn chunks should call identity$Int and identity$String.
        assert!(
            disasm.contains("-> identity$Int"),
            "call site not rewritten: {}",
            disasm
        );
        assert!(
            disasm.contains("-> identity$String"),
            "call site not rewritten: {}",
            disasm
        );
    }
}
