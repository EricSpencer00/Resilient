//! Callable and captured-value flow across reachable helper boundaries.
use super::CallSite;
use crate::Node;
use std::collections::{BTreeMap, BTreeSet};

type Targets = BTreeSet<String>;
type Env = BTreeMap<String, Targets>;
type Literals = BTreeMap<*const Node, String>;

fn visit<'a>(node: &'a Node, f: &mut impl FnMut(&'a Node)) {
    f(node);
    if matches!(node, Node::Function { .. } | Node::FunctionLiteral { .. }) {
        return;
    }
    crate::uniqueness_walk::children(node, &mut |child| visit(child, f));
}

struct CollectionCall<'a> {
    method: &'a str,
    receiver: &'a Node,
    callback: &'a Node,
    initial: Option<&'a Node>,
}

fn collection_call(node: &Node) -> Option<CollectionCall<'_>> {
    let Node::CallExpression {
        function,
        arguments,
        ..
    } = node
    else {
        return None;
    };
    let Node::FieldAccess { target, field, .. } = function.as_ref() else {
        return None;
    };
    let (callback, initial) = match (field.as_str(), arguments.as_slice()) {
        ("map" | "filter" | "flat_map" | "for_each" | "find" | "any" | "all", [callback])
        | ("reduce", [callback]) => (callback, None),
        ("reduce", [initial, callback]) => (callback, Some(initial)),
        _ => return None,
    };
    Some(CollectionCall {
        method: field,
        receiver: target,
        callback,
        initial,
    })
}

fn is_array(node: &Node, arrays: &Targets) -> bool {
    match node {
        Node::ArrayLiteral { .. } => true,
        Node::Identifier { name, .. } => arrays.contains(name),
        Node::Slice { target, .. } => is_array(target, arrays),
        Node::ExpressionStatement { expr, .. } => is_array(expr, arrays),
        Node::Block { stmts, .. } => stmts.last().is_some_and(|node| is_array(node, arrays)),
        Node::IfStatement {
            consequence,
            alternative: Some(other),
            ..
        } => is_array(consequence, arrays) && is_array(other, arrays),
        Node::CallExpression { function, .. } => {
            matches!(function.as_ref(), Node::FieldAccess { target, field, .. }
            if is_array(target, arrays) && matches!(field.as_str(), "map" | "flat_map" | "filter" | "collect" | "reverse" | "sort" | "sort_desc" | "dedup" | "flatten" | "take" | "skip"))
        }
        _ => false,
    }
}

#[derive(Default)]
pub(super) struct ArrayFacts {
    calls: BTreeSet<*const Node>,
    captures: BTreeMap<*const Node, Targets>,
}

pub(super) fn array_facts(body: &Node, inherited: &Targets) -> ArrayFacts {
    fn kill_writes(node: &Node, env: &mut Targets) {
        visit(node, &mut |node| {
            if let Node::Assignment { name, .. } = node {
                env.remove(name);
            }
        });
    }
    fn scan(node: &Node, env: &mut Targets, facts: &mut ArrayFacts) {
        match node {
            Node::FunctionLiteral { .. } => {
                facts.captures.insert(node as *const Node, env.clone());
                return;
            }
            Node::Function { name, .. } => {
                // Named declarations may be hoisted before local initializers.
                facts.captures.insert(node as *const Node, Targets::new());
                env.remove(name);
                return;
            }
            Node::Block { stmts, .. } => {
                let mut block = env.clone();
                for statement in stmts {
                    scan(statement, &mut block, facts);
                }
                kill_writes(node, env);
                return;
            }
            Node::IfStatement { .. } | Node::WhileStatement { .. } => {
                kill_writes(node, env);
                crate::uniqueness_walk::children(node, &mut |child| {
                    scan(child, &mut env.clone(), facts)
                });
                return;
            }
            Node::ForInStatement {
                name,
                iterable,
                body,
                ..
            } => {
                scan(iterable, env, facts);
                kill_writes(body, env);
                let mut inner = env.clone();
                inner.remove(name);
                scan(body, &mut inner, facts);
                return;
            }
            Node::Match { .. } | Node::TryCatch { .. } => {
                kill_writes(node, env);
                crate::uniqueness_walk::children(node, &mut |child| {
                    scan(child, &mut Targets::new(), facts)
                });
                return;
            }
            Node::LetStatement { name, value, .. }
            | Node::StaticLet { name, value, .. }
            | Node::Const { name, value, .. }
            | Node::Assignment { name, value, .. } => {
                scan(value, env, facts);
                if is_array(value, env) {
                    env.insert(name.clone());
                } else {
                    env.remove(name);
                }
                return;
            }
            Node::LetTupleDestructure { names, value, .. } => {
                scan(value, env, facts);
                for name in names {
                    env.remove(name);
                }
                return;
            }
            _ => {}
        }
        crate::uniqueness_walk::children(node, &mut |child| scan(child, env, facts));
        if collection_call(node).is_some_and(|call| is_array(call.receiver, env)) {
            facts.calls.insert(node as *const Node);
        }
    }
    // Shape facts follow lexical order and scope. In particular, a later
    // declaration must not change the shape captured by an earlier closure.
    let mut facts = ArrayFacts::default();
    scan(body, &mut inherited.clone(), &mut facts);
    facts
}

