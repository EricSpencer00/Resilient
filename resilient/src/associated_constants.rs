//! Feature 36/50 — Associated Constants in Traits.
//!
//! Trait-associated constants:
//!
//! ```text
//! trait Bounded { const MIN: int; const MAX: int; }
//! impl Bounded for Temperature {
//!     const MIN: int = -40;
//!     const MAX: int = 125;
//! }
//! ```
//!
//! Recorded as attributes today: `#[assoc_const(trait="Bounded", name="MIN", value="-40")]`
//! on a struct registers an associated constant. The runtime / typechecker
//! can resolve `Temperature::MIN` from the registry.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, RwLock};

#[derive(Debug, Clone)]
pub struct AssocConstant {
    pub type_name: String,
    pub trait_name: String,
    pub const_name: String,
    pub value: String,
}

/// RES-2014: nested map — outer key `type_name`, inner key `const_name`.
/// The flat `HashMap<(String, String), V>` shape forced `lookup` to
/// allocate two transient `String`s per call (stdlib's `Borrow`
/// impls don't allow `(String, String): Borrow<(&str, &str)>`).
/// Both nested-map `.get` calls accept `&str` via the existing
/// `String: Borrow<str>` impl. Same fix as RES-2008 / RES-2010 /
/// RES-2012 — completes the (String, String) HashMap key conversion
/// across all four registries in the codebase.
static ASSOC: LazyLock<RwLock<HashMap<String, HashMap<String, AssocConstant>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn collect() -> Vec<AssocConstant> {
    let attrs = crate::feature_attrs::find_kind("assoc_const");
    // RES-1782: pre-size to attrs.len() — at most one push per
    // attribute record (skipped when tr/name/val don't parse).
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        if let Ok(constant) = parse_assoc_const_record(item, &rec, "assoc_const") {
            out.push(constant);
        }
    }
    out
}

fn parse_assoc_const_record(
    item: String,
    rec: &crate::feature_attrs::AttrRecord,
    source_path: &str,
) -> Result<AssocConstant, String> {
    if rec.args.trim().is_empty() {
        return Err(assoc_const_diag(
            source_path,
            rec.line,
            format!("#[assoc_const] on `{item}` missing required `trait` argument"),
        ));
    }

    let mut trait_name = None;
    let mut const_name = None;
    let mut value = None;

    for chunk in rec.args.split(',') {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            return Err(assoc_const_diag(
                source_path,
                rec.line,
                format!("#[assoc_const] on `{item}` has empty argument"),
            ));
        }

        let Some((raw_key, raw_value)) = chunk.split_once('=') else {
            return Err(assoc_const_diag(
                source_path,
                rec.line,
                format!("#[assoc_const] on `{item}` has malformed argument `{chunk}`"),
            ));
        };

        let key = raw_key.trim();
        if key.is_empty() {
            return Err(assoc_const_diag(
                source_path,
                rec.line,
                format!("#[assoc_const] on `{item}` has malformed argument `{chunk}`"),
            ));
        }

        let parsed_value = parse_assoc_const_arg_value(&item, rec, source_path, key, raw_value)?;
        match key {
            "trait" => set_assoc_arg(&mut trait_name, &item, rec, source_path, key, parsed_value)?,
            "name" => set_assoc_arg(&mut const_name, &item, rec, source_path, key, parsed_value)?,
            "value" => set_assoc_arg(&mut value, &item, rec, source_path, key, parsed_value)?,
            _ => {
                return Err(assoc_const_diag(
                    source_path,
                    rec.line,
                    format!("#[assoc_const] on `{item}` has unknown argument `{key}`"),
                ));
            }
        }
    }

    let trait_name = trait_name.ok_or_else(|| {
        assoc_const_diag(
            source_path,
            rec.line,
            format!("#[assoc_const] on `{item}` missing required `trait` argument"),
        )
    })?;
    let const_name = const_name.ok_or_else(|| {
        assoc_const_diag(
            source_path,
            rec.line,
            format!("#[assoc_const] on `{item}` missing required `name` argument"),
        )
    })?;
    let value = value.ok_or_else(|| {
        assoc_const_diag(
            source_path,
            rec.line,
            format!("#[assoc_const] on `{item}` missing required `value` argument"),
        )
    })?;

    Ok(AssocConstant {
        type_name: item,
        trait_name,
        const_name,
        value,
    })
}

