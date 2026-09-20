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
    // Keep discovery in lockstep with the repository-wide AST walker. The
    // previous hand-written match only descended through a small subset of
    // expression nodes, so a literal generic call inside an array, tuple,
    // map, match arm, or index expression was never eligible for lowering.
    crate::uniqueness_walk::visit(node, &mut |nested| {
        if let Node::CallExpression {
            function,
            arguments,
            ..
        } = nested
            && let Node::Identifier { name, .. } = function.as_ref()
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
            is_pub,
        } => Node::Function {
            name: name.clone(),
            parameters: parameters.clone(),
            defaults: defaults
                .iter()
                .map(|d| {
                    d.as_ref()
                        .map(|expr| Box::new(rewrite_node(expr, generic_fns, instantiations)))
                })
                .collect(),
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
            is_pub: *is_pub,
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
            is_const,
        } => Node::LetStatement {
            name: name.clone(),
            value: Box::new(rewrite_node(value, generic_fns, instantiations)),
            type_annot: type_annot.clone(),
            span: *span,
            is_const: *is_const,
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
            label,
        } => Node::WhileStatement {
            condition: Box::new(rewrite_node(condition, generic_fns, instantiations)),
            body: Box::new(rewrite_node(body, generic_fns, instantiations)),
            invariants: invariants
                .iter()
                .map(|i| rewrite_node(i, generic_fns, instantiations))
                .collect(),
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
            iterable: Box::new(rewrite_node(iterable, generic_fns, instantiations)),
            body: Box::new(rewrite_node(body, generic_fns, instantiations)),
            invariants: invariants
                .iter()
                .map(|i| rewrite_node(i, generic_fns, instantiations))
                .collect(),
            span: *span,
            label: label.clone(),
        },
        Node::InfixExpression {
            left,
            operator,
            right,
            span,
        } => Node::InfixExpression {
            left: Box::new(rewrite_node(left, generic_fns, instantiations)),
            operator,
            right: Box::new(rewrite_node(right, generic_fns, instantiations)),
            span: *span,
        },
        Node::PrefixExpression {
            operator,
            right,
            span,
        } => Node::PrefixExpression {
            operator,
            right: Box::new(rewrite_node(right, generic_fns, instantiations)),
            span: *span,
        },
        Node::Assert {
            condition,
            message,
            span,
        } => Node::Assert {
            condition: Box::new(rewrite_node(condition, generic_fns, instantiations)),
            message: message
                .as_ref()
                .map(|m| Box::new(rewrite_node(m, generic_fns, instantiations))),
            span: *span,
        },
        Node::Assume {
            condition,
            message,
            span,
        } => Node::Assume {
            condition: Box::new(rewrite_node(condition, generic_fns, instantiations)),
            message: message
                .as_ref()
                .map(|m| Box::new(rewrite_node(m, generic_fns, instantiations))),
            span: *span,
        },
        Node::BreakWith { value, span } => Node::BreakWith {
            value: Box::new(rewrite_node(value, generic_fns, instantiations)),
            span: *span,
        },
        Node::DeferStatement { expr, span } => Node::DeferStatement {
            expr: Box::new(rewrite_node(expr, generic_fns, instantiations)),
            span: *span,
        },
        Node::InvariantStatement { expr, span } => Node::InvariantStatement {
            expr: Box::new(rewrite_node(expr, generic_fns, instantiations)),
            span: *span,
        },
        Node::FunctionLiteral {
            parameters,
            body,
            requires,
            ensures,
            recovers_to,
            return_type,
            span,
            explicit_effect,
        } => Node::FunctionLiteral {
            parameters: parameters.clone(),
            body: Box::new(rewrite_node(body, generic_fns, instantiations)),
            requires: requires
                .iter()
                .map(|r| rewrite_node(r, generic_fns, instantiations))
                .collect(),
            ensures: ensures
                .iter()
                .map(|e| rewrite_node(e, generic_fns, instantiations))
                .collect(),
            recovers_to: recovers_to
                .as_ref()
                .map(|r| Box::new(rewrite_node(r, generic_fns, instantiations))),
            return_type: return_type.clone(),
            span: *span,
            explicit_effect: *explicit_effect,
        },
        Node::Match {
            scrutinee,
            arms,
            span,
        } => Node::Match {
            scrutinee: Box::new(rewrite_node(scrutinee, generic_fns, instantiations)),
            arms: arms
                .iter()
                .map(|(pattern, guard, body)| {
                    (
                        pattern.clone(),
                        guard
                            .as_ref()
                            .map(|g| rewrite_node(g, generic_fns, instantiations)),
                        rewrite_node(body, generic_fns, instantiations),
                    )
                })
                .collect(),
            span: *span,
        },
        Node::FieldAccess {
            target,
            field,
            span,
        } => Node::FieldAccess {
            target: Box::new(rewrite_node(target, generic_fns, instantiations)),
            field: field.clone(),
            span: *span,
        },
        Node::FieldAssignment {
            target,
            field,
            value,
            span,
        } => Node::FieldAssignment {
            target: Box::new(rewrite_node(target, generic_fns, instantiations)),
            field: field.clone(),
            value: Box::new(rewrite_node(value, generic_fns, instantiations)),
            span: *span,
        },
        Node::ArrayLiteral { items, span } => Node::ArrayLiteral {
            items: items
                .iter()
                .map(|item| rewrite_node(item, generic_fns, instantiations))
                .collect(),
            span: *span,
        },
        Node::TupleLiteral { items, span } => Node::TupleLiteral {
            items: items
                .iter()
                .map(|item| rewrite_node(item, generic_fns, instantiations))
                .collect(),
            span: *span,
        },
        Node::MapLiteral { entries, span } => Node::MapLiteral {
            entries: entries
                .iter()
                .map(|(key, value)| {
                    (
                        rewrite_node(key, generic_fns, instantiations),
                        rewrite_node(value, generic_fns, instantiations),
                    )
                })
                .collect(),
            span: *span,
        },
        Node::SetLiteral { items, span } => Node::SetLiteral {
            items: items
                .iter()
                .map(|item| rewrite_node(item, generic_fns, instantiations))
                .collect(),
            span: *span,
        },
        Node::StructLiteral {
            name,
            fields,
            base,
            span,
        } => Node::StructLiteral {
            name: name.clone(),
            fields: fields
                .iter()
                .map(|(field, value)| {
                    (
                        field.clone(),
                        rewrite_node(value, generic_fns, instantiations),
                    )
                })
                .collect(),
            base: base
                .as_ref()
                .map(|b| Box::new(rewrite_node(b, generic_fns, instantiations))),
            span: *span,
        },
        Node::LetDestructureStruct {
            struct_name,
            fields,
            has_rest,
            value,
            span,
        } => Node::LetDestructureStruct {
            struct_name: struct_name.clone(),
            fields: fields.clone(),
            has_rest: *has_rest,
            value: Box::new(rewrite_node(value, generic_fns, instantiations)),
            span: *span,
        },
        Node::LetTupleDestructure { names, value, span } => Node::LetTupleDestructure {
            names: names.clone(),
            value: Box::new(rewrite_node(value, generic_fns, instantiations)),
            span: *span,
        },
        Node::IndexExpression {
            target,
            index,
            span,
        } => Node::IndexExpression {
            target: Box::new(rewrite_node(target, generic_fns, instantiations)),
            index: Box::new(rewrite_node(index, generic_fns, instantiations)),
            span: *span,
        },
        Node::Slice {
            target,
            lo,
            hi,
            inclusive,
            span,
        } => Node::Slice {
            target: Box::new(rewrite_node(target, generic_fns, instantiations)),
            lo: lo
                .as_ref()
                .map(|l| Box::new(rewrite_node(l, generic_fns, instantiations))),
            hi: hi
                .as_ref()
                .map(|h| Box::new(rewrite_node(h, generic_fns, instantiations))),
            inclusive: *inclusive,
            span: *span,
        },
        Node::IndexAssignment {
            target,
            index,
            value,
            span,
        } => Node::IndexAssignment {
            target: Box::new(rewrite_node(target, generic_fns, instantiations)),
            index: Box::new(rewrite_node(index, generic_fns, instantiations)),
            value: Box::new(rewrite_node(value, generic_fns, instantiations)),
            span: *span,
        },
        Node::TryExpression { expr, span } => Node::TryExpression {
            expr: Box::new(rewrite_node(expr, generic_fns, instantiations)),
            span: *span,
        },
        Node::NewtypeConstruct {
            type_name,
            value,
            span,
        } => Node::NewtypeConstruct {
            type_name: type_name.clone(),
            value: Box::new(rewrite_node(value, generic_fns, instantiations)),
            span: *span,
        },
        Node::NamedArg { name, value, span } => Node::NamedArg {
            name: name.clone(),
            value: Box::new(rewrite_node(value, generic_fns, instantiations)),
            span: *span,
        },
        Node::OptionalChain {
            object,
            access,
            span,
        } => Node::OptionalChain {
            object: Box::new(rewrite_node(object, generic_fns, instantiations)),
            access: match access {
                crate::ChainAccess::Field(field) => crate::ChainAccess::Field(field.clone()),
                crate::ChainAccess::Method(name, args) => crate::ChainAccess::Method(
                    name.clone(),
                    args.iter()
                        .map(|arg| rewrite_node(arg, generic_fns, instantiations))
                        .collect(),
                ),
            },
            span: *span,
        },
        Node::Range {
            lo,
            hi,
            inclusive,
            span,
        } => Node::Range {
            lo: Box::new(rewrite_node(lo, generic_fns, instantiations)),
            hi: Box::new(rewrite_node(hi, generic_fns, instantiations)),
            inclusive: *inclusive,
            span: *span,
        },
        Node::TupleIndex { tuple, index, span } => Node::TupleIndex {
            tuple: Box::new(rewrite_node(tuple, generic_fns, instantiations)),
            index: *index,
            span: *span,
        },
        Node::InterpolatedString { parts, span } => Node::InterpolatedString {
            parts: parts
                .iter()
                .map(|part| match part {
                    crate::string_interp::StringPart::Literal(text) => {
                        crate::string_interp::StringPart::Literal(text.clone())
                    }
                    crate::string_interp::StringPart::Expr(expr) => {
                        crate::string_interp::StringPart::Expr(Box::new(rewrite_node(
                            expr,
                            generic_fns,
                            instantiations,
                        )))
                    }
                })
                .collect(),
            span: *span,
        },
        Node::TryCatch {
            span,
            body,
            handlers,
        } => Node::TryCatch {
            span: *span,
            body: body
                .iter()
                .map(|stmt| rewrite_node(stmt, generic_fns, instantiations))
                .collect(),
            handlers: handlers
                .iter()
                .map(|(name, statements)| {
                    (
                        name.clone(),
                        statements
                            .iter()
                            .map(|stmt| rewrite_node(stmt, generic_fns, instantiations))
                            .collect(),
                    )
                })
                .collect(),
        },
        Node::LiveBlock {
            body,
            invariants,
            backoff,
            backoff_kind,
            timeout,
            max_retries,
            span,
        } => Node::LiveBlock {
            body: Box::new(rewrite_node(body, generic_fns, instantiations)),
            invariants: invariants
                .iter()
                .map(|i| rewrite_node(i, generic_fns, instantiations))
                .collect(),
            backoff: backoff.clone(),
            backoff_kind: *backoff_kind,
            timeout: timeout
                .as_ref()
                .map(|t| Box::new(rewrite_node(t, generic_fns, instantiations))),
            max_retries: *max_retries,
            span: *span,
        },
        Node::Quantifier {
            kind,
            var,
            range,
            body,
            span,
        } => Node::Quantifier {
            kind: *kind,
            var: var.clone(),
            range: match range {
                crate::quantifiers::QuantRange::Range { lo, hi } => {
                    crate::quantifiers::QuantRange::Range {
                        lo: Box::new(rewrite_node(lo, generic_fns, instantiations)),
                        hi: Box::new(rewrite_node(hi, generic_fns, instantiations)),
                    }
                }
                crate::quantifiers::QuantRange::Iterable(iterable) => {
                    crate::quantifiers::QuantRange::Iterable(Box::new(rewrite_node(
                        iterable,
                        generic_fns,
                        instantiations,
                    )))
                }
            },
            body: Box::new(rewrite_node(body, generic_fns, instantiations)),
            span: *span,
        },
        Node::StaticAssert {
            condition,
            message,
            span,
        } => Node::StaticAssert {
            condition: Box::new(rewrite_node(condition, generic_fns, instantiations)),
            message: message.clone(),
            span: *span,
        },
        Node::UnsafeBlock { body, span } => Node::UnsafeBlock {
            body: Box::new(rewrite_node(body, generic_fns, instantiations)),
            span: *span,
        },
        Node::BenchBlock { name, body, span } => Node::BenchBlock {
            name: name.clone(),
            body: Box::new(rewrite_node(body, generic_fns, instantiations)),
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

    #[test]
    fn nested_expression_call_sites_are_rewritten() {
        let src = r#"
fn identity<T>(T x) -> T { return x; }
fn main() {
    let values = [identity(42), identity(43)];
    let pair = (identity(1), identity(2));
}
main();
"#;
        let lowered = lower_src(src);
        let mut call_targets = Vec::new();
        crate::uniqueness_walk::visit(&lowered, &mut |node| {
            if let Node::CallExpression { function, .. } = node
                && let Node::Identifier { name, .. } = function.as_ref()
            {
                call_targets.push(name.clone());
            }
        });

        assert_eq!(
            call_targets
                .iter()
                .filter(|name| *name == "identity$Int")
                .count(),
            4
        );
        assert_eq!(
            call_targets
                .iter()
                .filter(|name| *name == "identity")
                .count(),
            0
        );
        assert_eq!(count_fns_with_prefix(&lowered, "identity$Int"), 1);
    }
}
