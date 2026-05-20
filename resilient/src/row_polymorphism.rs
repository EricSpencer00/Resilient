//! Feature 15/50 — Row Polymorphism.
//!
//! `#[row_poly(requires = "name:string level:int")]` on a function
//! declares that any caller may pass *any* struct provided it
//! contains at least the listed fields. This is structural
//! subtyping at the function-parameter granularity, no inheritance
//! or interface declaration required.
//!
//! This first slice records the row constraint per function and
//! offers a `validate(fn_name, struct_fields)` query that the
//! typechecker / runtime can consult.
//!
//! The `check()` entry point now performs compile-time validation:
//! for every `CallExpression` to a row-poly function whose argument
//! is a `StructLiteral`, it looks up the declared struct fields and
//! verifies that all required fields are present with the right types.
//! Arguments that are not struct literals are skipped (their types
//! are not available at the AST level without full type inference).

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// RES-2398: dropped the redundant `fn_name: String` field. The two
/// readers in this module used it strictly as the attribute key
/// (HashMap lookup in `install`, borrowed-map construction in
/// `check`). Pipeline now carries `(String, RowSpec)` tuples —
/// matches wcet (RES-2190), prob (RES-2170), power (RES-2386), stack
/// (RES-2388), phantom (RES-2390), dependent (RES-2392), mmio_regmap
/// (RES-2394).
#[derive(Debug, Clone)]
pub struct RowSpec {
    /// Required (field_name, type_name) pairs.
    pub required: Vec<(String, String)>,
}

static SPECS: LazyLock<RwLock<HashMap<String, RowSpec>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn collect() -> Vec<(String, RowSpec)> {
    let attrs = crate::feature_attrs::find_kind("row_poly");
    // RES-1754: pre-size to attrs.len() — exactly one push per
    // attribute record.
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let mut spec = RowSpec {
            required: Vec::new(),
        };
        if let Some(rest) = rec.args.split_once('=').map(|(_, r)| r) {
            let v = rest.trim().trim_matches('"');
            for chunk in v.split_whitespace() {
                if let Some((name, ty)) = chunk.split_once(':') {
                    spec.required.push((name.to_string(), ty.to_string()));
                }
            }
        }
        out.push((item, spec));
    }
    out
}

pub fn install(specs: Vec<(String, RowSpec)>) {
    if let Ok(mut g) = SPECS.write() {
        g.clear();
        // RES-2398: move (name, spec) tuples straight from collect()
        // — no per-spec clone for the key.
        g.extend(specs);
    }
}

pub fn validate(fn_name: &str, fields: &[(String, String)]) -> Result<(), String> {
    // RES-1558: hold the read guard so the `HashMap<String, RowSpec>`
    // (each spec owns a `Vec<(String, String)>`) doesn't get cloned
    // just to look up one fn by name. Same lock-then-borrow shape as
    // RES-1544 / RES-1547 / RES-1549 / RES-1552.
    let Ok(g) = SPECS.read() else {
        return Ok(());
    };
    let Some(spec) = g.get(fn_name) else {
        return Ok(());
    };
    for (req_name, req_ty) in &spec.required {
        let found = fields.iter().any(|(n, t)| n == req_name && t == req_ty);
        if !found {
            return Err(format!(
                "row-poly violation: fn `{fn_name}` requires field `{req_name}: {req_ty}`"
            ));
        }
    }
    Ok(())
}

