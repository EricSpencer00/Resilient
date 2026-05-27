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
    // Recurse into children via uniqueness_walk. The closure captures
    // `errors` by reference, but uniqueness_walk's visitor signature
    // takes `&mut dyn FnMut(&Node)`, so we collect errors into a
    // separate Vec and extend after the visit.
    let mut nested: Vec<ExhaustivenessError> = Vec::new();
    crate::uniqueness_walk::visit(node, &mut |n| {
        // Skip Function nodes — they will be handled by the outer walk
        // call in analyze() with the correct context name. Processing them
        // here would use the outer function's name instead.
        if matches!(n, Node::Function { .. }) {
            return;
        }
        if let Node::Match { arms, .. } = n {
            if let Some(e) = check_match(arms, enum_map, context) {
                nested.push(e);
            }
        }
    });
    errors.extend(nested);
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

    // RES-2591: payload enum variant exhaustiveness tests.

    #[test]
    fn tuple_payload_exhaustive_no_error() {
        // All three variants covered — even though two carry tuple payloads.
        let src = r#"
enum Expr {
    Lit(int),
    Add(Expr, Expr),
    Neg(Expr),
}
fn eval(Expr e) -> int {
    return match e {
        Expr::Lit(n) => n,
        Expr::Add(a, b) => eval(a) + eval(b),
        Expr::Neg(x) => 0 - eval(x),
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert!(
            errs.is_empty(),
            "all payload variants covered — expected no errors; got: {:?}",
            errs
        );
    }

    #[test]
    fn tuple_payload_missing_variant_detected() {
        // Neg is missing — the checker must detect it even though the
        // present arms have tuple payloads.
        let src = r#"
enum Expr {
    Lit(int),
    Add(Expr, Expr),
    Neg(Expr),
}
fn eval(Expr e) -> int {
    return match e {
        Expr::Lit(n) => n,
        Expr::Add(a, b) => eval(a) + eval(b),
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert_eq!(
            errs.len(),
            1,
            "expected exactly one exhaustiveness error; got: {:?}",
            errs
        );
        assert_eq!(errs[0].enum_name, "Expr");
        assert!(
            errs[0].missing.contains(&"Neg".to_string()),
            "missing variants should include Neg; got: {:?}",
            errs[0].missing
        );
    }

    #[test]
    fn named_field_payload_exhaustive_no_error() {
        // Named-field payloads (`{ r }`) — all variants covered.
        let src = r#"
enum Shape {
    Circle { r: float },
    Square { side: float },
}
fn area(Shape s) -> float {
    return match s {
        Shape::Circle { r } => 3.14 * r * r,
        Shape::Square { side } => side * side,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert!(
            errs.is_empty(),
            "all named-field payload variants covered — expected no errors; got: {:?}",
            errs
        );
    }

    #[test]
    fn named_field_payload_missing_variant_detected() {
        // Rect is missing — the checker must detect it.
        let src = r#"
enum Shape {
    Circle { r: float },
    Square { side: float },
    Rect { w: float, h: float },
}
fn area(Shape s) -> float {
    return match s {
        Shape::Circle { r } => 3.14 * r * r,
        Shape::Square { side } => side * side,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert_eq!(
            errs.len(),
            1,
            "expected exactly one exhaustiveness error; got: {:?}",
            errs
        );
        assert_eq!(errs[0].enum_name, "Shape");
        assert!(
            errs[0].missing.contains(&"Rect".to_string()),
            "missing variants should include Rect; got: {:?}",
            errs[0].missing
        );
    }

    #[test]
    fn wildcard_covers_remaining_payload_variants() {
        // Only one arm is explicit; the wildcard covers the rest.
        let src = r#"
enum Expr {
    Lit(int),
    Add(Expr, Expr),
    Neg(Expr),
}
fn is_lit(Expr e) -> bool {
    return match e {
        Expr::Lit(n) => true,
        _ => false,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert!(
            errs.is_empty(),
            "wildcard arm covers remaining payload variants — expected no errors; got: {:?}",
            errs
        );
    }

    #[test]
    fn mixed_payload_and_payload_less_exhaustive() {
        // Mix of payload-carrying and payload-less variants, all covered.
        let src = r#"
enum Token {
    Number(int),
    Plus,
    Minus,
}
fn kind(Token t) -> int {
    return match t {
        Token::Number(n) => 0,
        Token::Plus => 1,
        Token::Minus => 2,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert!(
            errs.is_empty(),
            "mixed payload / payload-less — all covered; expected no errors; got: {:?}",
            errs
        );
    }

    #[test]
    fn mixed_payload_and_payload_less_missing_detected() {
        // Minus is missing.
        let src = r#"
enum Token {
    Number(int),
    Plus,
    Minus,
}
fn kind(Token t) -> int {
    return match t {
        Token::Number(n) => 0,
        Token::Plus => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert_eq!(
            errs.len(),
            1,
            "expected exactly one exhaustiveness error; got: {:?}",
            errs
        );
        assert!(
            errs[0].missing.contains(&"Minus".to_string()),
            "missing variants should include Minus; got: {:?}",
            errs[0].missing
        );
    }

    #[test]
    fn check_error_message_names_missing_payload_variant() {
        // The `check` entry point must produce an error whose text names
        // the missing payload variant by its unqualified name.
        let src = r#"
enum Expr {
    Lit(int),
    Add(Expr, Expr),
    Neg(Expr),
}
fn eval(Expr e) -> int {
    return match e {
        Expr::Lit(n) => n,
        Expr::Add(a, b) => 0,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let result = check(&prog, "test.rz");
        assert!(result.is_err(), "expected check to fail");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("non-exhaustive match on enum"),
            "error must contain 'non-exhaustive match on enum': {msg}"
        );
        assert!(
            msg.contains("Neg"),
            "error must name missing variant 'Neg': {msg}"
        );
    }
}
