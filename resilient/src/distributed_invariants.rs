//! Feature 23/50 — Distributed / Cross-Actor Invariants.
//!
//! `#[distributed_invariant(actors = "A B C", invariant = "...")]`
//! declares an invariant that spans multiple actors. The verifier
//! must establish that every reachable interleaving of message
//! dispatches preserves the invariant.
//!
//! This module records the invariants in a registry and pairs each
//! actor with the invariants that mention it. The Z3 backend in a
//! follow-up consumes the registry to discharge the obligations.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::sync::RwLock;

#[derive(Debug, Clone)]
pub struct DistributedInvariant {
    pub name: String,
    pub actors: Vec<String>,
    pub clause: String,
}

static INVARIANTS: RwLock<Vec<DistributedInvariant>> = RwLock::new(Vec::new());

pub fn collect() -> Vec<DistributedInvariant> {
    let attrs = crate::feature_attrs::find_kind("distributed_invariant");
    // RES-1764: pre-size to attrs.len() — exactly one push per
    // attribute record.
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let mut inv = DistributedInvariant {
            name: item,
            actors: Vec::new(),
            clause: String::new(),
        };
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "actors" => {
                        inv.actors = v.split_whitespace().map(|s| s.to_string()).collect();
                    }
                    "invariant" => inv.clause = v.to_string(),
                    _ => {}
                }
            }
        }
        out.push(inv);
    }
    out
}

pub fn install(invs: Vec<DistributedInvariant>) {
    if let Ok(mut g) = INVARIANTS.write() {
        *g = invs;
    }
}

pub fn for_actor(actor: &str) -> Vec<DistributedInvariant> {
    INVARIANTS
        .read()
        .ok()
        .map(|g| {
            g.iter()
                .filter(|i| i.actors.contains(&actor.to_string()))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1308: gate `install` on the non-empty case. The historical
    // wiring called `install(invs.clone())` before the early-out,
    // burning a RwLock write per compile and creating the
    // wipe-on-empty test race documented in RES-1302.
    let invs = collect();
    if invs.is_empty() {
        return Ok(());
    }
    // RES-1491: validate before `install` so `invs` moves in instead
    // of cloning. Same shape as RES-1481 (derives) / RES-1485
    // (recursive_types) / RES-1487 (ghost+async). Validation only
    // emits `eprintln!` warnings, so the install runs at the same
    // point on the success path. The non-Program early-return also
    // installs first so the global registry reflects this compile's
    // invs even on the unusual path.
    let Node::Program(stmts) = program else {
        install(invs);
        return Ok(());
    };
    // RES-1526: borrow each actor name as `&str` from the AST
    // into the lookup Vec. The contains check below works on
    // `Vec<&str>` (passing `&a.as_str()` matches the `&T` shape
    // `Vec::contains` requires). Same pattern as RES-1495 / RES-1500
    // etc.
    let actor_names: Vec<&str> = stmts
        .iter()
        .filter_map(|s| match &s.node {
            Node::ActorDecl { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    for inv in &invs {
        for a in &inv.actors {
            if !actor_names.contains(&a.as_str()) {
                eprintln!(
                    "warning: distributed_invariant `{}` references unknown actor `{}`",
                    inv.name, a
                );
            }
        }
    }
    install(invs);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_actor_set() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Ledger",
            crate::feature_attrs::AttrRecord {
                name: "distributed_invariant".into(),
                args: r#"actors = "A B C", invariant = "sum >= 0""#.into(),
                line: 0,
            },
        );
        let invs = collect();
        assert_eq!(
            invs[0].actors,
            vec!["A".to_string(), "B".to_string(), "C".to_string()]
        );
        crate::feature_attrs::reset();
    }
}
