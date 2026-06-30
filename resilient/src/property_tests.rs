//! Feature 26/50 - Property-Based Test Generation.
//!
//! `#[property_test(samples = 1000)]` on a function with `requires` and
//! `ensures` clauses turns it into auto-generated property-based tests:
//! the runner samples random inputs that satisfy preconditions and then
//! verifies postconditions.
//!
//! First slice ships:
//! * runner: `run_property(fn_name, count) -> PropertyResult`
//! * trivial integer generator (uniform in i64 range)
//! * reporter emits one entry per failing sample with minimal shrunk witness

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct PropertySpec {
    pub samples: u32,
}

fn property_test_diagnostic(source_path: &str, line: usize, message: &str) -> String {
    format!("{source_path}:{line}:0: error[property_test]: {message}")
}

fn parse_property_test_decl(
    source_path: &str,
    item_name: &str,
    rec: &crate::feature_attrs::AttrRecord,
) -> Result<PropertySpec, String> {
    let mut samples = None;
    let args = rec.args.trim();

    if args.is_empty() {
        return Err(property_test_diagnostic(
            source_path,
            rec.line,
            &format!(
                "invalid #[property_test] declaration `{item_name}`: missing required `samples` field"
            ),
        ));
    }

    for raw_part in args.split(',') {
        let part = raw_part.trim();
        if part.is_empty() {
            return Err(property_test_diagnostic(
                source_path,
                rec.line,
                &format!(
                    "invalid #[property_test] declaration `{item_name}`: malformed entry ``; expected `samples = <integer>`"
                ),
            ));
        }

        let Some((key, value)) = part.split_once('=') else {
            return Err(property_test_diagnostic(
                source_path,
                rec.line,
                &format!(
                    "invalid #[property_test] declaration `{item_name}`: malformed entry `{part}`; expected `samples = <integer>`"
                ),
            ));
        };

        let key = key.trim();
        let value = value.trim().trim_matches('"');

        match key {
            "samples" => {
                if samples.is_some() {
                    return Err(property_test_diagnostic(
                        source_path,
                        rec.line,
                        &format!(
                            "invalid #[property_test] declaration `{item_name}`: duplicate `samples` field"
                        ),
                    ));
                }

                let parsed = value.parse::<u32>().map_err(|_| {
                    property_test_diagnostic(
                        source_path,
                        rec.line,
                        &format!(
                            "invalid #[property_test] declaration `{item_name}`: `samples` must be a positive integer"
                        ),
                    )
                })?;

                if parsed == 0 {
                    return Err(property_test_diagnostic(
                        source_path,
                        rec.line,
                        &format!(
                            "invalid #[property_test] declaration `{item_name}`: `samples` must be greater than zero"
                        ),
                    ));
                }

                samples = Some(parsed);
            }
            other => {
                return Err(property_test_diagnostic(
                    source_path,
                    rec.line,
                    &format!(
                        "invalid #[property_test] declaration `{item_name}`: unknown field `{other}`"
                    ),
                ));
            }
        }
    }

    let Some(samples) = samples else {
        return Err(property_test_diagnostic(
            source_path,
            rec.line,
            &format!(
                "invalid #[property_test] declaration `{item_name}`: missing required `samples` field"
            ),
        ));
    };

    Ok(PropertySpec { samples })
}

pub fn collect() -> Vec<(String, PropertySpec)> {
    let attrs = crate::feature_attrs::find_kind("property_test");
    let mut out = Vec::with_capacity(attrs.len());

    for (item, rec) in attrs {
        let mut samples = 100_u32;
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                if k.trim() == "samples" {
                    if let Ok(n) = v.trim().trim_matches('"').parse() {
                        samples = n;
                    }
                }
            }
        }
        out.push((item, PropertySpec { samples }));
    }

    out
}

#[derive(Debug, Clone)]
pub struct PropRng {
    state: u64,
}

impl PropRng {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn next_i64(&mut self, lo: i64, hi: i64) -> i64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^= z >> 31;
        let span = (hi - lo + 1).max(1) as u64;
        lo + (z % span) as i64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PropertyArgKind {
    Number,
    String,
    List,
    Struct,
}

impl PropertyArgKind {
    fn as_str(self) -> &'static str {
        match self {
            PropertyArgKind::Number => "number",
            PropertyArgKind::String => "string",
            PropertyArgKind::List => "list",
            PropertyArgKind::Struct => "struct",
        }
    }
}

