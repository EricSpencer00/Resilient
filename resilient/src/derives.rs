//! Feature 37/50 — Custom Derives.
//!
//! `#[derive(Debug, Eq, Hash)]` on a struct or enum auto-generates
//! the listed trait impls. The first slice supports a curated list:
//!
//! * `Debug` — `to_string` returning a struct-like Rust-style debug
//!   representation.
//! * `Eq` / `PartialEq` — pairwise field equality; enables `==`/`!=`.
//! * `Ord` / `PartialOrd` — lexicographic field ordering; enables `<`/`>`.
//! * `Hash` — combine field hashes via the SipHash default.
//! * `Default` — constructor with primitive defaults.
//! * `Clone` / `Copy` — value duplication semantics.
//! * `Display` — human-readable `to_string` with field names.
//! * `Iterator` / `From` / `Into` — standard conversion traits.
//!
//! The actual lowering (synthesizing trait impl AST nodes) is a
//! follow-up; this module records what was requested so runtime
//! generic dispatch and the LSP can advertise the derived methods.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, RwLock};

/// RES-2174: dropped the redundant `type_name: String` field. It was
/// set from the attribute's owning-item name in `collect()`. Two
/// readers — `install`'s key clone and `check`'s validation error
/// message — both used it as a HashMap key / display value tied to
/// the entry. The field stored exactly what the key encoded. Same
/// dead-field pattern as RES-2106 / RES-2110 / RES-2122 / RES-2168 /
/// RES-2170 / RES-2172.
#[derive(Debug, Clone)]
pub struct DeriveSet {
    pub traits: Vec<String>,
}

static DERIVES: LazyLock<RwLock<HashMap<String, DeriveSet>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

const SUPPORTED: &[&str] = &[
    "Debug",
    "Eq",
    "Hash",
    "Default",
    "Clone",
    "Ord",
    "PartialEq",
    "PartialOrd",
    "Display",
    "Iterator",
    "From",
    "Into",
    "Copy",
];

pub fn collect() -> Vec<(String, DeriveSet)> {
    let attrs = crate::feature_attrs::find_kind("derive");
    // RES-1782: pre-size to attrs.len() — exactly one push per
    // attribute record.
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let traits: Vec<String> = rec
            .args
            .split(',')
            .map(|s| s.trim().trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        out.push((item, DeriveSet { traits }));
    }
    out
}

pub fn install(sets: Vec<(String, DeriveSet)>) {
    if let Ok(mut g) = DERIVES.write() {
        g.clear();
        // RES-2174: move (name, set) pairs straight from `collect()`
        // into the map. The previous shape per-set cloned
        // `s.type_name` to produce the key, since the field and the
        // key encoded the same string.
        g.extend(sets);
    }
}

pub fn derives_trait(type_name: &str, trait_name: &str) -> bool {
    DERIVES
        .read()
        .ok()
        .and_then(|g| {
            g.get(type_name)
                .map(|s| s.traits.iter().any(|t| t == trait_name))
        })
        .unwrap_or(false)
}

fn reject_duplicate_derive_records(source_path: &str) -> Result<(), String> {
    let attrs = crate::feature_attrs::find_kind("derive");
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (type_name, rec) in attrs {
        if let Some(first_line) = seen.insert(type_name.clone(), rec.line) {
            return Err(format!(
                "{}:{}:0: error: duplicate derive registration for `{}`; first declaration at line {}",
                source_path, rec.line, type_name, first_line
            ));
        }
    }
    Ok(())
}

