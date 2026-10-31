//! Named callable flow across reachable helper boundaries.
use super::CallSite;
use crate::Node;
use std::collections::{BTreeMap, BTreeSet};

type Targets = BTreeSet<String>;
type Env = BTreeMap<String, Targets>;

fn visit<'a>(node: &'a Node, f: &mut impl FnMut(&'a Node)) {
    if matches!(node, Node::Function { .. } | Node::FunctionLiteral { .. }) {
        return;
    }
    f(node);
    crate::uniqueness_walk::children(node, &mut |child| visit(child, f));
}

fn value(node: &Node, env: &Env, returns: &Env, known: &Targets) -> Targets {
    match node {
        Node::Identifier { name, .. } => env.get(name).cloned().unwrap_or_else(|| {
            if known.contains(name) {
                BTreeSet::from([name.clone()])
            } else {
                Targets::new()
            }
        }),
        Node::ArrayLiteral { items, .. } | Node::TupleLiteral { items, .. } => items
            .iter()
            .flat_map(|n| value(n, env, returns, known))
            .collect(),
        Node::IndexExpression { target, .. }
        | Node::FieldAccess { target, .. }
        | Node::Slice { target, .. } => value(target, env, returns, known),
        Node::TupleIndex { tuple, .. } => value(tuple, env, returns, known),
        Node::ExpressionStatement { expr, .. } | Node::NamedArg { value: expr, .. } => {
            value(expr, env, returns, known)
        }
        Node::ReturnStatement {
            value: Some(expr), ..
        } => value(expr, env, returns, known),
        Node::Block { stmts, .. } => stmts
            .last()
            .map(|n| value(n, env, returns, known))
            .unwrap_or_default(),
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            let mut result = value(consequence, env, returns, known);
            if let Some(other) = alternative {
                result.extend(value(other, env, returns, known));
            }
            result
        }
        Node::CallExpression { function, .. } => value(function, env, returns, known)
            .iter()
            .flat_map(|name| returns.get(name).into_iter().flatten().cloned())
            .collect(),
        Node::StructLiteral { fields, base, .. } => {
            let mut result: Targets = fields
                .iter()
                .flat_map(|(_, n)| value(n, env, returns, known))
                .collect();
            if let Some(base) = base {
                result.extend(value(base, env, returns, known));
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

pub(super) fn calls(program: &Node, root: &str) -> Vec<CallSite> {
    let Node::Program(statements) = program else {
        return Vec::new();
    };
    let mut functions = BTreeMap::new();
    let mut known = Targets::new();
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
            }
            Node::Extern { decls, .. } => {
                known.extend(decls.iter().map(|d| d.resilient_name.clone()))
            }
            _ => {}
        }
    }
    let mut globals = Env::new();
    let mut locals = BTreeMap::<String, Env>::new();
    for (name, (parameters, body)) in &functions {
        let mut env = Env::new();
        for (_, name) in *parameters {
            env.insert(name.clone(), Targets::new());
        }
        visit(body, &mut |node| {
            if !matches!(node, Node::Assignment { .. }) {
                for (name, _) in bindings(node) {
                    env.entry(name.to_string()).or_default();
                }
            }
        });
        locals.insert(name.clone(), env);
    }
    let mut returns = Env::new();
    let mut reachable = BTreeSet::from([root.to_string()]);
    let mut sites = BTreeSet::new();
    // All sets grow over a finite universe of named functions. Recursive
    // forwarding converges without a depth cutoff that could hide a target.
    loop {
        let mut changed = false;
        for statement in statements {
            for (name, expr) in bindings(&statement.node) {
                let targets = value(expr, &globals, &returns, &known);
                changed |= extend(&mut globals, name, targets);
            }
        }
        for name in reachable.clone() {
            let Some((_, body)) = functions.get(&name) else {
                continue;
            };
            let mut env = globals.clone();
            env.extend(locals[&name].clone());
            let storage_aliases = storage_aliases(body);
            visit(body, &mut |node| {
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
                    let targets = value(expr, &env, &returns, &known);
                    for binding in storage_targets(target, &storage_aliases) {
                        if locals[&name].contains_key(&binding) {
                            changed |= extend(
                                locals.entry(name.clone()).or_default(),
                                &binding,
                                targets.clone(),
                            );
                        } else if globals.contains_key(&binding) {
                            changed |= extend(&mut globals, &binding, targets.clone());
                        }
                    }
                }
                for (binding, expr) in bindings(node) {
                    let targets = value(expr, &env, &returns, &known);
                    if locals[&name].contains_key(binding) {
                        changed |=
                            extend(locals.entry(name.clone()).or_default(), binding, targets);
                    } else {
                        changed |= extend(&mut globals, binding, targets);
                    }
                }
                if let Node::ReturnStatement {
                    value: Some(expr), ..
                } = node
                {
                    let result = value(expr, &env, &returns, &known);
                    changed |= extend(&mut returns, &name, result);
                }
                if let Node::CallExpression {
                    function,
                    arguments,
                    span,
                    ..
                } = node
                {
                    for callee in value(function, &env, &returns, &known) {
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
                                        value(argument, &env, &returns, &known),
                                    );
                                }
                            }
                        }
                    }
                }
            });
            let result = value(body, &env, &returns, &known);
            changed |= extend(&mut returns, &name, result);
        }
        if !changed {
            break;
        }
    }
    sites
        .into_iter()
        .map(|(callee, line, column)| CallSite {
            callee,
            line,
            column,
        })
        .collect()
}