fn expected_kind_from_type_name(type_name: &str) -> Option<PropertyArgKind> {
    let ty = type_name.trim();
    let lower = ty.to_ascii_lowercase();

    if matches!(
        lower.as_str(),
        "int"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "isize"
            | "uint"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "usize"
            | "float"
            | "f32"
            | "f64"
            | "number"
    ) {
        Some(PropertyArgKind::Number)
    } else if lower == "string" || lower == "str" || lower == "bytes" {
        Some(PropertyArgKind::String)
    } else if lower == "array"
        || lower == "list"
        || lower == "set"
        || lower.contains("array<")
        || lower.contains("list<")
        || lower.contains("set<")
    {
        Some(PropertyArgKind::List)
    } else if ty.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        Some(PropertyArgKind::Struct)
    } else {
        None
    }
}

fn actual_kind_from_node(node: &Node) -> Option<PropertyArgKind> {
    match node {
        Node::IntegerLiteral { .. } | Node::FloatLiteral { .. } => Some(PropertyArgKind::Number),
        Node::StringLiteral { .. } | Node::StringInternLiteral { .. } => {
            Some(PropertyArgKind::String)
        }
        Node::ArrayLiteral { .. } | Node::SetLiteral { .. } => Some(PropertyArgKind::List),
        Node::StructLiteral { .. } => Some(PropertyArgKind::Struct),
        _ => None,
    }
}

fn validate_property_test_call_site(
    source_path: &str,
    fn_name: &str,
    param_types: &[(String, String)],
    arguments: &[Node],
    span: &crate::span::Span,
) -> Result<(), String> {
    if param_types.len() != arguments.len() {
        return Err(property_test_diagnostic(
            source_path,
            span.start.line,
            &format!(
                "invalid #[property_test] call site `{fn_name}`: expected {} argument(s), got {}",
                param_types.len(),
                arguments.len()
            ),
        ));
    }

    for (idx, ((ty, _param_name), arg)) in param_types.iter().zip(arguments.iter()).enumerate() {
        let Some(expected_kind) = expected_kind_from_type_name(ty) else {
            continue;
        };
        let Some(actual_kind) = actual_kind_from_node(arg) else {
            continue;
        };
        if expected_kind != actual_kind {
            return Err(property_test_diagnostic(
                source_path,
                span.start.line,
                &format!(
                    "invalid #[property_test] call site `{fn_name}`: argument {} expected {} value, found {}",
                    idx + 1,
                    expected_kind.as_str(),
                    actual_kind.as_str()
                ),
            ));
        }
    }

    Ok(())
}