pub(super) fn array_callback(node: &Node, arrays: &ArrayFacts) -> bool {
    arrays.calls.contains(&(node as *const Node))
}

fn value(
    node: &Node,
    env: &Env,
    returns: &Env,
    known: &Targets,
    literals: &Literals,
    arrays: &ArrayFacts,
) -> Targets {
    match node {
        Node::FunctionLiteral { .. } => literals
            .get(&(node as *const Node))
            .cloned()
            .into_iter()
            .collect(),
        Node::Identifier { name, .. } => env.get(name).cloned().unwrap_or_else(|| {
            if known.contains(name) {
                BTreeSet::from([name.clone()])
            } else {
                Targets::new()
            }
        }),
        Node::ArrayLiteral { items, .. } | Node::TupleLiteral { items, .. } => items
            .iter()
            .flat_map(|n| value(n, env, returns, known, literals, arrays))
            .collect(),
        Node::IndexExpression { target, .. }
        | Node::FieldAccess { target, .. }
        | Node::Slice { target, .. } => value(target, env, returns, known, literals, arrays),
        Node::TupleIndex { tuple, .. } => value(tuple, env, returns, known, literals, arrays),
        Node::ExpressionStatement { expr, .. } | Node::NamedArg { value: expr, .. } => {
            value(expr, env, returns, known, literals, arrays)
        }
        Node::ReturnStatement {
            value: Some(expr), ..
        } => value(expr, env, returns, known, literals, arrays),
        Node::Block { stmts, .. } => stmts
            .last()
            .map(|n| value(n, env, returns, known, literals, arrays))
            .unwrap_or_default(),
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            let mut result = value(consequence, env, returns, known, literals, arrays);
            if let Some(other) = alternative {
                result.extend(value(other, env, returns, known, literals, arrays));
            }
            result
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            let ordinary = if array_callback(node, arrays) {
                Targets::new()
            } else {
                value(function, env, returns, known, literals, arrays)
            };
            let mut result: Targets = ordinary
                .iter()
                .flat_map(|name| returns.get(name).into_iter().flatten().cloned())
                .collect();
            if let Some(call) = collection_call(node) {
                let callback_returns: Targets =
                    value(call.callback, env, returns, known, literals, arrays)
                        .iter()
                        .flat_map(|name| returns.get(name).into_iter().flatten().cloned())
                        .collect();
                match call.method {
                    "map" | "flat_map" => result.extend(callback_returns),
                    "filter" | "find" => {
                        result.extend(value(call.receiver, env, returns, known, literals, arrays))
                    }
                    "reduce" => {
                        result.extend(value(
                            call.initial.unwrap_or(call.receiver),
                            env,
                            returns,
                            known,
                            literals,
                            arrays,
                        ));
                        result.extend(callback_returns);
                    }
                    _ => {}
                }
            } else {
                match function.as_ref() {
                    Node::Identifier { name, .. }
                        if matches!(name.as_str(), "unwrap" | "Some" | "Ok" | "Err")
                            && !env.contains_key(name)
                            && !known.contains(name) =>
                    {
                        if let [argument] = arguments.as_slice() {
                            result.extend(value(argument, env, returns, known, literals, arrays));
                        }
                    }
                    Node::FieldAccess { target, field, .. }
                        if (arguments.is_empty()
                            && matches!(
                                field.as_str(),
                                "unwrap"
                                    | "collect"
                                    | "reverse"
                                    | "sort"
                                    | "sort_desc"
                                    | "dedup"
                                    | "flatten"
                                    | "pop"
                            ))
                            || (arguments.len() == 1
                                && matches!(field.as_str(), "take" | "skip")) =>
                    {
                        result.extend(value(target, env, returns, known, literals, arrays));
                    }
                    _ => {}
                }
            }
            result
        }
        Node::StructLiteral { fields, base, .. } => {
            let mut result: Targets = fields
                .iter()
                .flat_map(|(_, n)| value(n, env, returns, known, literals, arrays))
                .collect();
            if let Some(base) = base {
                result.extend(value(base, env, returns, known, literals, arrays));
            }
            result
        }
        _ => Targets::new(),
    }
}

