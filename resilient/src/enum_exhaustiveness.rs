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
    /// RES-3934: variant names referenced by match arms that don't
    /// exist on the declared enum at all — almost always a typo (e.g.
    /// `Color::Reed` when the declared variant is `Red`). Surfaced as
    /// a "did you mean" hint in `check()`'s formatted message so the
    /// missing-variant list doesn't read as a false accusation when the
    /// user actually did write an arm for it, just misspelled.
    pub unrecognized: Vec<String>,
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
/// constraining to a specific variant). RES-3934: recurses into `Or`
/// branches — `_ | Color::Red => ...` matches unconditionally (first-match
/// wins, so if ANY branch is a catch-all the whole arm is), so it must be
/// treated the same as a bare `_` arm.
fn is_catch_all(pattern: &crate::Pattern) -> bool {
    match pattern {
        crate::Pattern::Wildcard | crate::Pattern::Identifier(_) => true,
        crate::Pattern::Or(branches) => branches.iter().any(is_catch_all),
        _ => false,
    }
}

/// RES-3934: recursively flatten a pattern into the `(enum_name,
/// variant_name)` pairs it covers. Handles the common case (a single
/// qualified `EnumVariant`) and `Or` patterns (`Color::Red | Color::Green
/// => ...`), which union the coverage of their branches.
///
/// Returns `false` if any leaf of the pattern isn't a qualified
/// `EnumVariant` — a bare variant name (`type_name: None`), a mix with a
/// non-enum pattern, or any other pattern shape. The caller bails
/// conservatively (no exhaustiveness claim) in that case, exactly as it
/// did before `Or` patterns were unpacked.
///
/// RES-4012: `pub(crate)` so `typechecker.rs`'s inline `Node::Match`
/// exhaustiveness gate (in `check_node`) can reuse this exact
/// Or-flattening logic for its own enum-covered-set computation instead
/// of maintaining a second, divergent implementation — that duplication
/// is what let the or-pattern false-negative fixed here (RES-3934) keep
/// rejecting exhaustive matches end-to-end even after this module was
/// fixed.
pub(crate) fn collect_variant_leaves<'p>(
    pattern: &'p crate::Pattern,
    out: &mut Vec<(&'p str, &'p str)>,
) -> bool {
    match pattern {
        crate::Pattern::EnumVariant {
            type_name: Some(tn),
            variant_name,
            ..
        } => {
            out.push((tn.as_str(), variant_name.as_str()));
            true
        }
        crate::Pattern::Or(branches) => branches.iter().all(|b| collect_variant_leaves(b, out)),
        _ => false,
    }
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

    // Collect all EnumVariant type_names referenced in the arms — RES-3934:
    // `Or` patterns are flattened to the union of their branches first, so
    // `Color::Red | Color::Green => ...` contributes both variant names
    // instead of causing the whole match to be skipped. If arms mix
    // different enums or non-EnumVariant patterns, skip.
    let mut enum_name_seen: Option<&str> = None;
    let mut matched_variants: HashSet<&str> = HashSet::new();

    for (pat, _guard, _) in arms {
        let mut leaves: Vec<(&str, &str)> = Vec::new();
        if !collect_variant_leaves(pat, &mut leaves) {
            // Bare variant name, non-EnumVariant pattern, or an `Or`
            // branch that isn't a qualified EnumVariant — bail
            // conservatively, same as before RES-3934.
            return None;
        }
        for (tn, variant_name) in leaves {
            if let Some(existing) = enum_name_seen {
                if existing != tn {
                    // Multiple different enum type names — bail conservatively.
                    return None;
                }
            } else {
                enum_name_seen = Some(tn);
            }
            matched_variants.insert(variant_name);
        }
    }

    let enum_name = enum_name_seen?;
    let declared = enum_map.get(enum_name)?;

    let missing: Vec<String> = declared
        .iter()
        .filter(|v| !matched_variants.contains(*v))
        .map(|v| v.to_string())
        .collect();

    // RES-3934: variant names the arms reference that aren't declared on
    // this enum at all — surfaced as "did you mean" hints in `check()`.
    let declared_set: HashSet<&str> = declared.iter().copied().collect();
    let mut unrecognized: Vec<String> = matched_variants
        .iter()
        .filter(|v| !declared_set.contains(*v))
        .map(|v| v.to_string())
        .collect();
    unrecognized.sort();

    if missing.is_empty() {
        return None;
    }

    Some(ExhaustivenessError {
        context: context.to_string(),
        enum_name: enum_name.to_string(),
        missing,
        unrecognized,
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
    // RES-3934: if any arm pattern named a variant that isn't declared on
    // this enum at all, it's very likely the *actual* reason a "missing"
    // variant shows up — the user did write an arm for it, just misspelled
    // the name. Surface a "did you mean" hint per unrecognized name,
    // matched against the variants this error already lists as missing.
    let hint: String = e
        .unrecognized
        .iter()
        .map(|u| crate::did_you_mean::hint_from(u, e.missing.iter().map(String::as_str)))
        .filter(|h| !h.is_empty())
        .collect::<Vec<_>>()
        .join("");
    Err(format!(
        "{}: error: non-exhaustive match on enum `{}` in `{}`: missing variant(s): {}{}",
        source_path,
        e.enum_name,
        e.context,
        e.missing.join(", "),
        hint
    ))
}

// ---------- RES-4011: nested/payload pattern exhaustiveness ----------
//
// The checks above (and `struct_exhaustiveness.rs` / the inline gate in
// `typechecker.rs`) only ever look at a match's TOP-LEVEL pattern shape.
// A match like:
//
//   match os {
//       Some(Shape::Circle(r)) => r,
//       None => 0,
//   }
//
// is accepted by every existing pass — `os`'s top level (`Some`/`None`)
// is fully covered, so nothing ever looks at whether `Shape::Circle(r)`
// is itself an exhaustive match against `Shape`'s declared variants. If
// `Shape` also declares `Square`, this program compiles and produces a
// silently wrong runtime value (a `None`-shaped hole) the moment
// `Shape::Square` reaches this position — a soundness hole, not merely
// a missing lint.
//
// This section implements a decision-tree / recursive-descent
// exhaustiveness check over the FULL nested pattern shape: enum variant
// payloads, and `Option`'s `Some(..)` payload, at any depth. It is
// deliberately scoped to constructors whose full domain can be read
// straight off the pattern text with no type inference (declared enum
// variants, `bool` literals, and `Option`'s two built-in variants) —
// anything else (struct payloads, int/string/range literals, `Result`,
// generic type parameters, ...) is treated as an opaque leaf and simply
// not decomposed further, exactly like every check above. That keeps
// this pass strictly additive: it only ever rejects programs no existing
// pass already accepted-by-analysis, never a program the top-level
// checks already prove exhaustive.
//
// Deliberately out of scope (documented, not silently wrong): struct
// payload fields and `Result::Ok`/`Err` payloads are not recursed into.
// `struct_exhaustiveness.rs` already owns bool-domain struct coverage
// at the top level; generalizing that cartesian truth table to nested
// positions is a larger, separate follow-up.

/// One cell in the pattern matrix: either a real sub-pattern borrowed
/// from the AST, or a synthetic wildcard standing in for a position a
/// wildcard/identifier arm expanded into (e.g. `_` at the top level of
/// an enum column expands into `arity` synthetic wildcards, one per
/// declared field, when specializing on a particular variant).
#[derive(Debug, Clone, Copy)]
enum Cell<'p> {
    P(&'p crate::Pattern),
    Wild,
}

fn cell_is_wildcard(c: &Cell) -> bool {
    matches!(
        c,
        Cell::Wild | Cell::P(crate::Pattern::Wildcard) | Cell::P(crate::Pattern::Identifier(_))
    )
}

/// Declared shape of one enum variant, reduced to exactly what the
/// matrix algorithm needs: its name, how many payload fields it has,
/// and (for named-field payloads) the declared field names in order so
/// a `{ field: pat }` sub-pattern can be aligned positionally.
#[derive(Debug, Clone)]
struct VariantMeta<'a> {
    name: &'a str,
    arity: usize,
    /// `Some(names)` for a named-field payload (declaration order);
    /// `None` for a tuple or payload-less variant (positional / empty).
    field_names: Option<Vec<&'a str>>,
}

/// Build `enum_name -> declared variant shapes` for every top-level
/// `Node::EnumDecl`. Unlike `collect_enum_variants` this also records
/// payload arity/field-names so the matrix algorithm can specialize a
/// column into the right number of sub-columns.
fn collect_enum_meta(program: &Node) -> HashMap<&str, Vec<VariantMeta<'_>>> {
    let mut map: HashMap<&str, Vec<VariantMeta<'_>>> = HashMap::new();
    let Node::Program(stmts) = program else {
        return map;
    };
    for s in stmts {
        if let Node::EnumDecl { name, variants, .. } = &s.node {
            let metas = variants
                .iter()
                .map(|v| {
                    let (arity, field_names) = match &v.payload {
                        crate::EnumPayload::None => (0, None),
                        crate::EnumPayload::Tuple(tys) => (tys.len(), None),
                        crate::EnumPayload::Named(fields) => (
                            fields.len(),
                            Some(fields.iter().map(|f| f.name.as_str()).collect()),
                        ),
                    };
                    VariantMeta {
                        name: v.name.as_str(),
                        arity,
                        field_names,
                    }
                })
                .collect();
            map.insert(name.as_str(), metas);
        }
    }
    map
}

