//! RES-400: Enum exhaustiveness checking for `match` expressions.
//!
//! Verifies that when a `match` expression pattern-matches on enum variants,
//! all declared variants of the enum are covered — or a catch-all arm
//! (`_`, an identifier binding) is present.
//!
//! ## Detection strategy
//!
//! Works without full type inference: if ALL match arms (ignoring guard
//! conditions) use `Pattern::EnumVariant` patterns with the **same**
//! `type_name`, AND no arm is an unguarded catch-all, AND the set of
//! matched variants is a strict subset of the declared variant names for
//! that enum, then the match is non-exhaustive.
//!
//! A match is considered exhaustive when ANY of:
//! - At least one arm is a `_` wildcard or unguarded identifier.
//! - At least one arm is a guarded `Pattern::EnumVariant` (guards may
//!   fail at runtime, so we conservatively accept the arm as covering
//!   its variant for exhaustiveness purposes).
//! - All `EnumDecl` variant names appear in the match arm set.
//!
//! ## Scope
//!
//! Walks `Node::Function` bodies via `uniqueness_walk::visit`. Function
//! names are tracked for the error message. Top-level bare match
//! expressions are attributed to `"<top-level>"`.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation)]

use crate::Node;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct ExhaustivenessError {
    pub context: String,
    pub enum_name: String,
    pub missing: Vec<String>,
}

/// Build a map of `enum_name → Vec<variant_name>` from all `Node::EnumDecl`
/// nodes in the program (top-level only — nested EnumDecls inside functions
/// are unusual and handled conservatively by returning no match error).
fn collect_enum_variants<'a>(program: &'a Node) -> HashMap<&'a str, Vec<&'a str>> {
    let mut map: HashMap<&'a str, Vec<&'a str>> = HashMap::new();
    let Node::Program(stmts) = program else {
        return map;
    };
    for s in stmts {
        if let Node::EnumDecl { name, variants, .. } = &s.node {
            map.insert(
                name.as_str(),
                variants.iter().map(|v| v.name.as_str()).collect(),
            );
        }
    }
    map
}

/// Returns true if the arm's pattern is a catch-all (always matches without
/// constraining to a specific variant).
fn is_catch_all(pattern: &crate::Pattern) -> bool {
    matches!(
        pattern,
        crate::Pattern::Wildcard | crate::Pattern::Identifier(_)
    )
}

/// Analyze one `Node::Match` expression. Returns an `ExhaustivenessError`
/// if the match is non-exhaustive over a known enum.
fn check_match(
    arms: &[(crate::Pattern, Option<Node>, Node)],
    enum_map: &HashMap<&str, Vec<&str>>,
    context: &str,
) -> Option<ExhaustivenessError> {
    // If any arm is a catch-all (with no guard), the match is exhaustive.
    for (pat, guard, _) in arms {
        if guard.is_none() && is_catch_all(pat) {
            return None;
        }
    }

    // Collect all EnumVariant type_names referenced in the arms.
    // If arms mix different enums or non-EnumVariant patterns, skip.
    let mut enum_name_seen: Option<&str> = None;
    let mut matched_variants: HashSet<&str> = HashSet::new();

    for (pat, _guard, _) in arms {
        match pat {
            crate::Pattern::EnumVariant {
                type_name: Some(tn),
                variant_name,
                ..
            } => {
                if let Some(existing) = enum_name_seen {
                    if existing != tn.as_str() {
                        // Multiple different enum type names — bail conservatively.
                        return None;
                    }
                } else {
                    enum_name_seen = Some(tn.as_str());
                }
                matched_variants.insert(variant_name.as_str());
            }
            crate::Pattern::EnumVariant {
                type_name: None, ..
            } => {
                // Bare variant name without enum prefix — can't attribute to an enum.
                return None;
            }
            _ => {
                // Non-EnumVariant pattern mixed in — bail conservatively.
                return None;
            }
        }
    }

    let enum_name = enum_name_seen?;
    let declared = enum_map.get(enum_name)?;

    let missing: Vec<String> = declared
        .iter()
        .filter(|v| !matched_variants.contains(*v))
        .map(|v| v.to_string())
        .collect();

    if missing.is_empty() {
        return None;
    }

    Some(ExhaustivenessError {
        context: context.to_string(),
        enum_name: enum_name.to_string(),
        missing,
    })
}