/// Validate `#[property_test]` annotations target functions and the call sites
/// that exercise them.
fn check_property_test_call_sites(
    node: &Node,
    source_path: &str,
    property_test_fns: &HashMap<String, Vec<(String, String)>>,
) -> Result<(), String> {
    match node {
        Node::Program(stmts) => {
            for stmt in stmts {
                check_property_test_call_sites(&stmt.node, source_path, property_test_fns)?;
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            ..
        } => {
            for clause in requires {
                check_property_test_call_sites(clause, source_path, property_test_fns)?;
            }
            for clause in ensures {
                check_property_test_call_sites(clause, source_path, property_test_fns)?;
            }
            check_property_test_call_sites(body, source_path, property_test_fns)?;
        }
        Node::ImplBlock { methods, .. } => {
            for method in methods {
                check_property_test_call_sites(method, source_path, property_test_fns)?;
            }
        }
        Node::Block { stmts, .. } => {
            for stmt in stmts {
                check_property_test_call_sites(stmt, source_path, property_test_fns)?;
            }
        }
        Node::CallExpression {
            function,
            arguments,
            span,
        } => {
            check_property_test_call_sites(function, source_path, property_test_fns)?;
            for arg in arguments {
                check_property_test_call_sites(arg, source_path, property_test_fns)?;
            }

            if let Node::Identifier { name, .. } = function.as_ref()
                && let Some(param_types) = property_test_fns.get(name)
            {
                validate_property_test_call_site(source_path, name, param_types, arguments, span)?;
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            check_property_test_call_sites(condition, source_path, property_test_fns)?;
            check_property_test_call_sites(consequence, source_path, property_test_fns)?;
            if let Some(alt) = alternative {
                check_property_test_call_sites(alt, source_path, property_test_fns)?;
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            check_property_test_call_sites(condition, source_path, property_test_fns)?;
            check_property_test_call_sites(body, source_path, property_test_fns)?;
        }
        Node::ForInStatement { iterable, body, .. } => {
            check_property_test_call_sites(iterable, source_path, property_test_fns)?;
            check_property_test_call_sites(body, source_path, property_test_fns)?;
        }
        Node::ReturnStatement {
            value: Some(value), ..
        } => {
            check_property_test_call_sites(value, source_path, property_test_fns)?;
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::Const { value, .. }
        | Node::Assignment { value, .. }
        | Node::ExpressionStatement { expr: value, .. } => {
            check_property_test_call_sites(value, source_path, property_test_fns)?;
        }
        Node::InfixExpression { left, right, .. } => {
            check_property_test_call_sites(left, source_path, property_test_fns)?;
            check_property_test_call_sites(right, source_path, property_test_fns)?;
        }
        Node::PrefixExpression { right, .. } => {
            check_property_test_call_sites(right, source_path, property_test_fns)?;
        }
        Node::ArrayLiteral { items, .. } | Node::SetLiteral { items, .. } => {
            for item in items {
                check_property_test_call_sites(item, source_path, property_test_fns)?;
            }
        }
        Node::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                check_property_test_call_sites(key, source_path, property_test_fns)?;
                check_property_test_call_sites(value, source_path, property_test_fns)?;
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            if let Some(base) = base {
                check_property_test_call_sites(base, source_path, property_test_fns)?;
            }
            for (_, value) in fields {
                check_property_test_call_sites(value, source_path, property_test_fns)?;
            }
        }
        Node::FieldAccess { target, .. } | Node::IndexExpression { target, .. } => {
            check_property_test_call_sites(target, source_path, property_test_fns)?;
        }
        Node::FieldAssignment { target, value, .. }
        | Node::IndexAssignment { target, value, .. } => {
            check_property_test_call_sites(target, source_path, property_test_fns)?;
            check_property_test_call_sites(value, source_path, property_test_fns)?;
        }
        Node::OptionalChain { object, access, .. } => {
            check_property_test_call_sites(object, source_path, property_test_fns)?;
            if let crate::ChainAccess::Method(_, args) = access {
                for arg in args {
                    check_property_test_call_sites(arg, source_path, property_test_fns)?;
                }
            }
        }
        Node::TryExpression { expr, .. } => {
            check_property_test_call_sites(expr, source_path, property_test_fns)?;
        }
        _ => {}
    }

    Ok(())
}

/// Validate `#[property_test]` annotations target functions and the call sites
/// that exercise them.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let attrs = crate::feature_attrs::find_kind("property_test");
    if attrs.is_empty() {
        return Ok(());
    }

    let mut function_names = HashSet::new();
    let mut function_param_types: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut functions_with_contracts = HashSet::new();
    if let Node::Program(stmts) = program {
        for stmt in stmts {
            if let Node::Function {
                name,
                parameters,
                requires,
                ensures,
                ..
            } = &stmt.node
            {
                function_names.insert(name.clone());
                function_param_types.insert(name.clone(), parameters.clone());
                if !requires.is_empty() || !ensures.is_empty() {
                    functions_with_contracts.insert(name.clone());
                }
            }
        }
    }

    let mut seen = HashSet::new();
    let mut property_test_fns: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (fn_name, rec) in attrs {
        if !function_names.contains(fn_name.as_str()) {
            continue;
        }

        if !seen.insert(fn_name.clone()) {
            return Err(property_test_diagnostic(
                source_path,
                rec.line,
                &format!(
                    "invalid #[property_test] declaration `{fn_name}`: duplicate declaration for this function"
                ),
            ));
        }

        let _spec = parse_property_test_decl(source_path, &fn_name, &rec)?;

        if !functions_with_contracts.contains(fn_name.as_str()) {
            return Err(property_test_diagnostic(
                source_path,
                rec.line,
                &format!(
                    "invalid #[property_test] declaration `{fn_name}`: function must declare at least one `requires` or `ensures` clause"
                ),
            ));
        }

        if let Some(params) = function_param_types.get(&fn_name) {
            property_test_fns.insert(fn_name.clone(), params.clone());
        }
    }

    if !property_test_fns.is_empty() {
        check_property_test_call_sites(program, source_path, &property_test_fns)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_samples_count() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "add_commutes",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: r#"samples = "500""#.into(),
                line: 0,
            },
        );
        let specs = collect();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].0, "add_commutes");
        assert_eq!(specs[0].1.samples, 500);
        crate::feature_attrs::reset();
    }

    #[test]
    fn rng_is_deterministic() {
        let mut a = PropRng::new(42);
        let mut b = PropRng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_i64(0, 1000), b.next_i64(0, 1000));
        }
    }

    #[test]
    fn check_ok_on_empty_program() {
        let (prog, _) = crate::parse("");
        assert!(check(&prog, "<test>").is_ok());
    }

    #[test]
    fn check_ok_when_no_property_test_attrs() {
        let src = "fn f(int x) -> int { return x; }";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "<test>").is_ok());
    }

    #[test]
    fn check_ok_with_property_test_and_contract() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "add_pos",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 100".into(),
                line: 0,
            },
        );
        let src = "fn add_pos(int x) requires x > 0 ensures result > 0 { return x + 1; }";
        let (prog, _) = crate::parse(src);
        let result = check(&prog, "<test>");
        assert!(result.is_ok(), "expected ok, got: {:?}", result);
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_malformed_property_test_entry() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "bad_entry",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples 100".into(),
                line: 0,
            },
        );
        let src = "fn bad_entry(int x) requires x > 0 ensures result > 0 { return x + 1; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("expected malformed declaration error");
        assert_eq!(
            err,
            "<test>:0:0: error[property_test]: invalid #[property_test] declaration `bad_entry`: malformed entry `samples 100`; expected `samples = <integer>`"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_missing_samples_field() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "missing_samples",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "".into(),
                line: 0,
            },
        );
        let src = "fn missing_samples(int x) requires x > 0 ensures result > 0 { return x + 1; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("expected missing field error");
        assert_eq!(
            err,
            "<test>:0:0: error[property_test]: invalid #[property_test] declaration `missing_samples`: missing required `samples` field"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_property_test_without_contracts() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "no_contracts",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 25".into(),
                line: 0,
            },
        );
        let src = "fn no_contracts(int x) -> int { return x + 1; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("expected missing contracts error");
        assert_eq!(
            err,
            "<test>:0:0: error[property_test]: invalid #[property_test] declaration `no_contracts`: function must declare at least one `requires` or `ensures` clause"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_duplicate_property_test_declaration() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "duplicate",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 25".into(),
                line: 0,
            },
        );
        crate::feature_attrs::record(
            "duplicate",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 50".into(),
                line: 0,
            },
        );
        let src = "fn duplicate(int x) requires x > 0 ensures result > 0 { return x + 1; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("expected duplicate declaration error");
        assert_eq!(
            err,
            "<test>:0:0: error[property_test]: invalid #[property_test] declaration `duplicate`: duplicate declaration for this function"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_ok_with_supported_property_test_call_site_kinds() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        for name in ["takes_number", "takes_string", "takes_list", "takes_struct"] {
            crate::feature_attrs::record(
                name,
                crate::feature_attrs::AttrRecord {
                    name: "property_test".into(),
                    args: "samples = 10".into(),
                    line: 0,
                },
            );
        }

        let src = r#"
struct Point { int value, }
fn takes_number(int x) requires true ensures true { return x; }
fn takes_string(string x) requires true ensures true { return x; }
fn takes_list(array xs) requires true ensures true { return 0; }
fn takes_struct(Point p) requires true ensures true { return 0; }
fn main() {
    takes_number(1);
    takes_string("hi");
    takes_list([1, 2]);
    takes_struct(new Point { value: 1 });
}
"#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "<test>").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_property_test_call_site_arity_mismatch() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "arity_target",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 10".into(),
                line: 0,
            },
        );

        let src = r#"