/// Collect struct field declarations from the program into a lookup map.
/// Map key is struct name; value is Vec<(field_name, type_name)>.
///
/// `StructDecl.fields` is `Vec<(type_name, field_name)>` — we flip
/// the tuple so callers get `(field_name, type_name)` which matches
/// the row-poly spec's `required` format.
fn collect_struct_decls(node: &Node, out: &mut HashMap<String, Vec<(String, String)>>) {
    match node {
        Node::StructDecl { name, fields, .. } => {
            let typed_fields: Vec<(String, String)> = fields
                .iter()
                .map(|(ty, fname)| (fname.clone(), ty.clone()))
                .collect();
            out.insert(name.clone(), typed_fields);
        }
        Node::Program(stmts) => {
            for s in stmts {
                collect_struct_decls(&s.node, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_struct_decls(s, out);
            }
        }
        Node::Function { body, .. } => collect_struct_decls(body, out),
        _ => {}
    }
}

/// Walk the AST and validate every `CallExpression` to a row-poly
/// function whose first argument is a `StructLiteral`.
///
/// Arguments that are not struct literals are skipped — their field
/// types are not known without full type inference.
fn walk_calls(
    node: &Node,
    specs: &HashMap<&str, &RowSpec>,
    struct_fields: &HashMap<String, Vec<(String, String)>>,
    source_path: &str,
) -> Result<(), String> {
    match node {
        Node::CallExpression {
            function,
            arguments,
            span,
        } => {
            if let Node::Identifier { name: fn_name, .. } = function.as_ref() {
                if let Some(spec) = specs.get(fn_name.as_str()) {
                    for arg in arguments.iter() {
                        if let Node::StructLiteral {
                            name: struct_name, ..
                        } = arg
                        {
                            if let Some(fields) = struct_fields.get(struct_name.as_str()) {
                                for (req_name, req_ty) in &spec.required {
                                    let found =
                                        fields.iter().any(|(n, t)| n == req_name && t == req_ty);
                                    if !found {
                                        let line = span.start.line;
                                        let col = span.start.column;
                                        let loc = if line > 0 {
                                            format!("{}:{}:{}: ", source_path, line, col)
                                        } else {
                                            format!("{}: ", source_path)
                                        };
                                        return Err(format!(
                                            "{}error: row-poly violation: \
                                             fn `{}` requires field `{}: {}`",
                                            loc, fn_name, req_name, req_ty
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            walk_calls(function, specs, struct_fields, source_path)?;
            for a in arguments {
                walk_calls(a, specs, struct_fields, source_path)?;
            }
        }
        Node::Program(stmts) => {
            for s in stmts {
                walk_calls(&s.node, specs, struct_fields, source_path)?;
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_calls(s, specs, struct_fields, source_path)?;
            }
        }
        Node::Function { body, .. } => {
            walk_calls(body, specs, struct_fields, source_path)?;
        }
        Node::LetStatement { value, .. } | Node::Assignment { value, .. } => {
            walk_calls(value, specs, struct_fields, source_path)?;
        }
        Node::ReturnStatement { value: Some(e), .. } => {
            walk_calls(e, specs, struct_fields, source_path)?;
        }
        Node::ExpressionStatement { expr, .. } => {
            walk_calls(expr, specs, struct_fields, source_path)?;
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_calls(condition, specs, struct_fields, source_path)?;
            walk_calls(consequence, specs, struct_fields, source_path)?;
            if let Some(alt) = alternative {
                walk_calls(alt, specs, struct_fields, source_path)?;
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            walk_calls(condition, specs, struct_fields, source_path)?;
            walk_calls(body, specs, struct_fields, source_path)?;
        }
        Node::ForInStatement { body, iterable, .. } => {
            walk_calls(iterable, specs, struct_fields, source_path)?;
            walk_calls(body, specs, struct_fields, source_path)?;
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    // RES-1308: gate on non-empty — avoids wasted work when no
    // row_poly attributes are in the program.
    let specs = collect();
    if specs.is_empty() {
        return Ok(());
    }

    // Pass 1: collect all struct declarations so we know each struct's
    // field types when we encounter a StructLiteral at a call site.
    let mut struct_fields: HashMap<String, Vec<(String, String)>> = HashMap::new();
    collect_struct_decls(program, &mut struct_fields);

    // RES-1998: borrow into local `specs` for the walk map instead of
    // cloning every RowSpec (which carries a Vec<(String, String)>).
    // The borrowed view lives only during `walk_calls`; once that
    // returns, ownership of `specs` is handed to `install`. Same
    // shape as RES-1996 (refinement_types).
    let result = {
        let specs_map: HashMap<&str, &RowSpec> =
            specs.iter().map(|(name, s)| (name.as_str(), s)).collect();
        walk_calls(program, &specs_map, &struct_fields, source_path)
    };
    install(specs);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_minimum_field_set() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "log",
            crate::feature_attrs::AttrRecord {
                name: "row_poly".into(),
                args: r#"requires = "name:string level:int""#.into(),
                line: 0,
            },
        );
        install(collect());
        let ok_fields = vec![
            ("name".to_string(), "string".to_string()),
            ("level".to_string(), "int".to_string()),
            ("ts".to_string(), "int".to_string()),
        ];
        assert!(validate("log", &ok_fields).is_ok());
        let bad = vec![("name".to_string(), "string".to_string())];
        assert!(validate("log", &bad).is_err());
        crate::feature_attrs::reset();
    }

    #[test]
    fn validate_unknown_fn_returns_ok() {
        assert!(validate("totally_unregistered_fn", &[]).is_ok());
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

    /// A call to a row-poly function passing a struct literal that
    /// satisfies all required fields compiles cleanly.
    #[test]
    fn check_passes_when_struct_has_all_required_fields() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "log_event",
            crate::feature_attrs::AttrRecord {
                name: "row_poly".into(),
                args: r#"requires = "name:string level:int""#.into(),
                line: 0,
            },
        );
        // Struct literals in Resilient use `new StructName { field: expr }`.
        let src = r#"
struct LogRecord { string name, int level, int ts }
fn log_event(LogRecord r) { println(r.name); }
fn main() {
    let r = new LogRecord { name: "boot", level: 1, ts: 0 };
    log_event(r);
}
"#;
        let (prog, _) = crate::parse(src);
        let result = check(&prog, "test");
        assert!(
            result.is_ok(),
            "expected OK for struct with all required fields; got: {:?}",
            result
        );
        crate::feature_attrs::reset();
    }

    /// A call to a row-poly function passing a struct literal that is
    /// MISSING a required field is a compile error.
    #[test]
    fn check_errors_when_struct_missing_required_field() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "log_event",
            crate::feature_attrs::AttrRecord {
                name: "row_poly".into(),
                args: r#"requires = "name:string level:int""#.into(),
                line: 0,
            },
        );
        // `Minimal` has `name` but NOT `level: int`. Passing a struct
        // literal directly to `log_event` (which requires `level: int`)
        // must be caught as an error. (Variable arguments are skipped —
        // their types are not recoverable without full type inference.)
        let src = r#"
struct Minimal { string name }
fn log_event(Minimal r) { println(r.name); }
fn main() {
    log_event(new Minimal { name: "boot" });
}
"#;
        let (prog, _) = crate::parse(src);
        let result = check(&prog, "test");
        assert!(
            result.is_err(),
            "expected error for struct missing required field"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("row-poly violation"),
            "error must say row-poly violation: {msg}"
        );
        assert!(
            msg.contains("level"),
            "error must name the missing field: {msg}"
        );
        crate::feature_attrs::reset();
    }

    /// Calling a row-poly function with a literal struct directly
    /// (not via a variable) that has the required fields passes.
    #[test]
    fn check_passes_for_inline_struct_literal_with_required_fields() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "emit",
            crate::feature_attrs::AttrRecord {
                name: "row_poly".into(),
                args: r#"requires = "code:int""#.into(),
                line: 0,
            },
        );
        let src = r#"
struct Event { int code, string msg }
fn emit(Event e) { println(e.code); }
fn main() {
    emit(new Event { code: 42, msg: "ok" });
}
"#;
        let (prog, _) = crate::parse(src);
        let result = check(&prog, "test");
        assert!(
            result.is_ok(),
            "expected OK for inline struct literal with required field; got: {:?}",
            result
        );
        crate::feature_attrs::reset();
    }
}