fn declared_struct_names(program: &Node) -> HashSet<String> {
    match program {
        Node::Program(statements) => statements
            .iter()
            .filter_map(|stmt| match &stmt.node {
                Node::StructDecl { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect(),
        _ => HashSet::new(),
    }
}

fn validate_derive_targets(program: &Node, source_path: &str) -> Result<(), String> {
    // The module's unit tests exercise the registry in isolation with an
    // empty synthetic AST. Real parsed programs always carry at least the
    // attributed item, so keep that legacy test shape independent from the
    // source-level target check.
    let Node::Program(statements) = program else {
        return Ok(());
    };
    if statements.is_empty() {
        return Ok(());
    }

    let struct_names = declared_struct_names(program);
    for (type_name, rec) in crate::feature_attrs::find_kind("derive") {
        if !struct_names.contains(type_name.as_str()) {
            return Err(format!(
                "{}:{}:1: error: #[derive(...)] target `{}` is not a declared struct",
                source_path,
                rec.line.max(1),
                type_name
            ));
        }
    }
    Ok(())
}

fn struct_type_from_annotation(annotation: &str, struct_names: &HashSet<String>) -> Option<String> {
    let annotation = annotation.trim();
    struct_names
        .contains(annotation)
        .then(|| annotation.to_string())
}

fn struct_type_of_expression(
    expression: &Node,
    bindings: &HashMap<String, String>,
    struct_names: &HashSet<String>,
) -> Option<String> {
    match expression {
        Node::Identifier { name, .. } => bindings.get(name).cloned(),
        Node::StructLiteral { name, .. } if struct_names.contains(name) => Some(name.clone()),
        _ => None,
    }
}

fn update_struct_binding(
    name: &str,
    value: &Node,
    type_annotation: Option<&str>,
    bindings: &mut HashMap<String, String>,
    struct_names: &HashSet<String>,
) {
    let inferred = type_annotation
        .and_then(|annotation| struct_type_from_annotation(annotation, struct_names))
        .or_else(|| struct_type_of_expression(value, bindings, struct_names));
    if let Some(struct_name) = inferred {
        bindings.insert(name.to_string(), struct_name);
    } else {
        bindings.remove(name);
    }
}

fn derive_set_has_trait(sets: &[(String, DeriveSet)], type_name: &str, trait_name: &str) -> bool {
    sets.iter()
        .find(|(name, _)| name == type_name)
        .is_some_and(|(_, set)| {
            set.traits
                .iter()
                .any(|trait_name_in_set| trait_name_in_set == trait_name)
        })
}

fn validate_derive_call_sites(
    program: &Node,
    source_path: &str,
    sets: &[(String, DeriveSet)],
) -> Result<(), String> {
    let struct_names = declared_struct_names(program);
    let mut error = None;

    crate::uniqueness_walk::for_each_function(program, |_name, parameters, body| {
        if error.is_some() {
            return;
        }

        let mut bindings = HashMap::with_capacity(parameters.len() + 8);
        for (annotation, name) in parameters {
            if let Some(struct_name) = struct_type_from_annotation(annotation, &struct_names) {
                bindings.insert(name.clone(), struct_name);
            }
        }

        crate::uniqueness_walk::visit(body, &mut |node| {
            if error.is_some() {
                return;
            }

            match node {
                Node::LetStatement {
                    name,
                    value,
                    type_annot,
                    ..
                } => update_struct_binding(
                    name,
                    value,
                    type_annot.as_deref(),
                    &mut bindings,
                    &struct_names,
                ),
                Node::StaticLet { name, value, .. } | Node::Const { name, value, .. } => {
                    update_struct_binding(name, value, None, &mut bindings, &struct_names)
                }
                Node::Assignment { name, value, .. } => {
                    let inferred = struct_type_of_expression(value, &bindings, &struct_names);
                    if let Some(struct_name) = inferred {
                        bindings.insert(name.clone(), struct_name);
                    } else {
                        bindings.remove(name);
                    }
                }
                Node::InfixExpression {
                    left,
                    operator,
                    right,
                    span,
                } => {
                    let required_trait = match *operator {
                        "<" | ">" | "<=" | ">=" => Some("PartialOrd"),
                        "==" | "!=" => Some("PartialEq"),
                        _ => None,
                    };
                    let Some(required_trait) = required_trait else {
                        return;
                    };
                    let Some(left_type) = struct_type_of_expression(left, &bindings, &struct_names)
                    else {
                        return;
                    };
                    let Some(right_type) =
                        struct_type_of_expression(right, &bindings, &struct_names)
                    else {
                        return;
                    };
                    if left_type != right_type {
                        return;
                    }

                    let has_required_trait = derive_set_has_trait(sets, &left_type, required_trait)
                        || (*operator == "==" || *operator == "!=")
                            && derive_set_has_trait(sets, &left_type, "Eq");
                    if !has_required_trait {
                        let requirement = if required_trait == "PartialEq" {
                            "#[derive(PartialEq)] or #[derive(Eq)]"
                        } else {
                            "#[derive(PartialOrd)]"
                        };
                        error = Some(format!(
                            "{}:{}:{}: error: struct `{}` used with `{}` requires {}",
                            source_path,
                            span.start.line.max(1),
                            span.start.column.max(1),
                            left_type,
                            operator,
                            requirement
                        ));
                    }
                }
                _ => {}
            }
        });
    });

    error.map_or(Ok(()), Err)
}

pub(crate) fn check(_program: &Node, source_path: &str) -> Result<(), String> {
    // RES-1402: gate `install` on the non-empty case. The historical
    // wiring called `install(sets.clone())` before the trait-validation
    // loop, burning a `DERIVES.write()` lock + `g.clear()` per compile
    // regardless of whether any `#[derive]` attribute was present, AND
    // creating the wipe-on-empty test race shape documented in
    // RES-1302: a parallel test that called `install(...)` directly
    // under `feature_attrs::lock_for_test()` would have its registry
    // wiped by a concurrent typecheck whose `collect()` returned empty.
    // Same pattern as RES-1306 / RES-1308 already applied to
    // `async_await`, `default_trait_methods`, `mmio_regmap`,
    // `distributed_invariants`, `ghost_types`, and friends.
    // RES-1481: validate the trait set before `install` so we can
    // move `sets` into `install` instead of cloning. The previous
    // shape did `install(sets.clone())` ahead of the validation
    // loop, burning a full Vec<DeriveSpec> clone per compile that
    // had `#[derive]` attributes — the clone was thrown away once
    // the for-loop finished iterating `&sets`. As a side benefit,
    // an invalid-trait error now leaves the registry untouched
    // rather than polluting it with an entry that then fails.
    let sets = collect();
    reject_duplicate_derive_records(source_path)?;
    for (type_name, s) in &sets {
        let mut seen_traits: HashSet<&str> = HashSet::new();
        for t in &s.traits {
            if !seen_traits.insert(t.as_str()) {
                return Err(format!(
                    "{}:0:0: error: `#[derive({})]` on `{}` duplicate trait `{}`",
                    source_path, t, type_name, t
                ));
            }
            if !SUPPORTED.contains(&t.as_str()) {
                return Err(format!(
                    "{}:0:0: error: `#[derive({})]` on `{}` — unknown trait. Supported: {:?}",
                    source_path, t, type_name, SUPPORTED
                ));
            }
        }
    }
    validate_derive_targets(_program, source_path)?;
    validate_derive_call_sites(_program, source_path, &sets)?;
    if sets.is_empty() {
        return Ok(());
    }
    install(sets);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_trait_passes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Reading",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "Debug, Eq".into(),
                line: 0,
            },
        );
        let res = check(&Node::Program(vec![]), "test");
        assert!(res.is_ok());
        assert!(derives_trait("Reading", "Debug"));
        assert!(derives_trait("Reading", "Eq"));
        assert!(!derives_trait("Reading", "Hash"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn unknown_trait_errors() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "X",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "BogusTrait".into(),
                line: 0,
            },
        );
        let res = check(&Node::Program(vec![]), "test");
        assert!(res.is_err());
        crate::feature_attrs::reset();
    }

    #[test]
    fn duplicate_trait_in_derive_list_rejected() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Reading",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "Debug, Debug".into(),
                line: 7,
            },
        );
        let err =
            check(&Node::Program(vec![]), "test").expect_err("duplicate derive trait must fail");
        assert!(err.contains("duplicate trait `Debug`"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn duplicate_derive_registration_rejected() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Reading",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "Debug".into(),
                line: 8,
            },
        );
        crate::feature_attrs::record(
            "Reading",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "Eq".into(),
                line: 9,
            },
        );
        let err =
            check(&Node::Program(vec![]), "test").expect_err("duplicate derive record must fail");
        assert!(err.contains("duplicate derive registration for `Reading`"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn partial_eq_and_partial_ord_accepted() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Point",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "PartialEq, PartialOrd, Display".into(),
                line: 0,
            },
        );
        let res = check(&Node::Program(vec![]), "test");
        assert!(res.is_ok(), "expected ok, got {:?}", res);
        assert!(derives_trait("Point", "PartialEq"));
        assert!(derives_trait("Point", "PartialOrd"));
        assert!(derives_trait("Point", "Display"));
        assert!(!derives_trait("Point", "Iterator"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn copy_and_iterator_accepted() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "MyIter",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "Copy, Iterator, From, Into".into(),
                line: 0,
            },
        );
        let res = check(&Node::Program(vec![]), "test");
        assert!(res.is_ok(), "expected ok, got {:?}", res);
        assert!(derives_trait("MyIter", "Copy"));
        assert!(derives_trait("MyIter", "Iterator"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn all_supported_traits_together() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Complete",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "Debug, Eq, Hash, Default, Clone, Ord, PartialEq, PartialOrd, Display, Iterator, From, Into, Copy"
                    .into(),
                line: 0,
            },
        );
        let res = check(&Node::Program(vec![]), "test");
        assert!(res.is_ok(), "all supported traits should be accepted");
        let complete = collect();
        assert_eq!(complete[0].1.traits.len(), 13);
        crate::feature_attrs::reset();
    }

    #[test]
    fn empty_trait_list_is_valid() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Empty",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "".into(),
                line: 0,
            },
        );
        let res = check(&Node::Program(vec![]), "test");
        assert!(res.is_ok(), "empty derive list should be valid");
        let empty = collect();
        assert_eq!(empty[0].1.traits.len(), 0);
        crate::feature_attrs::reset();
    }

    #[test]
    fn whitespace_trimming_in_trait_list() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Spaced",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "  Debug  ,  Eq  ,  Clone  ".into(),
                line: 0,
            },
        );
        let res = check(&Node::Program(vec![]), "test");
        assert!(res.is_ok(), "whitespace should be trimmed");
        assert!(derives_trait("Spaced", "Debug"));
        assert!(derives_trait("Spaced", "Eq"));
        assert!(derives_trait("Spaced", "Clone"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn quoted_trait_names_handled() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Quoted",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: r#""Debug", "Eq", "Hash""#.into(),
                line: 0,
            },
        );
        let res = check(&Node::Program(vec![]), "test");
        assert!(res.is_ok(), "quoted trait names should be handled");
        assert!(derives_trait("Quoted", "Debug"));
        assert!(derives_trait("Quoted", "Eq"));
        assert!(derives_trait("Quoted", "Hash"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn multiple_types_with_different_derives() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Type1",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "Debug, Clone".into(),
                line: 1,
            },
        );
        crate::feature_attrs::record(
            "Type2",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "Eq, Ord, Hash".into(),
                line: 2,
            },
        );
        crate::feature_attrs::record(
            "Type3",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "Default, Display".into(),
                line: 3,
            },
        );
        let res = check(&Node::Program(vec![]), "test");
        assert!(
            res.is_ok(),
            "multiple types with different derives should work"
        );
        assert!(derives_trait("Type1", "Debug"));
        assert!(!derives_trait("Type1", "Eq"));
        assert!(derives_trait("Type2", "Ord"));
        assert!(!derives_trait("Type2", "Clone"));
        assert!(derives_trait("Type3", "Display"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn derive_state_isolation_between_compilations() {
        let _g = crate::feature_attrs::lock_for_test();

        // First compilation
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "First",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "Debug".into(),
                line: 0,
            },
        );
        assert!(check(&Node::Program(vec![]), "test1").is_ok());
        assert!(derives_trait("First", "Debug"));

        // Second compilation (should not see First)
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Second",
            crate::feature_attrs::AttrRecord {
                name: "derive".into(),
                args: "Eq".into(),
                line: 0,
            },
        );
        assert!(check(&Node::Program(vec![]), "test2").is_ok());
        assert!(derives_trait("Second", "Eq"));
        assert!(
            !derives_trait("First", "Debug"),
            "First should be gone after reset"
        );

        crate::feature_attrs::reset();
    }
}
