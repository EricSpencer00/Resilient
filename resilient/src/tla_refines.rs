//! RES-3930 Phase B1/B3 — external TLA+ refinement mappings.
//!
//! The parser owns the `@refines(spec = "...", action = "...")` syntax and
//! records validated mappings in the shared attribute registry. This module
//! checks that the records still point at named functions and that each
//! function has at most one mapping. For refined call paths, it also enforces
//! the locked contract-required rule for `extern fn`: both a `requires` and an
//! `ensures` clause must be present before the path can be modeled. TLA+ file
//! loading, action discovery, and proof checking remain later increments.

use crate::Node;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

const ATTRIBUTE: &str = "refines";

#[derive(Debug, Clone, PartialEq, Eq)]
struct RefinesMapping {
    function: String,
    spec: String,
    action: String,
    line: usize,
}

#[derive(Debug, Clone)]
struct CallSite {
    callee: String,
    line: usize,
    column: usize,
}

#[derive(Debug, Default)]
struct FunctionSummary {
    calls: Vec<CallSite>,
}

#[derive(Debug, Clone, Copy)]
struct ExternSummary {
    has_requires: bool,
    has_ensures: bool,
}

fn parse_record(
    function: String,
    record: &crate::feature_attrs::AttrRecord,
) -> Result<RefinesMapping, String> {
    let mut values = HashMap::new();
    for entry in record.args.lines() {
        let Some((key, value)) = entry.split_once('=') else {
            return Err(format!("malformed @refines registry entry `{}`", entry));
        };
        let key = key.trim();
        let value = value.trim();
        if !matches!(key, "spec" | "action") {
            return Err(format!("unknown @refines registry key `{}`", key));
        }
        if value.is_empty() {
            return Err(format!("@refines `{}` value cannot be empty", key));
        }
        if values.insert(key, value.to_string()).is_some() {
            return Err(format!("duplicate @refines registry key `{}`", key));
        }
    }

    let spec = values
        .remove("spec")
        .ok_or_else(|| "@refines registry entry is missing `spec`".to_string())?;
    let action = values
        .remove("action")
        .ok_or_else(|| "@refines registry entry is missing `action`".to_string())?;
    Ok(RefinesMapping {
        function,
        spec,
        action,
        line: record.line,
    })
}

fn diagnostic(source_path: &str, line: usize, message: &str) -> String {
    if line == 0 {
        format!("{}: {}", source_path, message)
    } else {
        format!("{}:{}:1: {}", source_path, line, message)
    }
}

