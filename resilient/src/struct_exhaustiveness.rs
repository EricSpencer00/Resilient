//! Feature 49/50 — Pattern Exhaustiveness for Structs.
//!
//! When `match` arms destructure a struct (`StructName { field1, field2 }`),
//! the analyzer verifies that every reachable variant of the struct's
//! field domain is covered. Initial coverage: bool fields (must
//! cover both true and false) and integer fields with explicit
//! literal patterns (must include a wildcard arm).
//!
//! A match is considered non-exhaustive when ALL of these hold:
//!   1. Every arm uses a `Pattern::Struct` destructure.
//!   2. No arm is an unguarded "catch-all": a wildcard `_`, an
//!      identifier binding, a struct pattern with `..` (`has_rest`),
//!      or a struct pattern whose every field sub-pattern is a wildcard
//!      or identifier (both always succeed without constraint).

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;

// RES-2224: borrow `function` as `&'a str` from the program AST.
// The walker already had `fn_name: &str`, so the per-warning
// `fn_name.to_string()` was pure overhead. Same shape as
// RES-2204 (coverage_warnings) and RES-2220 (labeled_break::DeepBreakWarning).
#[derive(Debug, Clone)]
pub struct ExhaustivenessWarning<'a> {
    pub function: &'a str,
    /// RES-2022: `&'static str` because the sole push site populates
    /// this from a string literal. The previous `String` shape forced
    /// a `.into()` allocation per push for content that already lived
    /// in `.rodata`. Sibling fix to RES-2020 for `coverage_warnings`.
    pub message: &'static str,
}

/// Returns true if the sub-pattern inside a struct field binding
/// cannot fail (i.e., it always matches any value).
fn is_irrefutable_sub_pattern(p: &crate::Pattern) -> bool {
    matches!(p, crate::Pattern::Wildcard | crate::Pattern::Identifier(_))
}

/// Returns true if `pattern` is an unguarded catch-all arm — one that
/// matches any struct value without constraint.
fn struct_arm_is_unguarded_catch_all(
    pattern: &crate::Pattern,
    guard: &Option<crate::Node>,
) -> bool {
    if guard.is_some() {
        return false;
    }
    match pattern {
        crate::Pattern::Wildcard | crate::Pattern::Identifier(_) => true,
        crate::Pattern::Struct {
            fields, has_rest, ..
        } => *has_rest || fields.iter().all(|(_, fp)| is_irrefutable_sub_pattern(fp)),
        _ => false,
    }
}

/// Build a map of `struct_name → declared (type, field_name) pairs, in
/// declaration order` for all top-level `Node::StructDecl` nodes. Used
/// to attempt bool-domain truth-table exhaustiveness (see
/// [`bool_fields_exhaustively_covered`]) — the only place this module
/// needs to know a struct's actual field types rather than just its
/// per-arm pattern shape.
fn collect_struct_field_types(program: &Node) -> HashMap<&str, &Vec<(String, String)>> {
    let mut map = HashMap::new();
    let Node::Program(stmts) = program else {
        return map;
    };
    for s in stmts {
        if let Node::StructDecl { name, fields, .. } = &s.node {
            map.insert(name.as_str(), fields);
        }
    }
    map
}

fn is_bool_type_name(ty: &str) -> bool {
    matches!(ty, "bool" | "Bool" | "boolean")
}