/// What column 0 of the (Or/Bind-expanded) matrix decomposes into, read
/// straight off the pattern shapes present. `Skip` covers both "every
/// row is a wildcard here" and "some row uses a shape we don't
/// recursively understand" — both cases are treated identically:
/// drop the column and don't require anything further of it.
enum ColKind {
    Skip,
    Bool,
    OptionK,
    Enum(String),
}

fn detect_col_kind(rows: &[Vec<Cell>]) -> ColKind {
    for r in rows {
        match &r[0] {
            c if cell_is_wildcard(c) => continue,
            Cell::P(crate::Pattern::Literal(Node::BooleanLiteral { .. })) => {
                return ColKind::Bool;
            }
            Cell::P(crate::Pattern::Some(_)) | Cell::P(crate::Pattern::None) => {
                return ColKind::OptionK;
            }
            Cell::P(crate::Pattern::EnumVariant {
                type_name: Some(tn),
                variant_name,
                ..
            }) => {
                if tn == "Option" && (variant_name == "Some" || variant_name == "None") {
                    return ColKind::OptionK;
                }
                return ColKind::Enum(tn.clone());
            }
            _ => return ColKind::Skip,
        }
    }
    ColKind::Skip
}

/// Expand `Or` patterns and unwrap `Bind` patterns sitting at column 0,
/// recursively, until no row's column 0 is an `Or`/`Bind`. Row order is
/// irrelevant to exhaustiveness (only the covered *set* matters), so a
/// simple work-stack is enough.
fn expand_col0<'p>(rows: Vec<Vec<Cell<'p>>>) -> Vec<Vec<Cell<'p>>> {
    let mut out = Vec::with_capacity(rows.len());
    let mut stack = rows;
    while let Some(mut row) = stack.pop() {
        match &row[0] {
            Cell::P(crate::Pattern::Or(branches)) => {
                for b in branches {
                    let mut nr = row.clone();
                    nr[0] = Cell::P(b);
                    stack.push(nr);
                }
            }
            Cell::P(crate::Pattern::Bind(_, inner)) => {
                row[0] = Cell::P(inner.as_ref());
                stack.push(row);
            }
            _ => out.push(row),
        }
    }
    out
}