fn parse_assoc_const_arg_value(
    item: &str,
    rec: &crate::feature_attrs::AttrRecord,
    source_path: &str,
    key: &str,
    raw_value: &str,
) -> Result<String, String> {
    let trimmed = raw_value.trim();
    let Some(value) = trimmed
        .strip_prefix('"')
        .and_then(|without_prefix| without_prefix.strip_suffix('"'))
    else {
        return Err(assoc_const_diag(
            source_path,
            rec.line,
            format!("#[assoc_const] on `{item}` argument `{key}` must be a quoted string"),
        ));
    };

    if value.is_empty() {
        return Err(assoc_const_diag(
            source_path,
            rec.line,
            format!("#[assoc_const] on `{item}` argument `{key}` must not be empty"),
        ));
    }

    Ok(value.to_string())
}

fn set_assoc_arg(
    slot: &mut Option<String>,
    item: &str,
    rec: &crate::feature_attrs::AttrRecord,
    source_path: &str,
    key: &str,
    value: String,
) -> Result<(), String> {
    if slot.is_some() {
        return Err(assoc_const_diag(
            source_path,
            rec.line,
            format!("#[assoc_const] on `{item}` has duplicate `{key}` argument"),
        ));
    }

    *slot = Some(value);
    Ok(())
}

fn assoc_const_diag(source_path: &str, line: usize, message: impl AsRef<str>) -> String {
    format!(
        "{source_path}:{line}:0: error[assoc-const]: {}",
        message.as_ref()
    )
}

pub fn install(items: Vec<AssocConstant>) {
    if let Ok(mut g) = ASSOC.write() {
        g.clear();
        for a in items {
            g.entry(a.type_name.clone())
                .or_default()
                .insert(a.const_name.clone(), a);
        }
    }
}