/// RES-3934: does every combination of the struct's declared bool fields
/// get covered by some *unguarded* arm? This implements the coverage the
/// module doc comment already promised ("bool fields ... must cover both
/// true and false") but that the original implementation never actually
/// computed — it only ever checked for an explicit catch-all arm, which
/// made a fully bool-exhaustive match like
/// `Flag { on: true } => .., Flag { on: false } => ..` report a false
/// non-exhaustiveness warning.
///
/// Returns `false` (meaning: fall back to requiring an explicit
/// catch-all, the pre-existing behavior) whenever:
/// - the struct's fields aren't known, or any declared field isn't bool
///   (int/string/nested-struct literal coverage is a materially larger
///   analysis and stays out of scope — the existing catch-all
///   requirement already handles it correctly by being conservative);
/// - any arm's pattern doesn't reduce to "bool literal or wildcard/
///   identifier per declared field" (e.g. a nested struct sub-pattern).
///
/// Guarded arms are skipped entirely rather than treated as covering
/// anything — a guard may fail at runtime, so it can never be relied on
/// for static coverage.
fn bool_fields_exhaustively_covered(
    arms: &[(crate::Pattern, Option<Node>, Node)],
    struct_decls: &HashMap<&str, &Vec<(String, String)>>,
) -> bool {
    let struct_name = match arms.first() {
        Some((crate::Pattern::Struct { struct_name, .. }, _, _)) => struct_name.as_str(),
        _ => return false,
    };
    let Some(decl_fields) = struct_decls.get(struct_name) else {
        return false;
    };
    if decl_fields.is_empty() || !decl_fields.iter().all(|(ty, _)| is_bool_type_name(ty)) {
        return false;
    }
    let field_names: Vec<&str> = decl_fields
        .iter()
        .map(|(_, fname)| fname.as_str())
        .collect();
    bool_fields_exhaustively_covered_by_names(arms, &field_names)
}

/// RES-4012: the actual bool-domain truth-table row-builder + coverage
/// check, parameterized only by the struct's declared bool field names
/// (in declaration order, already confirmed all-bool by the caller).
/// Shared by both [`bool_fields_exhaustively_covered`] (whole-program
/// `analyze()` pass, which discovers field types from `Node::StructDecl`'s
/// textual type names) and [`bool_fields_exhaustively_covered_typed`]
/// (typechecker.rs's inline `check_node` exhaustiveness gate, which
/// already has `Type::Bool`-checked fields from its own `struct_fields`
/// table) — neither caller can drift from the other on the algorithm
/// itself since both funnel through this one function.
fn bool_fields_exhaustively_covered_by_names(
    arms: &[(crate::Pattern, Option<Node>, Node)],
    field_names: &[&str],
) -> bool {
    if field_names.is_empty() {
        return false;
    }
    let struct_name = match arms.first() {
        Some((crate::Pattern::Struct { struct_name, .. }, _, _)) => struct_name.as_str(),
        _ => return false,
    };

    let mut rows: Vec<Vec<Option<bool>>> = Vec::new();
    for (pattern, guard, _) in arms {
        if guard.is_some() {
            continue;
        }
        let crate::Pattern::Struct {
            struct_name: sn,
            fields,
            has_rest,
        } = pattern
        else {
            return false;
        };
        if sn != struct_name || *has_rest {
            return false;
        }
        let mut row = Vec::with_capacity(field_names.len());
        for fname in field_names {
            let Some((_, sub)) = fields.iter().find(|(n, _)| n == fname) else {
                return false;
            };
            match sub.as_ref() {
                crate::Pattern::Literal(Node::BooleanLiteral { value, .. }) => {
                    row.push(Some(*value))
                }
                p if is_irrefutable_sub_pattern(p) => row.push(None),
                _ => return false,
            }
        }
        rows.push(row);
    }

    covers_bool_domain(&rows, 0, field_names.len())
}

/// RES-4012: `Type`-driven twin of [`bool_fields_exhaustively_covered`],
/// `pub(crate)` so `typechecker.rs`'s inline exhaustiveness gate can reuse
/// the exact same truth-table algorithm (via
/// [`bool_fields_exhaustively_covered_by_names`]) instead of maintaining a
/// second implementation with its own bug profile. Takes the
/// typechecker's own `(field_name, Type)` declaration list — the shape
/// already available from `self.struct_fields` — rather than re-deriving
/// field types from AST text the way `analyze()`'s whole-program pass
/// does.
pub(crate) fn bool_fields_exhaustively_covered_typed(
    arms: &[(crate::Pattern, Option<Node>, Node)],
    decl_fields: &[(String, crate::typechecker::Type)],
) -> bool {
    if decl_fields.is_empty()
        || !decl_fields
            .iter()
            .all(|(_, ty)| matches!(ty, crate::typechecker::Type::Bool))
    {
        return false;
    }
    let field_names: Vec<&str> = decl_fields
        .iter()
        .map(|(fname, _)| fname.as_str())
        .collect();
    bool_fields_exhaustively_covered_by_names(arms, &field_names)
}