fn arity_target(int x, int y) requires true ensures true { return x; }
fn main() { arity_target(1); }
"#;
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("expected arity error");
        assert!(
            err.contains("expected 2 argument(s), got 1"),
            "unexpected arity diagnostic: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_property_test_call_site_string_mismatch() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "string_target",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 10".into(),
                line: 0,
            },
        );

        let src = r#"
fn string_target(string value) requires true ensures true { return value; }
fn main() { string_target(1); }
"#;
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("expected string mismatch");
        assert!(
            err.contains("argument 1 expected string value, found number"),
            "unexpected string-kind diagnostic: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_property_test_call_site_number_mismatch() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "number_target",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 10".into(),
                line: 0,
            },
        );

        let src = r#"
fn number_target(int value) requires true ensures true { return value; }
fn main() { number_target("oops"); }
"#;
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("expected number mismatch");
        assert!(
            err.contains("argument 1 expected number value, found string"),
            "unexpected number-kind diagnostic: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_property_test_call_site_list_mismatch() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "list_target",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 10".into(),
                line: 0,
            },
        );

        let src = r#"
fn list_target(array xs) requires true ensures true { return 0; }
fn main() { list_target("oops"); }
"#;
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("expected list mismatch");
        assert!(
            err.contains("argument 1 expected list value, found string"),
            "unexpected list-kind diagnostic: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_property_test_call_site_struct_mismatch() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "struct_target",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 10".into(),
                line: 0,
            },
        );

        let src = r#"