fn chain_wild<'p>(n: usize, rest: &[Cell<'p>]) -> Vec<Cell<'p>> {
    let mut v = vec![Cell::Wild; n];
    v.extend_from_slice(rest);
    v
}

fn chain_one<'p>(c: Cell<'p>, rest: &[Cell<'p>]) -> Vec<Cell<'p>> {
    let mut v = Vec::with_capacity(rest.len() + 1);
    v.push(c);
    v.extend_from_slice(rest);
    v
}

fn chain_many<'p>(cells: Vec<Cell<'p>>, rest: &[Cell<'p>]) -> Vec<Cell<'p>> {
    let mut v = cells;
    v.extend_from_slice(rest);
    v
}

/// A single field of an `EnumVariant` sub-pattern's payload, as a `Cell`
/// aligned to `variant`'s declared field order. Defensive fallbacks
/// (missing/extra fields) return `Cell::Wild` rather than panicking —
/// genuine arity mismatches are already a hard error from
/// `enum_payload_match::check`, so this path is unreachable for valid
/// programs and must never be the thing that panics on an invalid one.
fn extract_payload_cells<'p>(
    payload: &'p crate::EnumPatternPayload,
    variant: &VariantMeta,
) -> Vec<Cell<'p>> {
    match payload {
        crate::EnumPatternPayload::None => Vec::new(),
        crate::EnumPatternPayload::Tuple(subs) => {
            let mut cells: Vec<Cell> = subs.iter().map(Cell::P).collect();
            cells.truncate(variant.arity);
            while cells.len() < variant.arity {
                cells.push(Cell::Wild);
            }
            cells
        }
        crate::EnumPatternPayload::Named(fields) => match &variant.field_names {
            Some(names) => names
                .iter()
                .map(|fname| {
                    fields
                        .iter()
                        .find(|(n, _)| n == fname)
                        .map(|(_, p)| Cell::P(p.as_ref()))
                        .unwrap_or(Cell::Wild)
                })
                .collect(),
            None => {
                let mut cells: Vec<Cell> =
                    fields.iter().map(|(_, p)| Cell::P(p.as_ref())).collect();
                cells.truncate(variant.arity);
                while cells.len() < variant.arity {
                    cells.push(Cell::Wild);
                }
                cells
            }
        },
    }
}

/// The single payload cell of a qualified `Option::Some(x)` pattern
/// (`Pattern::EnumVariant { variant_name: "Some", .. }`) — the
/// unqualified `Some(x)` form parses to `Pattern::Some` directly and is
/// handled separately.
fn extract_option_some_cell(payload: &crate::EnumPatternPayload) -> Cell<'_> {
    match payload {
        crate::EnumPatternPayload::Tuple(subs) if !subs.is_empty() => Cell::P(&subs[0]),
        crate::EnumPatternPayload::Named(fields) if !fields.is_empty() => {
            Cell::P(fields[0].1.as_ref())
        }
        _ => Cell::Wild,
    }
}