/// Recursively splits on each field's boolean domain (`true`/`false`)
/// and verifies some row remains that matches every combination.
/// `None` entries are wildcards — they match either value.
fn covers_bool_domain(rows: &[Vec<Option<bool>>], field_idx: usize, num_fields: usize) -> bool {
    if field_idx == num_fields {
        return !rows.is_empty();
    }
    [true, false].iter().all(|&want| {
        let filtered: Vec<Vec<Option<bool>>> = rows
            .iter()
            .filter(|r| r[field_idx].is_none() || r[field_idx] == Some(want))
            .cloned()
            .collect();
        covers_bool_domain(&filtered, field_idx + 1, num_fields)
    })
}

pub fn analyze<'a>(program: &'a Node) -> Vec<ExhaustivenessWarning<'a>> {
    let mut out = Vec::new();
    let Node::Program(stmts) = program else {
        return out;
    };
    let struct_decls = collect_struct_field_types(program);
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            walk(body, name.as_str(), &struct_decls, &mut out);
        }
    }
    out
}

fn walk<'a>(
    node: &'a Node,
    fn_name: &'a str,
    struct_decls: &HashMap<&str, &Vec<(String, String)>>,
    out: &mut Vec<ExhaustivenessWarning<'a>>,
) {
    match node {
        Node::Match { arms, .. } => {
            let all_struct = arms
                .iter()
                .all(|(p, _, _)| matches!(p, crate::Pattern::Struct { .. }));
            let has_cover = arms
                .iter()
                .any(|(p, g, _)| struct_arm_is_unguarded_catch_all(p, g));
            if all_struct
                && !arms.is_empty()
                && !has_cover
                && !bool_fields_exhaustively_covered(arms, struct_decls)
            {
                out.push(ExhaustivenessWarning {
                    function: fn_name,
                    message: "Non-exhaustive match on struct — add a wildcard arm \
                              (`_`, an identifier, or `StructName { .. }`)",
                });
            }
            for (_, _, body) in arms {
                walk(body, fn_name, struct_decls, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk(s, fn_name, struct_decls, out);
            }
        }
        Node::ExpressionStatement { expr, .. } => walk(expr, fn_name, struct_decls, out),
        Node::LetStatement { value, .. } => walk(value, fn_name, struct_decls, out),
        Node::ReturnStatement { value: Some(v), .. } => walk(v, fn_name, struct_decls, out),
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            walk(consequence, fn_name, struct_decls, out);
            if let Some(a) = alternative {
                walk(a, fn_name, struct_decls, out);
            }
        }
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            walk(body, fn_name, struct_decls, out);
        }
        _ => {}
    }
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let warnings = analyze(program);
    if warnings.is_empty() {
        return Ok(());
    }
    let w = &warnings[0];
    Err(format!(
        "{}: error: in fn `{}`: {}",
        source_path, w.function, w.message
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_program_no_warnings() {
        let p = Node::Program(vec![]);
        assert!(analyze(&p).is_empty());
    }

    #[test]
    fn check_always_returns_ok_without_match() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn function_without_struct_no_warnings() {
        let src = "fn g(int x) -> int { return x * 2; }\n";
        let (prog, _) = crate::parse(src);
        assert!(analyze(&prog).is_empty());
    }

    /// A struct match where ALL arms are specific literal-field patterns
    /// and no arm is a catch-all is non-exhaustive — analysis fires a warning.
    #[test]
    fn analyze_detects_missing_catch_all_arm() {
        let src = r#"
struct Point { int x, int y }
fn locate(Point p) -> int {
    return match p {
        Point { x: 0, y: 0 } => 0,
        Point { x: 1, y: 1 } => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let warnings = analyze(&prog);
        assert!(
            !warnings.is_empty(),
            "expected at least one exhaustiveness warning"
        );
        assert!(
            warnings[0].message.contains("Non-exhaustive"),
            "warning must mention Non-exhaustive: {}",
            warnings[0].message
        );
    }

    /// A struct match with `StructName { .. }` (`has_rest = true`) as its
    /// last arm is exhaustive — no warning should fire.
    #[test]
    fn analyze_ok_when_rest_arm_is_present() {
        let src = r#"
struct Point { int x, int y }
fn locate(Point p) -> int {
    return match p {
        Point { x: 0, y: 0 } => 0,
        Point { .. } => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let warnings = analyze(&prog);
        assert!(
            warnings.is_empty(),
            "expected no warnings for match with `..` catch-all; got: {:?}",
            warnings
        );
    }

    /// A struct match where the last arm binds every field as an identifier
    /// (no constraint) is exhaustive — no warning should fire.
    #[test]
    fn analyze_ok_when_identifier_catch_all_arm_is_present() {
        let src = r#"
struct Point { int x, int y }
fn locate(Point p) -> int {
    return match p {
        Point { x: 0, y: 0 } => 0,
        Point { x, y } => x + y,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let warnings = analyze(&prog);
        assert!(
            warnings.is_empty(),
            "expected no warnings for match with identifier catch-all arm; got: {:?}",
            warnings
        );
    }

    /// `check()` must surface the diagnostic as an error for programs
    /// with non-exhaustive struct matches.
    #[test]
    fn check_errors_on_nonexhaustive_struct_match() {
        let src = r#"
struct Event { int code }
fn handle(Event e) -> int {
    return match e {
        Event { code: 0 } => 0,
        Event { code: 1 } => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let result = check(&prog, "test.rz");
        assert!(
            result.is_err(),
            "expected check to fail for non-exhaustive struct match"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("Non-exhaustive match on struct"),
            "error must contain 'Non-exhaustive match on struct': {msg}"
        );
    }

    /// `check()` returns `Ok` for a match with an identifier-catch-all arm.
    #[test]
    fn check_ok_for_exhaustive_struct_match() {
        let src = r#"
struct Event { int code }
fn handle(Event e) -> int {
    return match e {
        Event { code: 0 } => 0,
        Event { code } => code,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test.rz").is_ok());
    }

    // RES-3934: adversarial corpus — bool-domain truth-table coverage,
    // guard interaction, and struct+enum-payload combinations.

    /// Bug fix: `struct_arm_is_unguarded_catch_all` never implemented
    /// the bool truth-table coverage this module's own doc comment
    /// promises ("bool fields ... must cover both true and false") — it
    /// only ever checked for an explicit catch-all arm. A single-bool-field
    /// struct matched on both `true` and `false` is genuinely exhaustive
    /// and must not warn.
    #[test]
    fn single_bool_field_true_and_false_covers_all_no_warning() {
        let src = r#"
struct Flag { bool on }
fn f(Flag flag) -> int {
    return match flag {
        Flag { on: true } => 1,
        Flag { on: false } => 0,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let warnings = analyze(&prog);
        assert!(
            warnings.is_empty(),
            "true+false covers the whole bool domain — expected no warning; got: {:?}",
            warnings
        );
    }

    #[test]
    fn single_bool_field_only_true_still_warns() {
        let src = r#"
struct Flag { bool on }
fn f(Flag flag) -> int {
    return match flag {
        Flag { on: true } => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let warnings = analyze(&prog);
        assert!(
            !warnings.is_empty(),
            "only `true` is covered — `false` is missing, expected a warning"
        );
    }

    /// Two independent bool fields — all four combinations covered by
    /// four literal arms is exhaustive, even though no single arm is a
    /// catch-all and no arm has a wildcard/identifier sub-pattern.
    #[test]
    fn two_bool_fields_full_cartesian_coverage_no_warning() {
        let src = r#"
struct Toggle { bool a, bool b }
fn f(Toggle t) -> int {
    return match t {
        Toggle { a: true, b: true } => 0,
        Toggle { a: true, b: false } => 1,
        Toggle { a: false, b: true } => 2,
        Toggle { a: false, b: false } => 3,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let warnings = analyze(&prog);
        assert!(
            warnings.is_empty(),
            "all four (a, b) combinations covered — expected no warning; got: {:?}",
            warnings
        );
    }

    #[test]
    fn two_bool_fields_missing_one_combination_still_warns() {
        let src = r#"
struct Toggle { bool a, bool b }
fn f(Toggle t) -> int {
    return match t {
        Toggle { a: true, b: true } => 0,
        Toggle { a: true, b: false } => 1,
        Toggle { a: false, b: true } => 2,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let warnings = analyze(&prog);
        assert!(
            !warnings.is_empty(),
            "(a: false, b: false) is never covered — expected a warning"
        );
    }

    /// A wildcard sub-pattern for one field, covering both bool values
    /// for that field, combined with explicit coverage of the other
    /// field, is exhaustive without every combination being spelled out.
    #[test]
    fn bool_field_wildcard_sub_pattern_covers_that_fields_domain() {
        let src = r#"
struct Toggle { bool a, bool b }
fn f(Toggle t) -> int {
    return match t {
        Toggle { a: true, b } => 0,
        Toggle { a: false, b } => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let warnings = analyze(&prog);
        assert!(
            warnings.is_empty(),
            "`b` wildcarded in both arms, `a` covers true+false — expected no warning; got: {:?}",
            warnings
        );
    }

    /// Guarded arms must never count toward bool coverage — a guard can
    /// fail at runtime, so a match relying solely on a guarded arm for
    /// `on: false` is genuinely still non-exhaustive.
    #[test]
    fn guarded_bool_arm_does_not_count_toward_coverage() {
        let src = r#"
struct Flag { bool on }
fn f(Flag flag, bool extra) -> int {
    return match flag {
        Flag { on: true } => 1,
        Flag { on: false } if extra => 0,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let warnings = analyze(&prog);
        assert!(
            !warnings.is_empty(),
            "the only `false` arm is guarded — must still warn as non-exhaustive; got: {:?}",
            warnings
        );
    }

    /// A struct with a non-bool field falls back to the pre-existing
    /// catch-all requirement — literal int coverage is out of scope for
    /// the truth-table optimization (an unbounded domain can't be proven
    /// complete from a finite set of literals in general).
    #[test]
    fn mixed_bool_and_int_field_falls_back_to_catch_all_requirement() {
        let src = r#"
struct Reading { bool valid, int value }
fn f(Reading r) -> int {
    return match r {
        Reading { valid: true, value: 0 } => 0,
        Reading { valid: false, value: 0 } => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let warnings = analyze(&prog);
        assert!(
            !warnings.is_empty(),
            "int field can't be proven exhaustive from two literals — expected a warning; got: {:?}",
            warnings
        );
    }

    /// The mixed-field case is still satisfiable via the pre-existing
    /// catch-all path (unaffected by the bool truth-table addition).
    #[test]
    fn mixed_bool_and_int_field_with_catch_all_arm_no_warning() {
        let src = r#"
struct Reading { bool valid, int value }
fn f(Reading r) -> int {
    return match r {
        Reading { valid: true, value: 0 } => 0,
        Reading { .. } => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let warnings = analyze(&prog);
        assert!(
            warnings.is_empty(),
            "catch-all arm still suppresses the warning; got: {:?}",
            warnings
        );
    }

    /// Struct-pattern + enum-payload combination: a struct that also
    /// appears as an enum variant's payload type must not confuse this
    /// module's per-struct bool-domain analysis — it only ever looks at
    /// the struct pattern's own fields, never at any enclosing enum
    /// context.
    #[test]
    fn bool_struct_nested_inside_enum_payload_still_analyzed_correctly() {
        let src = r#"
struct Flag { bool on }
enum Wrapped { Has(Flag) }
fn describe(Flag flag) -> int {
    return match flag {
        Flag { on: true } => 1,
        Flag { on: false } => 0,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let warnings = analyze(&prog);
        assert!(
            warnings.is_empty(),
            "unrelated enum wrapping the same struct type must not affect analysis; got: {:?}",
            warnings
        );
    }
}