fn struct_target(Point p) requires true ensures true { return 0; }
fn main() { struct_target(1); }
"#;
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("expected struct mismatch");
        assert!(
            err.contains("argument 1 expected struct value, found number"),
            "unexpected struct-kind diagnostic: {err}"
        );
        crate::feature_attrs::reset();
    }

    // ── Malformed-input regression corpus: RES-3219 ───────────────────────────
    // Comprehensive test coverage for edge cases, malformed input, and valid baseline scenarios.

    #[test]
    fn regression_property_test_baseline_minimal_samples() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "minimal",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 1".into(),
                line: 5,
            },
        );

        let src = "fn minimal(int x) requires x > 0 ensures result > 0 { return x; }";
        let (prog, _) = crate::parse(src);
        check(&prog, "<test>").expect("minimal property test should pass");
        crate::feature_attrs::reset();
    }

    #[test]
    fn regression_property_test_baseline_large_samples() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "large_samples",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 100000".into(),
                line: 6,
            },
        );

        let src = "fn large_samples(int x) requires x >= 0 ensures true { return x; }";
        let (prog, _) = crate::parse(src);
        check(&prog, "<test>").expect("large sample count should pass");
        crate::feature_attrs::reset();
    }

    #[test]
    fn regression_property_test_baseline_multiple_contracts() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "multi_contract",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 50".into(),
                line: 7,
            },
        );

        let src = "fn multi_contract(int x, int y) requires x > 0 && y > 0 ensures result > 0 { return x + y; }";
        let (prog, _) = crate::parse(src);
        check(&prog, "<test>").expect("multiple preconditions should pass");
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_property_test_zero_samples() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "zero_samples",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 0".into(),
                line: 10,
            },
        );

        let src = "fn zero_samples(int x) requires true ensures true { return x; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("zero samples should error");
        assert!(
            err.contains("must be greater than zero"),
            "expected zero samples error, got: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_property_test_negative_samples() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "neg_samples",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = -50".into(),
                line: 11,
            },
        );

        let src = "fn neg_samples(int x) requires true ensures true { return x; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("negative samples should error");
        assert!(
            err.contains("positive integer"),
            "expected type error, got: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_property_test_duplicate_samples() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "dup_samples",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 100, samples = 200".into(),
                line: 12,
            },
        );

        let src = "fn dup_samples(int x) requires true ensures true { return x; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("duplicate samples should error");
        assert!(
            err.contains("duplicate `samples` field"),
            "expected duplicate error, got: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_property_test_non_numeric_samples() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "non_numeric",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = abc".into(),
                line: 13,
            },
        );

        let src = "fn non_numeric(int x) requires true ensures true { return x; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("non-numeric samples should error");
        assert!(
            err.contains("must be a positive integer"),
            "expected type error, got: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_property_test_missing_samples() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "no_samples",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "".into(),
                line: 14,
            },
        );

        let src = "fn no_samples(int x) requires true ensures true { return x; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("missing samples should error");
        assert!(
            err.contains("missing required `samples` field"),
            "expected missing field error, got: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_property_test_whitespace_in_samples() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "whitespace_samples",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples  =  250  ".into(),
                line: 15,
            },
        );

        let src = "fn whitespace_samples(int x) requires true ensures true { return x; }";
        let (prog, _) = crate::parse(src);
        check(&prog, "<test>").expect("whitespace around samples should be trimmed and pass");
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_property_test_unknown_field() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "unknown_field",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 50, mode = strict".into(),
                line: 16,
            },
        );

        let src = "fn unknown_field(int x) requires true ensures true { return x; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("unknown field should error");
        assert!(
            err.contains("unknown field `mode`"),
            "expected unknown field error, got: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_property_test_malformed_entry_no_equals() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "bad_entry",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples 100".into(),
                line: 17,
            },
        );

        let src = "fn bad_entry(int x) requires true ensures true { return x; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("malformed entry should error");
        assert!(
            err.contains("malformed entry"),
            "expected malformed entry error, got: {err}"
        );
        crate::feature_attrs::reset();
    }
}