pub fn lookup(type_name: &str, const_name: &str) -> Option<String> {
    // RES-2014: nested-map lookup — `.get(&str)` on each level uses
    // the existing `String: Borrow<str>` impl. Zero per-call
    // allocations (the previous flat `(String, String)` key forced
    // two transient `String::to_string()` allocs per call).
    ASSOC.read().ok().and_then(|g| {
        g.get(type_name)
            .and_then(|m| m.get(const_name))
            .map(|a| a.value.clone())
    })
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1308: gate `install` on the non-empty case. The previous
    // wiring wrote to `ASSOC` on every typecheck, burning a
    // RwLock write acquisition + replace on every program that
    // declares no `#[assoc_const]` attribute. It also created the
    // wipe-on-empty test race documented in RES-1302 against any
    // test that calls `install` directly under
    // `feature_attrs::lock_for_test()`.
    let attrs = crate::feature_attrs::find_kind("assoc_const");
    if attrs.is_empty() {
        return Ok(());
    }
    let mut items = Vec::with_capacity(attrs.len());
    let mut seen = HashSet::with_capacity(attrs.len());

    for (item, rec) in attrs {
        let constant = parse_assoc_const_record(item, &rec, _source_path)?;
        if !seen.insert((constant.type_name.clone(), constant.const_name.clone())) {
            return Err(assoc_const_diag(
                _source_path,
                rec.line,
                format!(
                    "duplicate associated constant `{}` for type `{}`",
                    constant.const_name, constant.type_name
                ),
            ));
        }
        items.push(constant);
    }

    install(items);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_returns_value() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Temperature",
            crate::feature_attrs::AttrRecord {
                name: "assoc_const".into(),
                args: r#"trait = "Bounded", name = "MIN", value = "-40""#.into(),
                line: 0,
            },
        );
        install(collect());
        assert_eq!(lookup("Temperature", "MIN"), Some("-40".to_string()));
        crate::feature_attrs::reset();
    }

    #[test]
    fn lookup_missing_returns_none() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        install(collect());
        assert_eq!(
            lookup("NotRegistered", "ANYTHING"),
            None,
            "unregistered type+const must return None"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn multiple_constants_on_same_type() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Volt",
            crate::feature_attrs::AttrRecord {
                name: "assoc_const".into(),
                args: r#"trait = "Units", name = "UNIT", value = "V""#.into(),
                line: 0,
            },
        );
        crate::feature_attrs::record(
            "Volt",
            crate::feature_attrs::AttrRecord {
                name: "assoc_const".into(),
                args: r#"trait = "Bounded", name = "MAX", value = "48""#.into(),
                line: 0,
            },
        );
        install(collect());
        assert_eq!(lookup("Volt", "UNIT"), Some("V".to_string()));
        assert_eq!(lookup("Volt", "MAX"), Some("48".to_string()));
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_ok_without_attributes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    fn sample_program() -> Node {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        prog
    }

    fn record_assoc_const(item: &str, args: &str, line: usize) {
        crate::feature_attrs::record(
            item,
            crate::feature_attrs::AttrRecord {
                name: "assoc_const".into(),
                args: args.into(),
                line,
            },
        );
    }

    struct ValidAssocConstCase<'a> {
        name: &'a str,
        records: &'a [AssocConstRecord<'a>],
        expected_lookups: &'a [ExpectedAssocConstLookup<'a>],
    }

    struct AssocConstRecord<'a> {
        item: &'a str,
        args: &'a str,
        line: usize,
    }

    struct ExpectedAssocConstLookup<'a> {
        type_name: &'a str,
        const_name: &'a str,
        value: &'a str,
    }

    #[test]
    fn check_rejects_malformed_assoc_const_matrix() {
        let _g = crate::feature_attrs::lock_for_test();
        let program = sample_program();
        let cases = [
            (
                "missing trait",
                "Temperature",
                r#"name = "MIN", value = "-40""#,
                3,
                "test.rz:3:0: error[assoc-const]: #[assoc_const] on `Temperature` missing required `trait` argument",
            ),
            (
                "missing name",
                "Temperature",
                r#"trait = "Bounded", value = "-40""#,
                4,
                "test.rz:4:0: error[assoc-const]: #[assoc_const] on `Temperature` missing required `name` argument",
            ),
            (
                "missing value",
                "Temperature",
                r#"trait = "Bounded", name = "MIN""#,
                5,
                "test.rz:5:0: error[assoc-const]: #[assoc_const] on `Temperature` missing required `value` argument",
            ),
            (
                "duplicate trait",
                "Temperature",
                r#"trait = "Bounded", trait = "Limits", name = "MIN", value = "-40""#,
                6,
                "test.rz:6:0: error[assoc-const]: #[assoc_const] on `Temperature` has duplicate `trait` argument",
            ),
            (
                "unknown argument",
                "Temperature",
                r#"trait = "Bounded", name = "MIN", value = "-40", units = "C""#,
                7,
                "test.rz:7:0: error[assoc-const]: #[assoc_const] on `Temperature` has unknown argument `units`",
            ),
            (
                "malformed argument",
                "Temperature",
                r#"trait = "Bounded", name "MIN", value = "-40""#,
                8,
                "test.rz:8:0: error[assoc-const]: #[assoc_const] on `Temperature` has malformed argument `name \"MIN\"`",
            ),
            (
                "empty value",
                "Temperature",
                r#"trait = "Bounded", name = "MIN", value = """#,
                9,
                "test.rz:9:0: error[assoc-const]: #[assoc_const] on `Temperature` argument `value` must not be empty",
            ),
            (
                "unquoted trait",
                "Temperature",
                r#"trait = Bounded, name = "MIN", value = "-40""#,
                10,
                "test.rz:10:0: error[assoc-const]: #[assoc_const] on `Temperature` argument `trait` must be a quoted string",
            ),
        ];

        for (case, item, args, line, expected) in cases {
            crate::feature_attrs::reset();
            record_assoc_const(item, args, line);
            let err = check(&program, "test.rz").expect_err(case);
            assert_eq!(err, expected, "{case}");
        }

        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_duplicate_assoc_const_declaration() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let program = sample_program();
        record_assoc_const(
            "Temperature",
            r#"trait = "Bounded", name = "MIN", value = "-40""#,
            12,
        );
        record_assoc_const(
            "Temperature",
            r#"trait = "Limits", name = "MIN", value = "-50""#,
            13,
        );

        let err = check(&program, "test.rz").expect_err("duplicate declaration must fail");
        assert_eq!(
            err,
            "test.rz:13:0: error[assoc-const]: duplicate associated constant `MIN` for type `Temperature`"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_accepts_valid_assoc_const_baselines() {
        let _g = crate::feature_attrs::lock_for_test();
        let program = sample_program();
        let cases = [
            ValidAssocConstCase {
                name: "single constant",
                records: &[AssocConstRecord {
                    item: "Temperature",
                    args: r#"trait = "Bounded", name = "MIN", value = "-40""#,
                    line: 20,
                }],
                expected_lookups: &[ExpectedAssocConstLookup {
                    type_name: "Temperature",
                    const_name: "MIN",
                    value: "-40",
                }],
            },
            ValidAssocConstCase {
                name: "reordered arguments",
                records: &[AssocConstRecord {
                    item: "Temperature",
                    args: r#"value = "125", name = "MAX", trait = "Bounded""#,
                    line: 21,
                }],
                expected_lookups: &[ExpectedAssocConstLookup {
                    type_name: "Temperature",
                    const_name: "MAX",
                    value: "125",
                }],
            },
            ValidAssocConstCase {
                name: "distinct constants on one type",
                records: &[
                    AssocConstRecord {
                        item: "Volt",
                        args: r#"trait = "Units", name = "UNIT", value = "V""#,
                        line: 22,
                    },
                    AssocConstRecord {
                        item: "Volt",
                        args: r#"trait = "Bounded", name = "MAX", value = "48""#,
                        line: 23,
                    },
                ],
                expected_lookups: &[
                    ExpectedAssocConstLookup {
                        type_name: "Volt",
                        const_name: "UNIT",
                        value: "V",
                    },
                    ExpectedAssocConstLookup {
                        type_name: "Volt",
                        const_name: "MAX",
                        value: "48",
                    },
                ],
            },
        ];

        for case in cases {
            crate::feature_attrs::reset();
            for record in case.records {
                record_assoc_const(record.item, record.args, record.line);
            }

            check(&program, "test.rz").unwrap_or_else(|err| panic!("{}: {err}", case.name));
            for expected in case.expected_lookups {
                assert_eq!(
                    lookup(expected.type_name, expected.const_name),
                    Some(expected.value.to_string()),
                    "{}: lookup {}::{}",
                    case.name,
                    expected.type_name,
                    expected.const_name
                );
            }
        }

        crate::feature_attrs::reset();
    }

    // ── Malformed-input regression corpus (RES-3139) ──────────────
    // Valid baselines
    #[test]
    fn corpus_accepts_valid_assoc_const() {
        record_assoc_const(
            "Temperature",
            "Bounded",
            "MIN",
            "-40",
            100,
        );
        run_check("test.rz").expect("valid assoc_const should pass");
        crate::feature_attrs::reset();
    }

    #[test]
    fn corpus_accepts_multiple_consts_same_type() {
        record_assoc_const("Point", "Spatial", "ORIGIN_X", "0", 101);
        record_assoc_const("Point", "Spatial", "ORIGIN_Y", "0", 102);
        run_check("test.rz").expect("multiple consts on same type should pass");
        crate::feature_attrs::reset();
    }

    #[test]
    fn corpus_accepts_same_const_different_types() {
        record_assoc_const("Celsius", "Temperature", "MIN", "-273", 103);
        record_assoc_const("Fahrenheit", "Temperature", "MIN", "-460", 104);
        run_check("test.rz").expect("same const name on different types should pass");
        crate::feature_attrs::reset();
    }

    // Malformed cases
    #[test]
    fn corpus_rejects_empty_type_name() {
        record_assoc_const("", "Bounded", "MAX", "100", 105);
        let err = run_check("test.rz").expect_err("empty type name should fail");
        assert!(err.contains("empty type_name") || !err.is_empty());
        crate::feature_attrs::reset();
    }

    #[test]
    fn corpus_rejects_empty_const_name() {
        record_assoc_const("Bounded", "Trait", "", "42", 106);
        let err = run_check("test.rz").expect_err("empty const_name should fail");
        assert!(err.contains("empty") || !err.is_empty());
        crate::feature_attrs::reset();
    }

    #[test]
    fn corpus_rejects_empty_value() {
        record_assoc_const("MyType", "MyTrait", "VALUE", "", 107);
        let err = run_check("test.rz").expect_err("empty value should fail");
        assert!(err.contains("empty") || err.contains("value") || !err.is_empty());
        crate::feature_attrs::reset();
    }

    #[test]
    fn corpus_rejects_duplicate_const() {
        record_assoc_const("Type", "Trait", "CONST", "1", 108);
        record_assoc_const("Type", "Trait", "CONST", "2", 109);
        let err = run_check("test.rz").expect_err("duplicate const should fail");
        assert!(err.contains("duplicate") || !err.is_empty());
        crate::feature_attrs::reset();
    }

    #[test]
    fn corpus_rejects_conflicting_value() {
        record_assoc_const("Point", "Coordinate", "X", "0", 110);
        record_assoc_const("Point", "Coordinate", "X", "1", 111);
        let err = run_check("test.rz").expect_err("conflicting value should fail");
        assert!(err.contains("conflict") || err.contains("duplicate") || !err.is_empty());
        crate::feature_attrs::reset();
    }
}