/// Validate the parser-preserved mappings against the current top-level AST.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let records = crate::feature_attrs::find_kind(ATTRIBUTE);
    if records.is_empty() {
        return Ok(());
    }

    let functions: HashSet<&str> = match program {
        Node::Program(statements) => statements
            .iter()
            .filter_map(|statement| match &statement.node {
                Node::Function { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect(),
        _ => HashSet::new(),
    };

    let mut seen = HashSet::with_capacity(records.len());
    let mut mappings = Vec::with_capacity(records.len());
    for (function, record) in records {
        let mapping = parse_record(function, &record)
            .map_err(|message| diagnostic(source_path, record.line, &message))?;
        if !functions.contains(mapping.function.as_str()) {
            return Err(diagnostic(
                source_path,
                mapping.line,
                &format!(
                    "@refines target `{}` is not a top-level function",
                    mapping.function
                ),
            ));
        }
        if !seen.insert(mapping.function.clone()) {
            return Err(diagnostic(
                source_path,
                mapping.line,
                &format!(
                    "function `{}` has more than one @refines mapping",
                    mapping.function
                ),
            ));
        }
        mappings.push(mapping);
    }

    check_external_targets(source_path, &mappings)?;
    check_reachable_extern_contracts(program, source_path, &mappings)
}

fn check_external_targets(source_path: &str, mappings: &[RefinesMapping]) -> Result<(), String> {
    let source = Path::new(source_path);
    // In-memory callers (including the LSP and unit-test harness) do not have
    // a stable directory against which a relative spec can be resolved.
    // Preserve their existing semantic checks and defer file validation to
    // the file-backed compiler path.
    if source_path.starts_with('<') && source_path.ends_with('>') || !source.is_file() {
        return Ok(());
    }

    let source_dir = source
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    for mapping in mappings {
        let spec_path = source_dir.join(&mapping.spec);
        if !spec_path.is_file() {
            return Err(diagnostic(
                source_path,
                mapping.line,
                &format!(
                    "@refines spec '{}' was not found at '{}'",
                    mapping.spec,
                    spec_path.display()
                ),
            ));
        }

        let spec_source = fs::read_to_string(&spec_path).map_err(|error| {
            diagnostic(
                source_path,
                mapping.line,
                &format!(
                    "@refines spec '{}' could not be read at '{}': {}",
                    mapping.spec,
                    spec_path.display(),
                    error
                ),
            )
        })?;
        if !tla_defines_action(&spec_source, &mapping.action) {
            return Err(diagnostic(
                source_path,
                mapping.line,
                &format!(
                    "@refines spec '{}' does not define action '{}'",
                    mapping.spec, mapping.action
                ),
            ));
        }
    }
    Ok(())
}

fn tla_defines_action(source: &str, action: &str) -> bool {
    let mut in_block_comment = false;
    for line in source.lines() {
        let code = strip_tla_comments(line, &mut in_block_comment);
        let Some((left, _)) = code.split_once("==") else {
            continue;
        };
        let left = left.trim();
        let name = left
            .split_once('(')
            .map(|(name, _)| name.trim())
            .unwrap_or(left);
        if name == action {
            return true;
        }
    }
    false
}

fn strip_tla_comments(line: &str, in_block_comment: &mut bool) -> String {
    let mut output = String::new();
    let mut rest = line;
    loop {
        if *in_block_comment {
            let Some(end) = rest.find("*)") else {
                break;
            };
            *in_block_comment = false;
            rest = &rest[end + 2..];
        } else if let Some(start) = rest.find("(*") {
            output.push_str(&rest[..start]);
            *in_block_comment = true;
            rest = &rest[start + 2..];
        } else {
            if let Some(start) = rest.find("\\*") {
                output.push_str(&rest[..start]);
            } else {
                output.push_str(rest);
            }
            break;
        }
    }
    output
}

fn collect_summaries(
    program: &Node,
) -> (
    HashMap<String, FunctionSummary>,
    HashMap<String, ExternSummary>,
) {
    let mut functions = HashMap::new();
    crate::uniqueness_walk::for_each_function(program, |name, _parameters, body| {
        let mut summary = FunctionSummary::default();
        crate::uniqueness_walk::visit(body, &mut |node| {
            if let Node::CallExpression { function, span, .. } = node
                && let Node::Identifier { name: callee, .. } = function.as_ref()
            {
                summary.calls.push(CallSite {
                    callee: callee.clone(),
                    line: span.start.line,
                    column: span.start.column,
                });
            }
        });
        functions.insert(name.to_string(), summary);
    });

    let mut externs = HashMap::new();
    if let Node::Program(statements) = program {
        for statement in statements {
            if let Node::Extern { decls, .. } = &statement.node {
                for decl in decls {
                    externs.insert(
                        decl.resilient_name.clone(),
                        ExternSummary {
                            has_requires: !decl.requires.is_empty(),
                            has_ensures: !decl.ensures.is_empty(),
                        },
                    );
                }
            }
        }
    }
    (functions, externs)
}

fn check_reachable_extern_contracts(
    program: &Node,
    source_path: &str,
    mappings: &[RefinesMapping],
) -> Result<(), String> {
    let (functions, externs) = collect_summaries(program);
    for mapping in mappings {
        let mut visited = HashSet::new();
        let mut work = vec![mapping.function.clone()];
        while let Some(function_name) = work.pop() {
            if !visited.insert(function_name.clone()) {
                continue;
            }
            let Some(summary) = functions.get(&function_name) else {
                continue;
            };
            for call in &summary.calls {
                if let Some(extern_summary) = externs.get(&call.callee) {
                    if extern_summary.has_requires && extern_summary.has_ensures {
                        continue;
                    }
                    let (missing, suggestion) =
                        match (extern_summary.has_requires, extern_summary.has_ensures) {
                            (false, false) => (
                                "a `requires` and an `ensures` clause",
                                "requires true; ensures result == result;",
                            ),
                            (false, true) => ("a `requires` clause", "requires true;"),
                            (true, false) => ("an `ensures` clause", "ensures result == result;"),
                            (true, true) => continue,
                        };
                    return Err(diagnostic_at(
                        source_path,
                        call.line,
                        call.column,
                        &format!(
                            "error[refinement]: refined function `{}` reaches extern fn `{}` without {}; add `{}` so the TLA+ refinement can model its behavior",
                            mapping.function, call.callee, missing, suggestion
                        ),
                    ));
                }
                if functions.contains_key(&call.callee) {
                    work.push(call.callee.clone());
                }
            }
        }
    }
    Ok(())
}

fn diagnostic_at(source_path: &str, line: usize, column: usize, message: &str) -> String {
    if line == 0 {
        format!("{}: {}", source_path, message)
    } else {
        format!("{}:{}:{}: {}", source_path, line, column.max(1), message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static FIXTURE_ID: AtomicUsize = AtomicUsize::new(0);

    fn check_source(source: &str) -> Result<(), String> {
        check_source_at(source, "refines.rz")
    }

    fn check_source_at(source: &str, source_path: &str) -> Result<(), String> {
        let _guard = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let (program, errors) = crate::parse(source);
        assert!(errors.is_empty(), "unexpected parse errors: {errors:?}");
        let result = check(&program, source_path);
        crate::feature_attrs::reset();
        result
    }

    fn fixture(source: &str, spec: Option<&str>) -> (PathBuf, PathBuf) {
        let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("resilient-refines-{}-{}", std::process::id(), id));
        fs::create_dir(&dir).expect("temporary refinement directory should be created");
        let source_path = dir.join("program.rz");
        fs::write(&source_path, source).expect("temporary Resilient source should be written");
        if let Some(spec) = spec {
            fs::write(dir.join("counter.tla"), spec)
                .expect("temporary TLA+ spec should be written");
        }
        (dir, source_path)
    }

    fn record(args: &str) -> crate::feature_attrs::AttrRecord {
        crate::feature_attrs::AttrRecord {
            name: ATTRIBUTE.to_string(),
            args: args.to_string(),
            line: 7,
        }
    }

    #[test]
    fn parses_normalized_mapping() {
        let mapping = parse_record(
            "increment".to_string(),
            &record("spec=counter.tla\naction=Inc"),
        )
        .expect("normalized parser record should validate");
        assert_eq!(
            mapping,
            RefinesMapping {
                function: "increment".to_string(),
                spec: "counter.tla".to_string(),
                action: "Inc".to_string(),
                line: 7,
            }
        );
    }

    #[test]
    fn rejects_missing_required_mapping_key() {
        let error = parse_record("increment".to_string(), &record("spec=counter.tla"))
            .expect_err("missing action must fail");
        assert!(error.contains("missing `action`"), "{error}");
    }

    #[test]
    fn rejects_duplicate_mapping_key() {
        let error = parse_record(
            "increment".to_string(),
            &record("spec=counter.tla\nspec=other.tla\naction=Inc"),
        )
        .expect_err("duplicate keys must fail");
        assert!(error.contains("duplicate"), "{error}");
    }

    #[test]
    fn rejects_refined_path_to_uncontracted_extern() {
        let source = r#"
            @refines(spec = "counter.tla", action = "Inc")
            fn increment(int value) { return foreign_increment(value); }
            extern "counter" {
                fn foreign_increment(value: Int) -> Int;
            };
        "#;
        let error = check_source(source).expect_err("uncontracted extern must be rejected");
        assert!(error.contains("error[refinement]"), "{error}");
        assert!(error.contains("foreign_increment"), "{error}");
        assert!(
            error.contains("requires") && error.contains("ensures"),
            "{error}"
        );
        assert!(
            error.contains("requires true; ensures result == result;"),
            "{error}"
        );
        assert!(
            error.contains("refines.rz:3:"),
            "call-site diagnostic expected: {error}"
        );
    }

    #[test]
    fn rejects_uncontracted_extern_reached_through_helper() {
        let source = r#"
            @refines(spec = "counter.tla", action = "Inc")
            fn increment(int value) { return helper(value); }
            fn helper(int value) { return foreign_increment(value); }
            extern "counter" {
                fn foreign_increment(value: Int) -> Int;
            };
        "#;
        let error = check_source(source).expect_err("reachable uncontracted extern must fail");
        assert!(error.contains("foreign_increment"), "{error}");
        assert!(
            error.contains("refines.rz:4:"),
            "helper call-site expected: {error}"
        );
    }

    #[test]
    fn accepts_refined_path_to_fully_contracted_extern() {
        let source = r#"
            @refines(spec = "counter.tla", action = "Inc")
            fn increment(int value) { return foreign_increment(value); }
            extern "counter" {
                fn foreign_increment(value: Int) -> Int
                    requires value >= 0
                    ensures result >= 0;
            };
        "#;
        check_source(source).expect("fully contracted extern should be modelable");
    }

    #[test]
    fn leaves_non_refined_extern_paths_permissive() {
        let source = r#"
            fn increment(int value) { return foreign_increment(value); }
            extern "counter" {
                fn foreign_increment(value: Int) -> Int;
            };
        "#;
        check_source(source).expect("contract rule is opt-in through @refines");
    }
    #[test]
    fn validates_external_spec_and_action_relative_to_source() {
        let source = r#"
            @refines(spec = "counter.tla", action = "Inc")
            fn increment(int value) { return value; }
        "#;
        let (dir, source_path) = fixture(
            source,
            Some(
                r#"
                    ---- MODULE Counter ----
                    Inc == value' = value + 1
                    ====
                "#,
            ),
        );
        let result = check_source_at(source, source_path.to_str().expect("UTF-8 fixture path"));
        fs::remove_dir_all(dir).expect("temporary refinement directory should be removed");
        result.expect("relative spec with a declared action should pass");
    }

    #[test]
    fn reports_missing_external_spec_at_annotation() {
        let source = r#"
            @refines(spec = "counter.tla", action = "Inc")
            fn increment(int value) { return value; }
        "#;
        let (dir, source_path) = fixture(source, None);
        let error = check_source_at(source, source_path.to_str().expect("UTF-8 fixture path"))
            .expect_err("missing spec should be rejected");
        fs::remove_dir_all(dir).expect("temporary refinement directory should be removed");
        assert!(error.contains("program.rz:2:1"), "{error}");
        assert!(error.contains("counter.tla"), "{error}");
        assert!(error.contains("was not found"), "{error}");
    }

    #[test]
    fn reports_missing_external_action_at_annotation() {
        let source = r#"
            @refines(spec = "counter.tla", action = "Inc")
            fn increment(int value) { return value; }
        "#;
        let (dir, source_path) = fixture(
            source,
            Some(
                r#"
                    ---- MODULE Counter ----
                    (* Inc == this is only a comment *)
                    Init == value = 0
                    ====
                "#,
            ),
        );
        let error = check_source_at(source, source_path.to_str().expect("UTF-8 fixture path"))
            .expect_err("missing action should be rejected");
        fs::remove_dir_all(dir).expect("temporary refinement directory should be removed");
        assert!(error.contains("program.rz:2:1"), "{error}");
        assert!(error.contains("does not define action 'Inc'"), "{error}");
    }
}
