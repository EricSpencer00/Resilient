//! RES-3836: General-purpose `#[state_machine(...)]` topology enforcement.
//!
//! Hardware state machines are handled by `hw_state_machine`. This module
//! covers general-purpose business logic FSMs on functions and impl blocks:
//!
//! ```rz
//! #[state_machine(states=[Init, Running, Done],
//!                 transitions=[(Init,Running),(Running,Done)])]
//! fn process(state: State) -> State { ... }
//! ```
//!
//! ## What is checked
//!
//! 1. Every state name in `transitions` must appear in `states` — a typo in a
//!    transition is a hard error.
//! 2. The function body is walked for `Identifier` nodes whose name matches a
//!    declared state. Any reachable state that is not in the declared `states`
//!    list is flagged as an undeclared transition target.
//! 3. Duplicate state names in the `states` list are a hard error.
//!
//! ## Attribute grammar (informal)
//!
//! ```
//! #[state_machine(states=[S1, S2, ...], transitions=[(S1,S2), ...])]
//! ```
//!
//! Both keys are required. State names are identifiers. The `transitions` list
//! is a comma-separated sequence of `(From,To)` pairs.

use crate::Node;
use std::collections::{HashMap, HashSet};

/// Parsed representation of one `#[state_machine(...)]` attribute.
#[derive(Debug, Clone)]
pub struct StateMachineSpec {
    pub fn_name: String,
    pub states: Vec<String>,
    /// Valid (from, to) transition pairs.
    pub transitions: Vec<(String, String)>,
    pub line: u32,
}

fn diagnostic(source_path: &str, line: u32, fn_name: &str, message: &str) -> String {
    format!(
        "{source_path}:{line}:0: error[state_machine]: `#[state_machine]` on `{fn_name}`: {message}"
    )
}

/// Parse `states=[A, B, C]` from the raw args string.
///
/// The parser reconstructs attribute args with spaces around `=` and converts
/// `[`/`]` to spaces, so the actual string looks like:
/// `"states =   A , B , C   , transitions = ..."`.
fn parse_states(args: &str) -> Option<Vec<String>> {
    let idx = args.find("states")?;
    let rest = args[idx + "states".len()..].trim_start();
    let rest = rest.strip_prefix('=')?;
    // Collect up until `transitions` (or end) — skip bracket-derived spaces.
    let end = rest.find("transitions").unwrap_or(rest.len());
    let segment = &rest[..end];
    Some(
        segment
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_'))
            .collect(),
    )
}

/// Parse `transitions=[(A,B),(C,D)]` from the raw args string.
///
/// After the parser's space injection, brackets vanish but parens survive
/// (they are reconstructed literally). Looks for `(A,B)` pairs after the
/// `transitions` keyword.
fn parse_transitions(args: &str) -> Option<Vec<(String, String)>> {
    let idx = args.find("transitions")?;
    let rest = &args[idx..];
    let mut pairs = Vec::new();
    let mut remaining = rest;
    while let Some(open) = remaining.find('(') {
        remaining = &remaining[open + 1..];
        let close = remaining.find(')')?;
        let pair_str = &remaining[..close];
        if let Some((a, b)) = pair_str.split_once(',') {
            let a = a.trim().to_string();
            let b = b.trim().to_string();
            if !a.is_empty() && !b.is_empty() {
                pairs.push((a, b));
            }
        }
        remaining = &remaining[close + 1..];
    }
    Some(pairs)
}

/// Collect all `#[state_machine(...)]` attributes from the feature registry.
pub fn collect_specs() -> Vec<StateMachineSpec> {
    let attrs = crate::feature_attrs::find_kind("state_machine");
    let mut specs = Vec::new();
    for (fn_name, rec) in attrs {
        let args = &rec.args;
        let states = parse_states(args).unwrap_or_default();
        let transitions = parse_transitions(args).unwrap_or_default();
        specs.push(StateMachineSpec {
            fn_name,
            states,
            transitions,
            line: rec.line as u32,
        });
    }
    specs
}