fn bindings(node: &Node) -> Vec<(&str, &Node)> {
    match node {
        Node::LetStatement { name, value, .. }
        | Node::StaticLet { name, value, .. }
        | Node::Const { name, value, .. }
        | Node::Assignment { name, value, .. } => vec![(name, value)],
        Node::LetTupleDestructure { names, value, .. } => names
            .iter()
            .map(|name| (name.as_str(), value.as_ref()))
            .collect(),
        Node::ForInStatement { name, iterable, .. } => vec![(name, iterable)],
        _ => Vec::new(),
    }
}

fn storage(node: &Node) -> Targets {
    match node {
        Node::Identifier { name, .. } => BTreeSet::from([name.clone()]),
        Node::IndexExpression { target, .. }
        | Node::FieldAccess { target, .. }
        | Node::Slice { target, .. } => storage(target),
        Node::TupleIndex { tuple, .. } => storage(tuple),
        _ => Targets::new(),
    }
}

fn storage_aliases(body: &Node) -> Env {
    let mut aliases = Env::new();
    visit(body, &mut |node| {
        for (name, expr) in bindings(node) {
            extend(&mut aliases, name, storage(expr));
        }
    });
    aliases
}

fn storage_targets(node: &Node, aliases: &Env) -> Targets {
    let mut found = Targets::new();
    let mut work: Vec<_> = storage(node).into_iter().collect();
    while let Some(name) = work.pop() {
        if !found.insert(name.clone()) {
            continue;
        }
        if let Some(next) = aliases.get(&name) {
            work.extend(next.iter().cloned());
        }
    }
    found
}

fn extend(env: &mut Env, name: &str, values: Targets) -> bool {
    let entry = env.entry(name.to_string()).or_default();
    let size = entry.len();
    entry.extend(values);
    entry.len() != size
}

#[derive(Default)]
struct GlobalValues {
    globals: Env,
    captures: BTreeMap<String, Env>,
    captured_arrays: BTreeMap<String, Targets>,
}

pub(super) fn calls(program: &Node, root: &str) -> Vec<CallSite> {
    // Initialization supplies values and captures, not mapped-root call obligations
    // or helper parameter summaries from unrelated startup invocations.
    let (_, initialized) = analyze(program, None, GlobalValues::default());
    analyze(program, Some(root), initialized).0
}