/// Breadcrumb of constructors traversed on the way to a missing case,
/// outermost first, plus a human-readable description of what's
/// missing at the innermost point.
#[derive(Debug, Clone)]
struct Witness {
    path: Vec<String>,
    missing: String,
}

/// A self-referential enum (`enum Expr { Add(Expr, Expr), Lit(int) }`)
/// would otherwise recurse forever resolving its own payload's domain.
/// Bounding both by an explicit visited-set (an enum already on the
/// active recursion stack resolves to `Skip` instead of recursing
/// again) and a hard depth cap keeps this pass total on any input.
const MAX_NESTING_DEPTH: usize = 8;

/// The core decision-tree exhaustiveness check. `rows` holds one Vec
/// per still-reachable arm; every row has the same length (the number
/// of pending columns still to be resolved). Returns `Ok(())` when
/// every combination reachable through the pending columns is covered,
/// or `Err(Witness)` naming one uncovered combination.
fn matrix_exhaustive<'p>(
    rows: Vec<Vec<Cell<'p>>>,
    enum_meta: &HashMap<&str, Vec<VariantMeta<'_>>>,
    stack: &mut Vec<String>,
) -> Result<(), Witness> {
    if rows.is_empty() {
        return Err(Witness {
            path: Vec::new(),
            missing: "_".to_string(),
        });
    }
    if rows[0].is_empty() {
        return Ok(());
    }
    let rows = expand_col0(rows);
    if rows.is_empty() {
        return Err(Witness {
            path: Vec::new(),
            missing: "_".to_string(),
        });
    }

    match detect_col_kind(&rows) {
        ColKind::Skip => {
            let next: Vec<Vec<Cell>> = rows
                .into_iter()
                .map(|mut r| {
                    r.remove(0);
                    r
                })
                .collect();
            matrix_exhaustive(next, enum_meta, stack)
        }
        ColKind::Bool => {
            for want in [true, false] {
                let specialized: Vec<Vec<Cell>> = rows
                    .iter()
                    .filter_map(|r| match &r[0] {
                        c if cell_is_wildcard(c) => Some(r[1..].to_vec()),
                        Cell::P(crate::Pattern::Literal(Node::BooleanLiteral {
                            value, ..
                        })) if *value == want => Some(r[1..].to_vec()),
                        _ => None,
                    })
                    .collect();
                if specialized.is_empty() {
                    return Err(Witness {
                        path: Vec::new(),
                        missing: format!("`{}`", want),
                    });
                }
                if let Err(mut w) = matrix_exhaustive(specialized, enum_meta, stack) {
                    w.path.insert(0, format!("{}", want));
                    return Err(w);
                }
            }
            Ok(())
        }
        ColKind::OptionK => {
            let some_rows: Vec<Vec<Cell>> = rows
                .iter()
                .filter_map(|r| match &r[0] {
                    c if cell_is_wildcard(c) => Some(chain_wild(1, &r[1..])),
                    Cell::P(crate::Pattern::Some(inner)) => {
                        Some(chain_one(Cell::P(inner.as_ref()), &r[1..]))
                    }
                    Cell::P(crate::Pattern::EnumVariant {
                        variant_name,
                        payload,
                        ..
                    }) if variant_name == "Some" => {
                        Some(chain_one(extract_option_some_cell(payload), &r[1..]))
                    }
                    _ => None,
                })
                .collect();
            if some_rows.is_empty() {
                return Err(Witness {
                    path: Vec::new(),
                    missing: "`Some(_)`".to_string(),
                });
            }
            if let Err(mut w) = matrix_exhaustive(some_rows, enum_meta, stack) {
                w.path.insert(0, "Some(..)".to_string());
                return Err(w);
            }

            let none_rows: Vec<Vec<Cell>> = rows
                .iter()
                .filter_map(|r| match &r[0] {
                    c if cell_is_wildcard(c) => Some(r[1..].to_vec()),
                    Cell::P(crate::Pattern::None) => Some(r[1..].to_vec()),
                    Cell::P(crate::Pattern::EnumVariant { variant_name, .. })
                        if variant_name == "None" =>
                    {
                        Some(r[1..].to_vec())
                    }
                    _ => None,
                })
                .collect();
            if none_rows.is_empty() {
                return Err(Witness {
                    path: Vec::new(),
                    missing: "`None`".to_string(),
                });
            }
            matrix_exhaustive(none_rows, enum_meta, stack)
        }
        ColKind::Enum(name) => {
            let Some(variants) = enum_meta.get(name.as_str()) else {
                // Unknown to `enum_meta` (e.g. `Result`, a type-param, or
                // any other builtin/unmodeled qualifier) — not ours to
                // check; drop the column and move on, same as `Skip`.
                let next: Vec<Vec<Cell>> = rows
                    .into_iter()
                    .map(|mut r| {
                        r.remove(0);
                        r
                    })
                    .collect();
                return matrix_exhaustive(next, enum_meta, stack);
            };
            if stack.iter().any(|s| s == &name) || stack.len() >= MAX_NESTING_DEPTH {
                let next: Vec<Vec<Cell>> = rows
                    .into_iter()
                    .map(|mut r| {
                        r.remove(0);
                        r
                    })
                    .collect();
                return matrix_exhaustive(next, enum_meta, stack);
            }
            stack.push(name.clone());
            let result = (|| {
                for v in variants {
                    let specialized: Vec<Vec<Cell>> = rows
                        .iter()
                        .filter_map(|r| match &r[0] {
                            c if cell_is_wildcard(c) => Some(chain_wild(v.arity, &r[1..])),
                            Cell::P(crate::Pattern::EnumVariant {
                                variant_name,
                                payload,
                                ..
                            }) if variant_name.as_str() == v.name => {
                                Some(chain_many(extract_payload_cells(payload, v), &r[1..]))
                            }
                            _ => None,
                        })
                        .collect();
                    if specialized.is_empty() {
                        return Err(Witness {
                            path: Vec::new(),
                            missing: format!("`{}::{}`", name, v.name),
                        });
                    }
                    if let Err(mut w) = matrix_exhaustive(specialized, enum_meta, stack) {
                        w.path.insert(0, format!("{}::{}(..)", name, v.name));
                        return Err(w);
                    }
                }
                Ok(())
            })();
            stack.pop();
            result
        }
    }
}

