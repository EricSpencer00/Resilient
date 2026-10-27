//! Conservative identifier-alias targets for the refinement extern guard.

use super::CallSite;
use crate::Node;
use std::collections::{BTreeMap, BTreeSet};

type Aliases = BTreeMap<String, BTreeSet<String>>;

fn binding(node: &Node) -> Option<(&str, &Node)> {
    match node {
        Node::LetStatement { name, value, .. }
        | Node::StaticLet { name, value, .. }
        | Node::Const { name, value, .. }
        | Node::Assignment { name, value, .. } => Some((name, value)),
        _ => None,
    }
}

fn record(aliases: &mut Aliases, node: &Node) {
    if let Some((name, Node::Identifier { name: target, .. })) = binding(node) {
        aliases
            .entry(name.to_string())
            .or_default()
            .insert(target.clone());
    }
}

pub(super) fn globals(program: &Node) -> Aliases {
    let mut aliases = Aliases::new();
    let mut names = BTreeSet::new();
    if let Node::Program(statements) = program {
        for statement in statements {
            if let Some((name, _)) = binding(&statement.node) {
                names.insert(name.to_string());
                record(&mut aliases, &statement.node);
            }
        }
    }
    // A helper can reassign a global callable before another function calls
    // it. Keep every known target rather than depending on execution order.
    crate::uniqueness_walk::visit(program, &mut |node| {
        if let Node::Assignment { name, .. } = node
            && names.contains(name)
        {
            record(&mut aliases, node);
        }
    });
    aliases
}

fn targets(name: &str, aliases: &Aliases) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut work = vec![name.to_string()];
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

pub(super) fn calls(body: &Node, globals: &Aliases) -> Vec<CallSite> {
    let mut aliases = globals.clone();
    crate::uniqueness_walk::visit(body, &mut |node| record(&mut aliases, node));
    let mut resolved = BTreeMap::new();
    let mut calls = Vec::new();
    crate::uniqueness_walk::visit(body, &mut |node| {
        if let Node::CallExpression { function, span, .. } = node
            && let Node::Identifier { name, .. } = function.as_ref()
        {
            let callees = resolved
                .entry(name.clone())
                .or_insert_with(|| targets(name, &aliases));
            for callee in callees.iter() {
                calls.push(CallSite {
                    callee: callee.clone(),
                    line: span.start.line,
                    column: span.start.column,
                });
            }
        }
    });
    calls
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(source: &str) -> Result<(), String> {
        let _guard = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let (program, errors) = crate::parse(source);
        assert!(errors.is_empty(), "parse errors: {errors:?}");
        let result = crate::tla_refines::check(&program, "<alias-test>");
        crate::feature_attrs::reset();
        result
    }

    fn refined(body: &str, extra: &str, contracts: &str) -> String {
        format!(
            "@refines(spec = \"counter.tla\", action = \"Inc\")\n\
             fn refined(int value) {{ {body} }}\n\
             {extra}\n\
             extern \"counter\" {{ fn foreign(value: Int) -> Int {contracts}; }};"
        )
    }

    #[test]
    fn local_alias_cannot_hide_uncontracted_extern() {
        let error = check(&refined("let call = foreign; return call(value);", "", ""))
            .expect_err("aliased extern needs contracts");
        assert!(error.contains("extern fn `foreign`"), "{error}");
        assert!(error.contains("<alias-test>:2:"), "{error}");
    }

    #[test]
    fn alias_chain_to_helper_preserves_transitive_guard() {
        let source = refined(
            "let first = helper; let second = first; return second(value);",
            "fn helper(int value) { return foreign(value); }",
            "",
        );
        assert!(
            check(&source)
                .expect_err("helper target needs checking")
                .contains("extern fn `foreign`")
        );
    }

    #[test]
    fn reassignments_keep_every_possible_target() {
        let source = refined(
            "let call = safe; if value > 0 { call = foreign; } return call(value);",
            "fn safe(int value) { return value; }",
            "",
        );
        assert!(check(&source).is_err());
    }

    #[test]
    fn global_alias_cannot_hide_uncontracted_extern() {
        let source = refined("return global(value);", "let global = foreign;", "");
        assert!(check(&source).is_err());
    }

    #[test]
    fn helper_reassignment_of_global_alias_is_checked() {
        let source = refined(
            "replace(); return global(value);",
            "let global = safe; fn safe(int value) { return value; } fn replace() { global = foreign; }",
            "",
        );
        assert!(check(&source).is_err());
    }

    #[test]
    fn fully_contracted_alias_remains_modelable() {
        let source = refined(
            "let call = foreign; return call(value);",
            "",
            "requires value >= 0 ensures result >= 0",
        );
        check(&source).expect("fully contracted alias should pass");
    }

    #[test]
    fn unused_reference_does_not_become_a_call() {
        check(&refined("let unused = foreign; return value;", "", ""))
            .expect("taking an unused reference does not call it");
    }

    #[test]
    fn non_refined_alias_path_stays_permissive() {
        check("fn plain(int value) { let call = foreign; return call(value); } extern \"counter\" { fn foreign(value: Int) -> Int; };")
            .expect("non-refined paths retain their behavior");
    }

    #[test]
    fn alias_cycles_terminate_without_losing_reachable_targets() {
        let mut aliases = Aliases::new();
        aliases.insert("a".into(), BTreeSet::from(["b".into()]));
        aliases.insert("b".into(), BTreeSet::from(["a".into(), "foreign".into()]));
        assert_eq!(
            targets("a", &aliases),
            BTreeSet::from(["a".into(), "b".into(), "foreign".into()])
        );
    }
}
