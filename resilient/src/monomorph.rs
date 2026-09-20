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
    // RES-1924: pre-size to `generic_fns.len()` — the upper bound on
    // distinct generic-fn names that can land as `instantiations`
    // keys. `generic_fns.is_empty()` is guarded by the early-return
    // above, so the capacity is always ≥ 1 on the live path. Mirrors
    // RES-1764 / RES-1796 / RES-1800 pre-sizes.
    let mut instantiations: HashMap<String, Vec<Vec<Type>>> =
        HashMap::with_capacity(generic_fns.len());
    for spanned in stmts {
        collect_in_node(&spanned.node, &generic_fns, &mut instantiations);
    }

    // Phase 3: assemble the lowered program.
    // RES-1718: pre-size to `stmts.len()` (every input statement is
    // rewritten and pushed once via the loop below) plus a small
    // margin for the appended specialised clones — saves ~3-4
    // reallocations on programs with generic call sites.
    let mut new_stmts: Vec<span::Spanned<Node>> =
        Vec::with_capacity(stmts.len() + instantiations.len() * 2);

    // Copy every non-generic statement with call sites rewritten.
    // Keep generic functions too (erasure fallback for non-literal call sites).
    for spanned in stmts {
        let rewritten = rewrite_node(&spanned.node, &generic_fns, &instantiations);
        new_stmts.push(span::Spanned::new(rewritten, spanned.span));
    }

    // Append specialized monomorphic clones after the existing declarations.
    for (fn_name, instances) in &instantiations {
        if let Some(fn_node) = generic_fns.get(fn_name.as_str()) {
            // RES-1924: pre-size to `instances.len()` — exact upper
            // bound, one mangled name per type-args tuple. Skips the
            // default 0→4 grow for fns with ≥ 4 distinct instantiations.
            let mut seen: std::collections::HashSet<String> =
                std::collections::HashSet::with_capacity(instances.len());
            for type_args in instances {
                let mangled = mangle_name(fn_name, type_args);
                if seen.insert(mangled.clone()) {
                    let specialized = specialize_fn(fn_node, &mangled, type_args);
                    let specialized = rewrite_node(&specialized, &generic_fns, &instantiations);
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

// RES-1536: borrow each generic-fn declaration into the index instead
// of cloning the full `Node` tree per entry. The map's only consumers
// (`try_infer_call`, `rewrite_node`'s CallExpression arm, and the
// `lower`-time specialization loop) all just read the fn node to
// inspect parameters / type_params or pass it to `specialize_fn`
// (which itself takes `&Node`). Cloning a `Node::Function` per entry
// — a deep tree clone including body, requires, ensures, etc. — was
// pure overhead, paid even when no call site actually instantiated
// the fn. The borrow keeps every reference inside the `stmts` slice.
fn collect_generic_fns(stmts: &[span::Spanned<Node>]) -> HashMap<&str, &Node> {
    // RES-1764: pre-size to stmts.len() — at most one insert per
    // top-level statement (when it's a generic Function). Upper bound.
    let mut map = HashMap::with_capacity(stmts.len());
    for spanned in stmts {
        if let Node::Function {
            name, type_params, ..
        } = &spanned.node
            && !type_params.is_empty()
        {
            map.insert(name.as_str(), &spanned.node);
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Phase 2: collect instantiations
// ---------------------------------------------------------------------------

fn collect_in_node(
    node: &Node,
    generic_fns: &HashMap<&str, &Node>,
    out: &mut HashMap<String, Vec<Vec<Type>>>,
) {
    // Keep instantiation discovery on the shared exhaustive walker. The old
    // hand-written match covered only statement-level expressions, so a
    // literal generic call in a match arm, handler, closure, or container was
    // silently omitted from the specialization set.
    crate::uniqueness_walk::visit(node, &mut |node| {
        let Node::CallExpression {
            function,
            arguments,
            ..
        } = node
        else {
            return;
        };
        if let Node::Identifier { name, .. } = function.as_ref()
            && let Some(type_args) = try_infer_call(name, arguments, generic_fns)
        {
            out.entry(name.clone()).or_default().push(type_args);
        }
    });
}

/// Try to infer the concrete type arguments for a call to a generic function.
///
/// Returns `None` if any required type parameter cannot be inferred from
/// the argument expressions (e.g., the argument is a variable).
fn try_infer_call(
    fn_name: &str,
    arguments: &[Node],
    generic_fns: &HashMap<&str, &Node>,
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
    // RES-1924: pre-size to `type_params.len()` — exact upper bound,
    // one mapping entry per matching type parameter at most.
    let mut mapping: HashMap<&str, Type> = HashMap::with_capacity(type_params.len());
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
        Node::StringInternLiteral { .. } => Some(Type::String),
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
    generic_fns: &HashMap<&str, &Node>,
    instantiations: &HashMap<String, Vec<Vec<Type>>>,
) -> Node {
    let mut rewritten = node.clone();
    rewrite_calls_in_place(&mut rewritten, generic_fns, instantiations);
    rewritten
}

/// Rewrite children in place so the traversal can cover the full AST without
/// duplicating every field of every `Node` variant. This is deliberately kept
/// separate from `uniqueness_walk`: that module owns read-only feature scans,
/// while this pass must replace call-site identifiers.
fn rewrite_calls_in_place(
    node: &mut Node,
    generic_fns: &HashMap<&str, &Node>,
    instantiations: &HashMap<String, Vec<Vec<Type>>>,
) {
    match node {
        Node::Program(stmts) => {
            for stmt in stmts {
                rewrite_calls_in_place(&mut stmt.node, generic_fns, instantiations);
            }
        }
        Node::Extern { decls, .. } => {
            for decl in decls {
                for clause in &mut decl.requires {
                    rewrite_calls_in_place(clause, generic_fns, instantiations);
                }
                for clause in &mut decl.ensures {
                    rewrite_calls_in_place(clause, generic_fns, instantiations);
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
            for default in defaults.iter_mut().flatten() {
                rewrite_calls_in_place(default, generic_fns, instantiations);
            }
            rewrite_calls_in_place(body, generic_fns, instantiations);
            for clause in requires {
                rewrite_calls_in_place(clause, generic_fns, instantiations);
            }
            for clause in ensures {
                rewrite_calls_in_place(clause, generic_fns, instantiations);
            }
            if let Some(clause) = recovers_to {
                rewrite_calls_in_place(clause, generic_fns, instantiations);
            }
        }
        Node::ImplBlock { methods, .. } | Node::BlanketImpl { methods, .. } => {
            for method in methods {
                rewrite_calls_in_place(method, generic_fns, instantiations);
            }
        }
        Node::ModuleDecl { body, .. } => {
            for item in body {
                rewrite_calls_in_place(item, generic_fns, instantiations);
            }
        }
        Node::Actor {
            state_init,
            concurrent_ensures,
            handlers,
            ..
        } => {
            rewrite_calls_in_place(state_init, generic_fns, instantiations);
            for clause in concurrent_ensures {
                rewrite_calls_in_place(clause, generic_fns, instantiations);
            }
            for handler in handlers {
                for clause in &mut handler.ensures {
                    rewrite_calls_in_place(clause, generic_fns, instantiations);
                }
                rewrite_calls_in_place(&mut handler.body, generic_fns, instantiations);
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
                rewrite_calls_in_place(initializer, generic_fns, instantiations);
            }
            for clause in always_clauses {
                rewrite_calls_in_place(clause, generic_fns, instantiations);
            }
            for clause in eventually_clauses {
                rewrite_calls_in_place(&mut clause.post, generic_fns, instantiations);
            }
            for handler in receive_handlers {
                for clause in &mut handler.requires {
                    rewrite_calls_in_place(clause, generic_fns, instantiations);
                }
                for clause in &mut handler.ensures {
                    rewrite_calls_in_place(clause, generic_fns, instantiations);
                }
                rewrite_calls_in_place(&mut handler.body, generic_fns, instantiations);
            }
            for handler in handlers {
                for clause in &mut handler.ensures {
                    rewrite_calls_in_place(clause, generic_fns, instantiations);
                }
                rewrite_calls_in_place(&mut handler.body, generic_fns, instantiations);
            }
        }
        Node::ClusterDecl { invariants, .. } => {
            for invariant in invariants {
                rewrite_calls_in_place(invariant, generic_fns, instantiations);
            }
        }
        Node::LiveBlock {
            body,
            invariants,
            timeout,
            ..
        } => {
            rewrite_calls_in_place(body, generic_fns, instantiations);
            for invariant in invariants {
                rewrite_calls_in_place(invariant, generic_fns, instantiations);
            }
            if let Some(timeout) = timeout {
                rewrite_calls_in_place(timeout, generic_fns, instantiations);
            }
        }
        Node::Block { stmts, .. } => {
            for stmt in stmts {
                rewrite_calls_in_place(stmt, generic_fns, instantiations);
            }
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::Const { value, .. }
        | Node::Assignment { value, .. }
        | Node::LetDestructureStruct { value, .. }
        | Node::LetTupleDestructure { value, .. }
        | Node::NewtypeConstruct { value, .. }
        | Node::NamedArg { value, .. } => {
            rewrite_calls_in_place(value, generic_fns, instantiations);
        }
        Node::ReturnStatement {
            value: Some(value), ..
        } => {
            rewrite_calls_in_place(value, generic_fns, instantiations);
        }
        Node::BreakWith { value, .. } => {
            rewrite_calls_in_place(value, generic_fns, instantiations);
        }
        Node::Assert {
            condition, message, ..
        }
        | Node::Assume {
            condition, message, ..
        } => {
            rewrite_calls_in_place(condition, generic_fns, instantiations);
            if let Some(message) = message {
                rewrite_calls_in_place(message, generic_fns, instantiations);
            }
        }
        Node::StaticAssert { condition, .. }
        | Node::InvariantStatement {
            expr: condition, ..
        } => {
            rewrite_calls_in_place(condition, generic_fns, instantiations);
        }
        Node::DeferStatement { expr, .. } => {
            rewrite_calls_in_place(expr, generic_fns, instantiations);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            rewrite_calls_in_place(condition, generic_fns, instantiations);
            rewrite_calls_in_place(consequence, generic_fns, instantiations);
            if let Some(alternative) = alternative {
                rewrite_calls_in_place(alternative, generic_fns, instantiations);
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
            rewrite_calls_in_place(condition, generic_fns, instantiations);
            rewrite_calls_in_place(body, generic_fns, instantiations);
            for invariant in invariants {
                rewrite_calls_in_place(invariant, generic_fns, instantiations);
            }
        }
        Node::ExpressionStatement { expr, .. } | Node::TryExpression { expr, .. } => {
            rewrite_calls_in_place(expr, generic_fns, instantiations);
        }
        Node::PrefixExpression { right, .. } => {
            rewrite_calls_in_place(right, generic_fns, instantiations);
        }
        Node::InfixExpression { left, right, .. } => {
            rewrite_calls_in_place(left, generic_fns, instantiations);
            rewrite_calls_in_place(right, generic_fns, instantiations);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            rewrite_calls_in_place(function, generic_fns, instantiations);
            for argument in arguments.iter_mut() {
                rewrite_calls_in_place(argument, generic_fns, instantiations);
            }
            let replacement = match function.as_ref() {
                Node::Identifier { name, span } if instantiations.contains_key(name.as_str()) => {
                    try_infer_call(name, arguments, generic_fns).map(|type_args| Node::Identifier {
                        name: mangle_name(name, &type_args),
                        span: *span,
                    })
                }
                _ => None,
            };
            if let Some(replacement) = replacement {
                **function = replacement;
            }
        }
        Node::OptionalChain { object, access, .. } => {
            rewrite_calls_in_place(object, generic_fns, instantiations);
            if let crate::ChainAccess::Method(_, arguments) = access {
                for argument in arguments {
                    rewrite_calls_in_place(argument, generic_fns, instantiations);
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
            rewrite_calls_in_place(body, generic_fns, instantiations);
            for clause in requires {
                rewrite_calls_in_place(clause, generic_fns, instantiations);
            }
            for clause in ensures {
                rewrite_calls_in_place(clause, generic_fns, instantiations);
            }
            if let Some(clause) = recovers_to {
                rewrite_calls_in_place(clause, generic_fns, instantiations);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            rewrite_calls_in_place(scrutinee, generic_fns, instantiations);
            for (_, guard, body) in arms {
                if let Some(guard) = guard {
                    rewrite_calls_in_place(guard, generic_fns, instantiations);
                }
                rewrite_calls_in_place(body, generic_fns, instantiations);
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            if let Some(base) = base {
                rewrite_calls_in_place(base, generic_fns, instantiations);
            }
            for (_, value) in fields {
                rewrite_calls_in_place(value, generic_fns, instantiations);
            }
        }
        Node::FieldAccess { target, .. } => {
            rewrite_calls_in_place(target, generic_fns, instantiations);
        }
        Node::FieldAssignment { target, value, .. } => {
            rewrite_calls_in_place(target, generic_fns, instantiations);
            rewrite_calls_in_place(value, generic_fns, instantiations);
        }
        Node::ArrayLiteral { items, .. }
        | Node::SetLiteral { items, .. }
        | Node::TupleLiteral { items, .. } => {
            for item in items {
                rewrite_calls_in_place(item, generic_fns, instantiations);
            }
        }
        Node::IndexExpression { target, index, .. } => {
            rewrite_calls_in_place(target, generic_fns, instantiations);
            rewrite_calls_in_place(index, generic_fns, instantiations);
        }
        Node::Slice { target, lo, hi, .. } => {
            rewrite_calls_in_place(target, generic_fns, instantiations);
            if let Some(lo) = lo {
                rewrite_calls_in_place(lo, generic_fns, instantiations);
            }
            if let Some(hi) = hi {
                rewrite_calls_in_place(hi, generic_fns, instantiations);
            }
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            rewrite_calls_in_place(target, generic_fns, instantiations);
            rewrite_calls_in_place(index, generic_fns, instantiations);
            rewrite_calls_in_place(value, generic_fns, instantiations);
        }
        Node::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                rewrite_calls_in_place(key, generic_fns, instantiations);
                rewrite_calls_in_place(value, generic_fns, instantiations);
            }
        }
        Node::TryCatch { body, handlers, .. } => {
            for stmt in body {
                rewrite_calls_in_place(stmt, generic_fns, instantiations);
            }
            for (_, handler_body) in handlers {
                for stmt in handler_body {
                    rewrite_calls_in_place(stmt, generic_fns, instantiations);
                }
            }
        }
        Node::Quantifier { range, body, .. } => {
            match range {
                crate::quantifiers::QuantRange::Range { lo, hi } => {
                    rewrite_calls_in_place(lo, generic_fns, instantiations);
                    rewrite_calls_in_place(hi, generic_fns, instantiations);
                }
                crate::quantifiers::QuantRange::Iterable(iterable) => {
                    rewrite_calls_in_place(iterable, generic_fns, instantiations);
                }
            }
            rewrite_calls_in_place(body, generic_fns, instantiations);
        }
        Node::Range { lo, hi, .. } => {
            rewrite_calls_in_place(lo, generic_fns, instantiations);
            rewrite_calls_in_place(hi, generic_fns, instantiations);
        }
        Node::InterpolatedString { parts, .. } => {
            for part in parts {
                if let crate::string_interp::StringPart::Expr(expr) = part {
                    rewrite_calls_in_place(expr, generic_fns, instantiations);
                }
            }
        }
        Node::TupleIndex { tuple, .. } => {
            rewrite_calls_in_place(tuple, generic_fns, instantiations);
        }
        Node::UnsafeBlock { body, .. } | Node::BenchBlock { body, .. } => {
            rewrite_calls_in_place(body, generic_fns, instantiations);
        }
        Node::ReturnStatement { value: None, .. }
        | Node::Break { .. }
        | Node::Continue { .. }
        | Node::BreakLabel { .. }
        | Node::ContinueLabel { .. }
        | Node::Use { .. }
        | Node::DurationLiteral { .. }
        | Node::Identifier { .. }
        | Node::IntegerLiteral { .. }
        | Node::FloatLiteral { .. }
        | Node::StringLiteral { .. }
        | Node::StringInternLiteral { .. }
        | Node::BytesLiteral { .. }
        | Node::CharLiteral { .. }
        | Node::BooleanLiteral { .. }
        | Node::StructDecl { .. }
        | Node::TraitDecl { .. }
        | Node::TypeAlias { .. }
        | Node::RegionDecl { .. }
        | Node::NewtypeDecl { .. }
        | Node::SupervisorDecl { .. }
        | Node::EnumDecl { .. }
        | Node::RegionParam { .. } => {}
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
                is_pub: false,
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

    fn lowered_call_targets(program: &Node) -> Vec<String> {
        let mut targets = Vec::new();
        crate::uniqueness_walk::visit(program, &mut |node| {
            let Node::CallExpression { function, .. } = node else {
                return;
            };
            if let Node::Identifier { name, .. } = function.as_ref() {
                targets.push(name.clone());
            }
        });
        targets
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
    fn match_arm_generic_call_is_specialized_and_rewritten() {
        let src = r#"
fn identity<T>(T x) -> T { return x; }
fn main() -> int {
    return match 1 {
        1 => identity(42),
        _ => 0,
    };
}
main();
"#;
        let lowered = lower_src(src);
        assert_eq!(count_fns_with_prefix(&lowered, "identity$Int"), 1);
        assert!(lowered_call_targets(&lowered).contains(&"identity$Int".to_string()));
    }

    #[test]
    fn try_handler_and_literal_container_calls_are_specialized() {
        let src = r#"
fn identity<T>(T x) -> T { return x; }
fn main() {
    try {
        let values = [identity(42)];
    } catch Timeout {
        let values = [identity("recovered")];
    }
}
main();
"#;
        let lowered = lower_src(src);
        assert_eq!(count_fns_with_prefix(&lowered, "identity$Int"), 1);
        assert_eq!(count_fns_with_prefix(&lowered, "identity$String"), 1);
        let targets = lowered_call_targets(&lowered);
        assert!(targets.contains(&"identity$Int".to_string()));
        assert!(targets.contains(&"identity$String".to_string()));
    }

    #[test]
    fn function_literal_and_defer_calls_are_specialized() {
        let src = r#"
fn identity<T>(T x) -> T { return x; }
fn main() {
    defer identity(7);
    let f = fn() -> int { return identity(9); };
    f();
}
main();
"#;
        let lowered = lower_src(src);
        assert_eq!(count_fns_with_prefix(&lowered, "identity$Int"), 1);
        let targets = lowered_call_targets(&lowered);
        assert!(
            targets
                .iter()
                .filter(|name| *name == "identity$Int")
                .count()
                >= 2
        );
    }

    #[test]
    fn struct_map_index_and_field_calls_are_specialized() {
        let src = r#"
struct Box { int value }
fn identity<T>(T x) -> T { return x; }
fn main() {
    let values = [identity(1)];
    let boxed = new Box { value: identity(2) };
    let entries = {"answer" -> identity(3)};
    let indexed = values[identity(0)];
    let projected = identity(4).value;
}
main();
"#;
        let lowered = lower_src(src);
        assert_eq!(count_fns_with_prefix(&lowered, "identity$Int"), 1);
        let targets = lowered_call_targets(&lowered);
        assert_eq!(
            targets
                .iter()
                .filter(|name| *name == "identity$Int")
                .count(),
            5,
            "every nested executable call should be rewritten: {targets:?}"
        );
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