/// One non-exhaustive nested pattern finding.
#[derive(Debug, Clone)]
pub struct NestedExhaustivenessError {
    pub context: String,
    pub message: String,
}

fn format_witness(w: &Witness) -> String {
    if w.path.is_empty() {
        format!(
            "missing case {} — add a wildcard `_` or identifier arm to cover it",
            w.missing
        )
    } else {
        format!(
            "inside `{}`, missing case {} — add a wildcard `_` or identifier arm to cover it",
            w.path.join(" -> "),
            w.missing
        )
    }
}

/// Check a single `Node::Match`'s arms for nested (beyond-top-level)
/// non-exhaustiveness. Guard-blind, matching the conservative behavior
/// of `check_match` above: a guarded arm's pattern still counts toward
/// coverage since a guard may fail, so treating it as *not* covering
/// would be strictly more precise but is out of scope here — the same
/// documented gap already exists at the top level.
fn check_one_match(
    arms: &[(crate::Pattern, Option<Node>, Node)],
    enum_meta: &HashMap<&str, Vec<VariantMeta<'_>>>,
) -> Option<Witness> {
    let rows: Vec<Vec<Cell>> = arms.iter().map(|(p, _guard, _)| vec![Cell::P(p)]).collect();
    let mut stack = Vec::new();
    matrix_exhaustive(rows, enum_meta, &mut stack).err()
}

fn walk_nested(
    node: &Node,
    enum_meta: &HashMap<&str, Vec<VariantMeta<'_>>>,
    context: &str,
    errors: &mut Vec<NestedExhaustivenessError>,
) {
    if let Node::Function { name, body, .. } = node {
        walk_nested(body, enum_meta, name.as_str(), errors);
        return;
    }
    if let Node::Match { arms, .. } = node {
        if let Some(w) = check_one_match(arms, enum_meta) {
            errors.push(NestedExhaustivenessError {
                context: context.to_string(),
                message: format_witness(&w),
            });
        }
    }
    let mut nested: Vec<NestedExhaustivenessError> = Vec::new();
    crate::uniqueness_walk::visit(node, &mut |n| {
        if matches!(n, Node::Function { .. }) {
            return;
        }
        if let Node::Match { arms, .. } = n {
            if let Some(w) = check_one_match(arms, enum_meta) {
                nested.push(NestedExhaustivenessError {
                    context: context.to_string(),
                    message: format_witness(&w),
                });
            }
        }
    });
    errors.extend(nested);
}

/// Analyze the program for non-exhaustive NESTED patterns — i.e. a
/// match whose top-level shape is already fully covered (or is beyond
/// this pass's scope) but whose nested enum-variant / `Option` payload
/// patterns are not. See the module-level comment above for the exact
/// scope (`RES-4011`).
pub fn analyze_nested(program: &Node) -> Vec<NestedExhaustivenessError> {
    let enum_meta = collect_enum_meta(program);
    let mut errors = Vec::new();
    let Node::Program(stmts) = program else {
        return errors;
    };
    for s in stmts {
        walk_nested(&s.node, &enum_meta, "<top-level>", &mut errors);
    }
    errors
}