/// Walk a node tree collecting exhaustiveness errors. `context` is the
/// enclosing function name (or `"<top-level>"`).
fn walk(
    node: &Node,
    enum_map: &HashMap<&str, Vec<&str>>,
    context: &str,
    errors: &mut Vec<ExhaustivenessError>,
) {
    // Handle top-level function to track context name.
    if let Node::Function { name, body, .. } = node {
        walk(body, enum_map, name.as_str(), errors);
        return;
    }
    // Direct match at this level.
    if let Node::Match { arms, .. } = node {
        if let Some(e) = check_match(arms, enum_map, context) {
            errors.push(e);
        }
    }
    // Recurse into children via uniqueness_walk. The closure pushes
    // directly into `errors` — `visit`'s actual signature is
    // `fn visit<'a>(&'a Node, &mut impl FnMut(&'a Node))` (generic,
    // not a trait object), so capturing `&mut errors` is fine.
    // RES-2358: replaced a stale `let mut nested = Vec::new(); …
    // errors.extend(nested);` workaround that dropped one Vec
    // allocation + extend memcpy per walk call.
    crate::uniqueness_walk::visit(node, &mut |n| {
        // Skip Function nodes — they will be handled by the outer walk
        // call in analyze() with the correct context name. Processing them
        // here would use the outer function's name instead.
        if matches!(n, Node::Function { .. }) {
            return;
        }
        if let Node::Match { arms, .. } = n {
            if let Some(e) = check_match(arms, enum_map, context) {
                errors.push(e);
            }
        }
    });
}

/// Analyze the program for non-exhaustive enum matches. Returns one
/// `ExhaustivenessError` per non-exhaustive `match` found.
pub fn analyze(program: &Node) -> Vec<ExhaustivenessError> {
    let enum_map = collect_enum_variants(program);
    if enum_map.is_empty() {
        return Vec::new();
    }
    let mut errors = Vec::new();
    let Node::Program(stmts) = program else {
        return errors;
    };
    for s in stmts {
        walk(&s.node, &enum_map, "<top-level>", &mut errors);
    }
    errors
}

/// Type-checker entry point. Returns `Err` on the first non-exhaustive
/// enum match found, formatted as `"source_path: error: ..."`.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let errs = analyze(program);
    if errs.is_empty() {
        return Ok(());
    }
    let e = &errs[0];
    Err(format!(
        "{}: error: non-exhaustive match on enum `{}` in `{}`: missing variant(s): {}",
        source_path,
        e.enum_name,
        e.context,
        e.missing.join(", ")
    ))
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_program_no_errors() {
        let p = Node::Program(vec![]);
        assert!(analyze(&p).is_empty());
    }

    #[test]
    fn program_with_no_enum_no_errors() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(analyze(&prog).is_empty());
    }

    #[test]
    fn exhaustive_match_no_error() {
        let src = r#"
enum Color { Red, Green, Blue }
fn describe(Color c) -> string {
    return match c {
        Color::Red => "red",
        Color::Green => "green",
        Color::Blue => "blue",
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert!(
            errs.is_empty(),
            "expected no errors for exhaustive match; got: {:?}",
            errs
        );
    }

    #[test]
    fn wildcard_arm_makes_exhaustive() {
        let src = r#"
enum Color { Red, Green, Blue }
fn describe(Color c) -> string {
    return match c {
        Color::Red => "red",
        _ => "other",
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert!(
            errs.is_empty(),
            "wildcard arm should suppress exhaustiveness check; got: {:?}",
            errs
        );
    }

    #[test]
    fn identifier_catch_all_is_exhaustive() {
        let src = r#"
enum Direction { North, South, East, West }
fn go(Direction d) -> int {
    return match d {
        Direction::North => 0,
        x => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert!(
            errs.is_empty(),
            "identifier catch-all should suppress exhaustiveness check; got: {:?}",
            errs
        );
    }

    #[test]
    fn non_exhaustive_match_reports_missing() {
        let src = r#"
enum Color { Red, Green, Blue }
fn describe(Color c) -> string {
    return match c {
        Color::Red => "red",
        Color::Green => "green",
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert_eq!(errs.len(), 1, "expected exactly one exhaustiveness error");
        assert_eq!(errs[0].enum_name, "Color");
        assert!(
            errs[0].missing.contains(&"Blue".to_string()),
            "missing variants should include Blue; got: {:?}",
            errs[0].missing
        );
    }

    #[test]
    fn check_errors_on_non_exhaustive() {
        let src = r#"
enum Status { Ok, Err, Pending }
fn handle(Status s) -> int {
    return match s {
        Status::Ok => 0,
        Status::Err => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let result = check(&prog, "test.rz");
        assert!(
            result.is_err(),
            "expected check to fail for non-exhaustive enum match"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("non-exhaustive match on enum"),
            "error must contain 'non-exhaustive match on enum': {msg}"
        );
        assert!(
            msg.contains("Pending"),
            "error must name missing variant 'Pending': {msg}"
        );
    }

    #[test]
    fn check_ok_for_exhaustive_match() {
        let src = r#"
enum Status { Ok, Err }
fn handle(Status s) -> int {
    return match s {
        Status::Ok => 0,
        Status::Err => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test.rz").is_ok());
    }
}