fn analyze(
    program: &Node,
    root: Option<&str>,
    initial: GlobalValues,
) -> (Vec<CallSite>, GlobalValues) {
    let Node::Program(statements) = program else {
        return (Vec::new(), initial);
    };
    let mut functions = BTreeMap::new();
    let mut known = Targets::new();
    let mut top_level = BTreeSet::new();
    for statement in statements {
        match &statement.node {
            Node::Function {
                name,
                parameters,
                body,
                ..
            } => {
                functions.insert(name.clone(), (parameters, body.as_ref()));
                known.insert(name.clone());
                top_level.insert(&statement.node as *const Node);
            }
            Node::Extern { decls, .. } => {
                known.extend(decls.iter().map(|d| d.resilient_name.clone()))
            }
            _ => {}
        }
    }
    // AST identity distinguishes equal source spans from different imports.
    // IDs are assigned in traversal order; addresses never escape this analysis.
    let mut literals = Literals::new();
    crate::uniqueness_walk::visit(program, &mut |node| match node {
        Node::FunctionLiteral {
            parameters, body, ..
        }
        | Node::Function {
            parameters, body, ..
        } if !top_level.contains(&(node as *const Node)) => {
            let name = format!("$closure:{}", literals.len());
            literals.insert(node as *const Node, name.clone());
            known.insert(name.clone());
            functions.insert(name, (parameters, body.as_ref()));
        }
        _ => {}
    });
    let initializer_parameters = Vec::new();
    if root.is_none() {
        functions.insert(
            "$initializers".to_string(),
            (&initializer_parameters, program),
        );
    }
    let GlobalValues {
        mut globals,
        mut captures,
        mut captured_arrays,
    } = initial;
    let mut locals = BTreeMap::<String, Env>::new();
    for (name, (parameters, body)) in &functions {
        let mut env = Env::new();
        for (_, name) in *parameters {
            env.insert(name.clone(), Targets::new());
        }
        if name == "$initializers" {
            locals.insert(name.clone(), env);
            continue;
        }
        visit(body, &mut |node| {
            if let Node::Function { name, .. } = node
                && let Some(identity) = literals.get(&(node as *const Node))
            {
                env.entry(name.clone())
                    .or_default()
                    .insert(identity.clone());
            }
            if !matches!(node, Node::Assignment { .. }) {
                for (name, _) in bindings(node) {
                    env.entry(name.to_string()).or_default();
                }
            }
        });
        locals.insert(name.clone(), env);
    }
    let mut capture_shadows = BTreeMap::<String, Targets>::new();
    loop {
        let mut changed = false;
        for (name, (_, body)) in &functions {
            let mut shadowed = capture_shadows.get(name).cloned().unwrap_or_default();
            shadowed.extend(locals[name].keys().cloned());
            visit(body, &mut |node| {
                if let Some(closure) = literals.get(&(node as *const Node)) {
                    let entry = capture_shadows.entry(closure.clone()).or_default();
                    let before = entry.len();
                    entry.extend(shadowed.iter().cloned());
                    changed |= before != entry.len();
                }
            });
        }
        if !changed {
            break;
        }
    }
    let mut returns = Env::new();
    let mut reachable = BTreeSet::from([root.unwrap_or("$initializers").to_string()]);
    let mut sites = BTreeSet::new();
    let mut escaped = Targets::new();
    // All sets grow over a finite universe of named and anonymous functions. Recursive
    // forwarding converges without a depth cutoff that could hide a target.
    loop {
        let mut changed = false;
        // Global captures remain live across initialization and mapped calls.
        // Lexical ancestors' bindings must retain their own captured values.
        for (closure, captured) in &mut captures {
            for (binding, targets) in &globals {
                if !capture_shadows
                    .get(closure)
                    .is_some_and(|shadowed| shadowed.contains(binding))
                {
                    changed |= extend(captured, binding, targets.clone());
                }
            }
        }
        for name in reachable.clone() {
            let Some((parameters, body)) = functions.get(&name) else {
                continue;
            };
            let mut env = globals.clone();
            if let Some(captured) = captures.get(&name) {
                env.extend(captured.clone());
            }
            env.extend(locals[&name].clone());
            let mut inherited_arrays = captured_arrays.get(&name).cloned().unwrap_or_default();
            for (_, parameter) in *parameters {
                inherited_arrays.remove(parameter);
            }
            let arrays = array_facts(body, &inherited_arrays);
            let storage_aliases = storage_aliases(body);
            visit(body, &mut |node| {
                if let Some(closure) = literals.get(&(node as *const Node)) {
                    let facts = captured_arrays.entry(closure.clone()).or_default();
                    let before = facts.len();
                    facts.extend(
                        arrays
                            .captures
                            .get(&(node as *const Node))
                            .into_iter()
                            .flatten()
                            .cloned(),
                    );
                    changed |= before != facts.len();
                    for (binding, targets) in &env {
                        changed |= extend(
                            captures.entry(closure.clone()).or_default(),
                            binding,
                            targets.clone(),
                        );
                    }
                }
                if let Node::IndexAssignment {
                    target,
                    value: expr,
                    ..
                }
                | Node::FieldAssignment {
                    target,
                    value: expr,
                    ..
                } = node
                {
                    let targets = value(expr, &env, &returns, &known, &literals, &arrays);
                    for binding in storage_targets(target, &storage_aliases) {
                        if locals[&name].contains_key(&binding) {
                            changed |= extend(
                                locals.entry(name.clone()).or_default(),
                                &binding,
                                targets.clone(),
                            );
                        } else if captures
                            .get(&name)
                            .is_some_and(|env| env.contains_key(&binding))
                        {
                            changed |= extend(
                                captures.entry(name.clone()).or_default(),
                                &binding,
                                targets.clone(),
                            );
                        } else if globals.contains_key(&binding) {
                            changed |= extend(&mut globals, &binding, targets.clone());
                        }
                    }
                }
                for (binding, expr) in bindings(node) {
                    let targets = value(expr, &env, &returns, &known, &literals, &arrays);
                    if locals[&name].contains_key(binding) {
                        changed |=
                            extend(locals.entry(name.clone()).or_default(), binding, targets);
                    } else if captures
                        .get(&name)
                        .is_some_and(|env| env.contains_key(binding))
                    {
                        changed |=
                            extend(captures.entry(name.clone()).or_default(), binding, targets);
                    } else {
                        changed |= extend(&mut globals, binding, targets);
                    }
                }
                if let Node::ReturnStatement {
                    value: Some(expr), ..
                } = node
                {
                    let result = value(expr, &env, &returns, &known, &literals, &arrays);
                    changed |= extend(&mut returns, &name, result);
                }
                if let Node::CallExpression {
                    function,
                    arguments,
                    span,
                    ..
                } = node
                {
                    if let Some(call) = collection_call(node) {
                        let elements =
                            value(call.receiver, &env, &returns, &known, &literals, &arrays);
                        let callbacks =
                            value(call.callback, &env, &returns, &known, &literals, &arrays);
                        let mut accumulator = value(
                            call.initial.unwrap_or(call.receiver),
                            &env,
                            &returns,
                            &known,
                            &literals,
                            &arrays,
                        );
                        for callback in &callbacks {
                            accumulator
                                .extend(returns.get(callback).into_iter().flatten().cloned());
                        }
                        for callback in callbacks {
                            sites.insert((callback.clone(), span.start.line, span.start.column));
                            changed |= reachable.insert(callback.clone());
                            if let Some((params, _)) = functions.get(&callback) {
                                for (index, (_, parameter)) in params.iter().enumerate() {
                                    let targets = match (call.method, index) {
                                        ("reduce", 0) => accumulator.clone(),
                                        ("reduce", 1) | (_, 0) => elements.clone(),
                                        _ => Targets::new(),
                                    };
                                    changed |= extend(
                                        locals.entry(callback.clone()).or_default(),
                                        parameter,
                                        targets,
                                    );
                                }
                            }
                        }
                    }
                    let modeled_array = array_callback(node, &arrays);
                    let callees = if modeled_array {
                        Targets::new()
                    } else {
                        value(function, &env, &returns, &known, &literals, &arrays)
                    };
                    // Unknown consumers (including builtin collection methods)
                    // may execute callbacks. Preserve conservative inspection
                    // of escaped closures instead of silently dropping bodies.
                    if !modeled_array
                        && (callees.is_empty()
                            || callees.iter().any(|callee| !functions.contains_key(callee)))
                    {
                        for argument in arguments {
                            for target in
                                value(argument, &env, &returns, &known, &literals, &arrays)
                            {
                                if target.starts_with("$closure:") {
                                    changed |= reachable.insert(target.clone());
                                    changed |= escaped.insert(target);
                                }
                            }
                        }
                    }
                    for callee in callees {
                        sites.insert((callee.clone(), span.start.line, span.start.column));
                        changed |= reachable.insert(callee.clone());
                        if let Some((params, _)) = functions.get(&callee) {
                            for (index, argument) in arguments.iter().enumerate() {
                                let parameter = match argument {
                                    Node::NamedArg { name, .. } => {
                                        params.iter().find(|(_, p)| p == name)
                                    }
                                    _ => params.get(index),
                                };
                                if let Some((_, parameter)) = parameter {
                                    changed |= extend(
                                        locals.entry(callee.clone()).or_default(),
                                        parameter,
                                        value(argument, &env, &returns, &known, &literals, &arrays),
                                    );
                                }
                            }
                        }
                    }
                }
            });
            let result = value(body, &env, &returns, &known, &literals, &arrays);
            changed |= extend(&mut returns, &name, result);
            if escaped.contains(&name) {
                for target in returns.get(&name).into_iter().flatten() {
                    if target.starts_with("$closure:") {
                        changed |= reachable.insert(target.clone());
                        changed |= escaped.insert(target.clone());
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let sites = sites
        .into_iter()
        .map(|(callee, line, column)| CallSite {
            callee,
            line,
            column,
        })
        .collect();
    (
        sites,
        GlobalValues {
            globals,
            captures,
            captured_arrays,
        },
    )
}