/// Type-checker entry point (RES-4011). Returns `Err` on the first
/// non-exhaustive nested pattern found.
pub(crate) fn check_nested(program: &Node, source_path: &str) -> Result<(), String> {
    let errs = analyze_nested(program);
    if errs.is_empty() {
        return Ok(());
    }
    let e = &errs[0];
    Err(format!(
        "{}: error: non-exhaustive nested match pattern in `{}`: {}",
        source_path, e.context, e.message
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

    // RES-3934: adversarial corpus — or-patterns, guard interaction,
    // range/struct payloads, and nested-pattern completeness.

    /// Bug fix: an or-pattern arm (`Color::Red | Color::Green => ...`)
    /// used to make `check_match` bail conservatively (treated as a
    /// non-EnumVariant pattern), so a match that was ACTUALLY exhaustive
    /// via an or-pattern reported no error only by accident — and the
    /// mirror case (a genuinely non-exhaustive or-pattern match) also
    /// silently passed. RES-3934 flattens `Or` into its branches so
    /// coverage is unioned correctly in both directions.
    #[test]
    fn or_pattern_covering_two_variants_is_exhaustive() {
        let src = r#"
enum Color { Red, Green, Blue }
fn describe(Color c) -> int {
    return match c {
        Color::Red | Color::Green => 0,
        Color::Blue => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert!(
            errs.is_empty(),
            "or-pattern covering Red+Green plus a Blue arm is exhaustive; got: {:?}",
            errs
        );
    }

    #[test]
    fn or_pattern_missing_variant_is_detected() {
        // Only Red and Green are covered (via the or-pattern) — Blue
        // is missing and must still be reported.
        let src = r#"
enum Color { Red, Green, Blue }
fn describe(Color c) -> int {
    return match c {
        Color::Red | Color::Green => 0,
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
            errs[0].missing.contains(&"Blue".to_string()),
            "missing variants should include Blue; got: {:?}",
            errs[0].missing
        );
    }

    #[test]
    fn or_pattern_with_payload_variants_all_covered_no_error() {
        // Or-pattern branches over payload-carrying variants — still a
        // pure variant-name union, payload shape doesn't matter here.
        let src = r#"
enum Expr {
    Lit(int),
    Add(Expr, Expr),
    Neg(Expr),
}
fn is_leaf(Expr e) -> bool {
    return match e {
        Expr::Lit(n) => true,
        Expr::Add(a, b) | Expr::Neg(a) => false,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert!(
            errs.is_empty(),
            "or-pattern over two payload variants plus Lit is exhaustive; got: {:?}",
            errs
        );
    }

    #[test]
    fn or_pattern_mixing_different_enums_bails_conservatively() {
        // `Color::Red | Status::Ok` can't type-check as a real program
        // (mismatched scrutinee type), but at the AST level this module
        // must still bail rather than panic or misattribute variants.
        let src = r#"
enum Color { Red, Green }
enum Status { Ok, Err }
fn f(Color c) -> int {
    return match c {
        Color::Red | Status::Ok => 0,
        Color::Green => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        // Must not panic; bailing (empty) is the documented conservative
        // behavior for arms whose patterns don't resolve to one enum.
        let errs = analyze(&prog);
        assert!(
            errs.is_empty(),
            "mixed-enum or-pattern must bail conservatively, not report a spurious error; got: {:?}",
            errs
        );
    }

    /// Documents a known, intentional conservative gap (see the module
    /// doc comment): a guarded `EnumVariant` arm is treated as covering
    /// its variant even though the guard might not fire at runtime, so
    /// this match compiles even though `Color::Red` is only reachable
    /// when `flag` is true. Tightening this requires proving something
    /// about the guard expression, which is out of scope for this
    /// AST-only pass — tracked as a follow-up rather than fixed here.
    #[test]
    fn guarded_only_arm_conservatively_counts_as_covering_its_variant() {
        let src = r#"
enum Color { Red, Green }
fn describe(Color c, bool flag) -> int {
    return match c {
        Color::Red if flag => 0,
        Color::Green => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert!(
            errs.is_empty(),
            "documented conservative behavior: guarded arm counts as covering; got: {:?}",
            errs
        );
    }

    /// Struct-pattern + enum-payload combination: a variant's payload is
    /// itself a struct, destructured in the arm. Variant-level coverage
    /// must still be tracked correctly regardless of how complex the
    /// nested payload pattern is — this module only reasons about
    /// variant names, never payload shape.
    #[test]
    fn struct_payload_variant_coverage_tracked_independent_of_nested_pattern() {
        let src = r#"
struct Point { int x, int y }
enum Shape {
    Circle(Point),
    Square(Point),
}
fn describe(Shape s) -> int {
    return match s {
        Shape::Circle(Point { x, y }) => x,
        Shape::Square(Point { .. }) => 0,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert!(
            errs.is_empty(),
            "both variants covered despite nested struct payload patterns; got: {:?}",
            errs
        );
    }

    #[test]
    fn struct_payload_variant_missing_still_detected() {
        let src = r#"
struct Point { int x, int y }
enum Shape {
    Circle(Point),
    Square(Point),
}
fn describe(Shape s) -> int {
    return match s {
        Shape::Circle(Point { x, y }) => x,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert_eq!(errs.len(), 1, "expected one exhaustiveness error");
        assert!(
            errs[0].missing.contains(&"Square".to_string()),
            "missing variants should include Square; got: {:?}",
            errs[0].missing
        );
    }

    /// Documents a known scope limitation: this module checks *variant*
    /// coverage only, never payload *value* coverage. A range sub-pattern
    /// that only covers part of the int domain (`0..10`) is treated the
    /// same as any other payload pattern — the variant counts as covered.
    /// Full payload-value exhaustiveness (union of ranges/literals over
    /// an unbounded int domain) is a materially different, much larger
    /// analysis and is out of scope for this pass.
    #[test]
    fn range_payload_subpattern_does_not_affect_variant_coverage() {
        let src = r#"
enum Reading {
    Value(int),
    Unavailable,
}
fn classify(Reading r) -> int {
    return match r {
        Reading::Value(0..10) => 0,
        Reading::Unavailable => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert!(
            errs.is_empty(),
            "variant-level coverage doesn't inspect payload range completeness; got: {:?}",
            errs
        );
    }

    /// RES-3934: documents `analyze()`'s permanent scope — it only ever
    /// inspects the TOP-LEVEL arm pattern of a `Node::Match` and has no
    /// concept of "nested match completeness" for an enum pattern
    /// embedded inside another pattern (e.g. `Some(Shape::Circle(r))`
    /// nested inside an `Option<Shape>` match). `analyze()` itself is
    /// intentionally left unchanged (many callers/tests rely on its
    /// top-level-only wording and did-you-mean hints), so this
    /// assertion stays true.
    ///
    /// RES-4011: the actual soundness hole this documented — an
    /// uncovered nested variant compiling and producing a wrong runtime
    /// value instead of a compile error — is now closed by the sibling
    /// `analyze_nested()` / `check_nested()` pass below, which *does*
    /// recurse into enum-variant payloads and `Option`'s `Some(..)`
    /// payload. See `res4011_nested_exhaustiveness::
    /// missing_nested_variant_inside_some_is_detected` for the same
    /// program now being rejected end-to-end.
    #[test]
    fn nested_enum_pattern_inside_option_is_not_analyzed() {
        let src = r#"
enum Shape {
    Circle(int),
    Square(int),
}
fn f(Option<Shape> os) -> int {
    return match os {
        Some(Shape::Circle(r)) => r,
        None => 0,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze(&prog);
        assert!(
            errs.is_empty(),
            "documents current scope limitation — nested pattern completeness \
             is not checked by this module; got: {:?}",
            errs
        );
    }

    // RES-3934: did-you-mean hint for a mistyped variant name.

    #[test]
    fn typo_variant_name_gets_did_you_mean_hint() {
        // `Statuss::Pending` doesn't exist — presumably a typo of `Pending`.
        // `Ok`/`Err` are covered under their real names, but `Pending` is
        // both genuinely missing AND was (mis)referenced under a typo'd
        // name, so the hint should point back at it.
        let src = r#"
enum Status { Ok, Err, Pending }
fn handle(Status s) -> int {
    return match s {
        Status::Ok => 0,
        Status::Err => 1,
        Status::Pendign => 2,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let result = check(&prog, "test.rz");
        assert!(result.is_err(), "expected check to fail");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("Pending"),
            "error must name missing variant 'Pending': {msg}"
        );
        assert!(
            msg.contains("did you mean"),
            "error should hint at the typo'd `Pendign` arm: {msg}"
        );
        assert!(
            msg.contains("`Pending`"),
            "hint should suggest the real variant name 'Pending': {msg}"
        );
    }

    #[test]
    fn no_typo_no_did_you_mean_hint() {
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
        let msg = result.unwrap_err();
        assert!(
            !msg.contains("did you mean"),
            "no unrecognized variant names — no hint expected: {msg}"
        );
    }
}

// RES-4011: nested/payload pattern exhaustiveness — `analyze_nested` /
// `check_nested` tests. See the module-level comment above those
// functions for the exact scope (enum-variant payloads and `Option`'s
// `Some(..)` payload, recursively; everything else is left alone).
#[cfg(test)]
mod res4011_nested_exhaustiveness {
    use super::*;

    /// The exact motivating example from issue #4011: `Shape` declares
    /// `Square` but only `Circle` is covered inside the `Some(..)` arm.
    /// This compiled silently before RES-4011 and produced a wrong
    /// runtime value the moment `Shape::Square` reached this position.
    #[test]
    fn missing_nested_variant_inside_some_is_detected() {
        let src = r#"
enum Shape {
    Circle(int),
    Square(int),
}
fn f(Option<Shape> os) -> int {
    return match os {
        Some(Shape::Circle(r)) => r,
        None => 0,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze_nested(&prog);
        assert_eq!(
            errs.len(),
            1,
            "expected exactly one nested exhaustiveness error; got: {:?}",
            errs
        );
        assert!(
            errs[0].message.contains("Shape::Square"),
            "error must name the missing nested variant Shape::Square: {}",
            errs[0].message
        );
    }

    /// The same shape, fully covered — must not be flagged.
    #[test]
    fn fully_covered_nested_variant_inside_some_is_accepted() {
        let src = r#"
enum Shape {
    Circle(int),
    Square(int),
}
fn f(Option<Shape> os) -> int {
    return match os {
        Some(Shape::Circle(r)) => r,
        Some(Shape::Square(s)) => s,
        None => 0,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze_nested(&prog);
        assert!(
            errs.is_empty(),
            "both Shape variants covered inside Some — expected no errors; got: {:?}",
            errs
        );
    }

    /// A wildcard/identifier at the nested position covers every
    /// remaining variant, exactly like at the top level.
    #[test]
    fn wildcard_at_nested_position_is_accepted() {
        let src = r#"
enum Shape {
    Circle(int),
    Square(int),
}
fn f(Option<Shape> os) -> int {
    return match os {
        Some(s) => 0,
        None => 1,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze_nested(&prog);
        assert!(
            errs.is_empty(),
            "identifier binding at the nested position covers every Shape variant; got: {:?}",
            errs
        );
    }

    /// Nesting more than one level deep: `Some(Wrap::Has(Shape::Circle(_)))`
    /// with `Shape::Square` never covered.
    #[test]
    fn two_levels_of_nesting_missing_case_is_detected() {
        let src = r#"
enum Shape {
    Circle(int),
    Square(int),
}
enum Wrap {
    Has(Shape),
}
fn f(Option<Wrap> ow) -> int {
    return match ow {
        Some(Wrap::Has(Shape::Circle(r))) => r,
        None => 0,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze_nested(&prog);
        assert_eq!(errs.len(), 1, "expected one nested error; got: {:?}", errs);
        assert!(
            errs[0].message.contains("Shape::Square"),
            "error must name the missing deeply-nested variant: {}",
            errs[0].message
        );
    }

    /// A self-referential enum (`Add(Expr, Expr)`) must not cause
    /// infinite recursion — plain identifier bindings at every payload
    /// position are trivially exhaustive regardless of recursion depth.
    #[test]
    fn self_referential_enum_does_not_infinite_loop() {
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
        Expr::Neg(x) => 0,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze_nested(&prog);
        assert!(
            errs.is_empty(),
            "self-referential Expr with identifier bindings is exhaustive; got: {:?}",
            errs
        );
    }

    /// Bool literals at a nested position are also checked: `Some(true)`
    /// alone is missing the `Some(false)` case.
    #[test]
    fn missing_nested_bool_case_is_detected() {
        let src = r#"
fn f(Option<bool> ob) -> int {
    return match ob {
        Some(true) => 1,
        None => 0,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze_nested(&prog);
        assert_eq!(errs.len(), 1, "expected one nested error; got: {:?}", errs);
        assert!(
            errs[0].message.contains("false"),
            "error must name the missing `false` case: {}",
            errs[0].message
        );
    }

    /// Struct payloads are intentionally out of scope for this pass —
    /// mirrors the top-level module's own documented scope limit
    /// (`struct_payload_variant_coverage_tracked_independent_of_nested_pattern`).
    /// `Shape` itself is fully covered (`At` + `Origin`, both reachable
    /// through `Some`/`None`); the struct destructure inside `At`'s
    /// payload must not be decomposed field-by-field or otherwise
    /// confuse the recursion — this must report no error.
    #[test]
    fn struct_payload_at_nested_position_is_not_analyzed() {
        let src = r#"
struct Point { int x, int y }
enum Shape {
    At(Point),
    Origin,
}
fn f(Option<Shape> os) -> int {
    return match os {
        Some(Shape::At(Point { x, y })) => x,
        Some(Shape::Origin) => 0,
        None => 0,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let errs = analyze_nested(&prog);
        assert!(
            errs.is_empty(),
            "Shape fully covered; struct payload fields are out of scope — expected no errors; got: {:?}",
            errs
        );
    }

    /// `check_nested` surfaces the diagnostic through the `Result`
    /// entry point used by the typechecker.
    #[test]
    fn check_nested_errors_on_missing_case() {
        let src = r#"
enum Shape {
    Circle(int),
    Square(int),
}
fn f(Option<Shape> os) -> int {
    return match os {
        Some(Shape::Circle(r)) => r,
        None => 0,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        let result = check_nested(&prog, "test.rz");
        assert!(result.is_err(), "expected check_nested to fail");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("non-exhaustive nested match pattern"),
            "error must contain 'non-exhaustive nested match pattern': {msg}"
        );
        assert!(msg.contains("Shape::Square"), "error text: {msg}");
    }

    #[test]
    fn check_nested_ok_for_fully_covered_program() {
        let src = r#"
enum Shape {
    Circle(int),
    Square(int),
}
fn f(Option<Shape> os) -> int {
    return match os {
        Some(Shape::Circle(r)) => r,
        Some(Shape::Square(s)) => s,
        None => 0,
    };
}
"#;
        let (prog, _) = crate::parse(src);
        assert!(check_nested(&prog, "test.rz").is_ok());
    }
}