/// Walk an expression node and collect all identifier names.
fn collect_identifiers(node: &Node, out: &mut HashSet<String>) {
    match node {
        Node::Identifier { name, .. } => {
            out.insert(name.clone());
        }
        Node::InfixExpression { left, right, .. } => {
            collect_identifiers(left, out);
            collect_identifiers(right, out);
        }
        Node::PrefixExpression { right, .. } => {
            collect_identifiers(right, out);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            collect_identifiers(function, out);
            for a in arguments {
                collect_identifiers(a, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_identifiers(s, out);
            }
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            collect_identifiers(v, out);
        }
        Node::LetStatement { value, .. } => {
            collect_identifiers(value, out);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_identifiers(condition, out);
            collect_identifiers(consequence, out);
            if let Some(e) = alternative {
                collect_identifiers(e, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_identifiers(condition, out);
            collect_identifiers(body, out);
        }
        Node::Assignment { value, .. } => {
            collect_identifiers(value, out);
        }
        Node::FieldAccess { target, .. } => {
            collect_identifiers(target, out);
        }
        Node::IndexExpression { target, index, .. } => {
            collect_identifiers(target, out);
            collect_identifiers(index, out);
        }
        _ => {}
    }
}

/// Build a map from function name → body node for top-level functions.
fn fn_bodies(program: &Node) -> HashMap<String, Node> {
    let mut map = HashMap::new();
    if let Node::Program(stmts) = program {
        for s in stmts {
            if let Node::Function { name, body, .. } = &s.node {
                map.insert(name.clone(), *body.clone());
            }
        }
    }
    map
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let specs = collect_specs();
    if specs.is_empty() {
        return Ok(());
    }

    let bodies = fn_bodies(program);
    let mut errors: Vec<String> = Vec::new();

    for spec in &specs {
        let fn_name = &spec.fn_name;
        let line = spec.line;

        // Build the declared state set for fast lookup.
        let state_set: HashSet<&str> = spec.states.iter().map(|s| s.as_str()).collect();

        // Rule: duplicate state names.
        {
            let mut seen = HashSet::new();
            for s in &spec.states {
                if !seen.insert(s.as_str()) {
                    errors.push(diagnostic(
                        source_path,
                        line,
                        fn_name,
                        &format!("duplicate state `{s}` in `states` list"),
                    ));
                }
            }
        }

        // Rule: every state in transitions must be declared.
        for (from, to) in &spec.transitions {
            if !state_set.contains(from.as_str()) {
                errors.push(diagnostic(
                    source_path,
                    line,
                    fn_name,
                    &format!(
                        "transition source `{from}` is not listed in `states` — \
                         declare it or fix the typo"
                    ),
                ));
            }
            if !state_set.contains(to.as_str()) {
                errors.push(diagnostic(
                    source_path,
                    line,
                    fn_name,
                    &format!(
                        "transition target `{to}` is not listed in `states` — \
                         declare it or fix the typo"
                    ),
                ));
            }
        }

        // Build the set of reachable states from declared transitions.
        let reachable: HashSet<&str> = spec
            .transitions
            .iter()
            .flat_map(|(a, b)| [a.as_str(), b.as_str()])
            .collect();

        // Rule: identifiers in the function body that match a declared state
        // name must appear in the reachable set. If the body mentions a
        // state that exists in `states` but not in any transition, that state
        // is unreachable — flag it as an undeclared transition.
        if let Some(body) = bodies.get(fn_name) {
            let mut used_idents = HashSet::new();
            collect_identifiers(body, &mut used_idents);

            for ident in &used_idents {
                if state_set.contains(ident.as_str()) && !reachable.contains(ident.as_str()) {
                    errors.push(diagnostic(
                        source_path,
                        line,
                        fn_name,
                        &format!(
                            "body references state `{ident}` which has no declared transition — \
                             add `({ident},NextState)` or `(PrevState,{ident})` to `transitions`"
                        ),
                    ));
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_src(src: &str) -> Result<(), String> {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let r = check(&prog, "<test>");
        crate::feature_attrs::reset();
        r
    }

    #[test]
    fn no_state_machine_attrs_is_ok() {
        assert!(check_src("fn f(int x) -> int { return x; }").is_ok());
    }

    #[test]
    fn valid_state_machine_passes() {
        assert!(
            check_src(
                r#"
#[state_machine(states=[Init, Running, Done], transitions=[(Init,Running),(Running,Done)])]
fn process(int s) -> int {
    let x = Init;
    let y = Running;
    let z = Done;
    return s;
}
"#
            )
            .is_ok()
        );
    }

    #[test]
    fn undeclared_transition_source_rejected() {
        let err = check_src(
            r#"
#[state_machine(states=[Init, Running], transitions=[(Ghost,Running)])]
fn process(int s) -> int { return s; }
"#,
        )
        .unwrap_err();
        assert!(err.contains("transition source `Ghost`"), "got: {err}");
        assert!(err.contains("error[state_machine]"), "got: {err}");
    }

    #[test]
    fn undeclared_transition_target_rejected() {
        let err = check_src(
            r#"
#[state_machine(states=[Init, Running], transitions=[(Init,Typo)])]
fn process(int s) -> int { return s; }
"#,
        )
        .unwrap_err();
        assert!(err.contains("transition target `Typo`"), "got: {err}");
    }

    #[test]
    fn duplicate_state_rejected() {
        let err = check_src(
            r#"
#[state_machine(states=[Init, Init, Done], transitions=[(Init,Done)])]
fn process(int s) -> int { return s; }
"#,
        )
        .unwrap_err();
        assert!(err.contains("duplicate state `Init`"), "got: {err}");
    }

    #[test]
    fn body_referencing_isolated_state_rejected() {
        // Done is declared but has no transition — body references it.
        let err = check_src(
            r#"
#[state_machine(states=[Init, Running, Done], transitions=[(Init,Running)])]
fn process(int s) -> int {
    let x = Done;
    return s;
}
"#,
        )
        .unwrap_err();
        assert!(err.contains("references state `Done`"), "got: {err}");
    }
}
