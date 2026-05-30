//! RES-2601: Exhaustive struct field checking in match patterns.
//!
//! When a struct pattern in a `match` arm does NOT use `..` (has_rest=false),
//! every field declared on the struct must appear in the pattern. A missing
//! field is a compile-time error — it almost certainly means the programmer
//! forgot to handle it.
//!
//! `..` is the explicit opt-out: `Config { debug: true, .. }` silently
//! matches and ignores any remaining fields.
//!
//! The check works in two passes:
//!   1. Collect all `StructDecl` field names into a registry.
//!   2. Walk every `Pattern::Struct { has_rest: false }` and verify the
//!      pattern lists all declared fields; recurse into field sub-patterns
//!      so nested struct patterns are also checked.

use std::collections::HashMap;

use crate::{Node, Pattern};

/// Build a map from struct name → ordered field names.
fn collect_structs(program: &Node) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    let Node::Program(stmts) = program else {
        return map;
    };
    for stmt in stmts {
        collect_structs_node(&stmt.node, &mut map);
    }
    map
}

fn collect_structs_node(node: &Node, map: &mut HashMap<String, Vec<String>>) {
    match node {
        Node::StructDecl { name, fields, .. } => {
            // fields: Vec<(type_name, field_name)>
            let names: Vec<String> = fields.iter().map(|(_, fname)| fname.clone()).collect();
            map.insert(name.clone(), names);
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_structs_node(s, map);
            }
        }
        Node::Function { body, .. } => collect_structs_node(body, map),
        _ => {}
    }
}

/// Walk the AST checking every `Pattern::Struct { has_rest: false }`.
fn check_node(
    node: &Node,
    registry: &HashMap<String, Vec<String>>,
    source_path: &str,
) -> Result<(), String> {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                check_node(&s.node, registry, source_path)?;
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            check_node(scrutinee, registry, source_path)?;
            for (pattern, guard, body) in arms {
                check_pattern(pattern, registry, source_path)?;
                if let Some(g) = guard {
                    check_node(g, registry, source_path)?;
                }
                check_node(body, registry, source_path)?;
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                check_node(s, registry, source_path)?;
            }
        }
        Node::ExpressionStatement { expr, .. } => check_node(expr, registry, source_path)?,
        Node::LetStatement { value, .. } => check_node(value, registry, source_path)?,
        Node::ReturnStatement { value: Some(v), .. } => check_node(v, registry, source_path)?,
        Node::Function { body, .. } => check_node(body, registry, source_path)?,
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            check_node(condition, registry, source_path)?;
            check_node(consequence, registry, source_path)?;
            if let Some(a) = alternative {
                check_node(a, registry, source_path)?;
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            check_node(condition, registry, source_path)?;
            check_node(body, registry, source_path)?;
        }
        Node::ForInStatement { body, .. } => check_node(body, registry, source_path)?,
        Node::InfixExpression { left, right, .. } => {
            check_node(left, registry, source_path)?;
            check_node(right, registry, source_path)?;
        }
        Node::CallExpression { arguments, .. } => {
            for arg in arguments {
                check_node(arg, registry, source_path)?;
            }
        }
        Node::Assignment { value, .. } => check_node(value, registry, source_path)?,
        _ => {}
    }
    Ok(())
}

/// Recursively check a pattern, validating every struct sub-pattern.
fn check_pattern(
    pattern: &Pattern,
    registry: &HashMap<String, Vec<String>>,
    source_path: &str,
) -> Result<(), String> {
    match pattern {
        Pattern::Struct {
            struct_name,
            fields,
            has_rest,
        } => {
            if !has_rest && let Some(declared) = registry.get(struct_name) {
                let mut missing: Vec<&str> = declared
                    .iter()
                    .filter(|fname| !fields.iter().any(|(pf, _)| pf == *fname))
                    .map(|s| s.as_str())
                    .collect();
                if !missing.is_empty() {
                    missing.sort();
                    return Err(format!(
                        "{}: error: struct pattern `{}` is missing field(s): {} — \
                         add the field(s) or use `..` to ignore them",
                        source_path,
                        struct_name,
                        missing.join(", ")
                    ));
                }
            }
            // Recurse into sub-patterns.
            for (_, sub_pat) in fields {
                check_pattern(sub_pat, registry, source_path)?;
            }
        }
        Pattern::Or(branches) => {
            for b in branches {
                check_pattern(b, registry, source_path)?;
            }
        }
        Pattern::Bind(_, inner) => check_pattern(inner, registry, source_path)?,
        Pattern::Some(inner) | Pattern::Ok(inner) | Pattern::Err(inner) => {
            check_pattern(inner, registry, source_path)?
        }
        Pattern::TupleStruct { fields, .. } => {
            for f in fields {
                check_pattern(f, registry, source_path)?;
            }
        }
        Pattern::Tuple(items) => {
            for item in items {
                check_pattern(item, registry, source_path)?;
            }
        }
        Pattern::EnumVariant { payload, .. } => match payload {
            crate::EnumPatternPayload::Named(fields) => {
                for (_, sub) in fields {
                    check_pattern(sub, registry, source_path)?;
                }
            }
            crate::EnumPatternPayload::Tuple(items) => {
                for item in items {
                    check_pattern(item, registry, source_path)?;
                }
            }
            crate::EnumPatternPayload::None => {}
        },
        Pattern::Literal(_)
        | Pattern::Identifier(_)
        | Pattern::Wildcard
        | Pattern::None
        | Pattern::Range { .. } => {}
    }
    Ok(())
}

/// Entry point registered in `<EXTENSION_PASSES>`.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let registry = collect_structs(program);
    check_node(program, &registry, source_path)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::check;

    fn check_ok(src: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        assert!(check(&prog, "test.rz").is_ok());
    }

    fn check_err(src: &str) -> String {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let result = check(&prog, "test.rz");
        assert!(result.is_err(), "expected error but check succeeded");
        result.unwrap_err()
    }

    #[test]
    fn complete_struct_pattern_ok() {
        check_ok(
            r#"
struct Point { int x, int y }
fn dist(Point p) -> int {
    return match p {
        Point { x: 0, y: 0 } => 0,
        Point { x, y } => x + y,
    };
}
"#,
        );
    }

    #[test]
    fn rest_pattern_ok() {
        check_ok(
            r#"
struct Config { bool debug, bool verbose, int port }
fn mode(Config c) -> int {
    return match c {
        Config { debug: true, .. } => 1,
        Config { .. } => 0,
    };
}
"#,
        );
    }

    #[test]
    fn missing_single_field_errors() {
        let e = check_err(
            r#"
struct Config { bool debug, bool verbose }
fn mode(Config c) -> int {
    return match c {
        Config { debug: true } => 1,
        Config { debug, verbose } => 0,
    };
}
"#,
        );
        assert!(
            e.contains("missing field") || e.contains("verbose"),
            "expected missing-field error, got: {e:?}"
        );
    }

    #[test]
    fn missing_multiple_fields_errors() {
        let e = check_err(
            r#"
struct Config { bool debug, bool verbose, int port }
fn mode(Config c) -> int {
    return match c {
        Config { debug: true } => 1,
        Config { debug, verbose, port } => 0,
    };
}
"#,
        );
        assert!(
            e.contains("missing field") || e.contains("verbose") || e.contains("port"),
            "expected missing-field error, got: {e:?}"
        );
    }

    #[test]
    fn wildcard_arm_ok() {
        check_ok(
            r#"
struct Event { int code, bool critical }
fn handle(Event e) -> int {
    return match e {
        Event { code: 0, critical: false } => 0,
        Event { code, critical } => code,
    };
}
"#,
        );
    }

    #[test]
    fn single_field_struct_complete_ok() {
        check_ok(
            r#"
struct Wrap { int value }
fn unwrap(Wrap w) -> int {
    return match w {
        Wrap { value: 0 } => -1,
        Wrap { value } => value,
    };
}
"#,
        );
    }

    #[test]
    fn nested_struct_pattern_missing_field_errors() {
        let e = check_err(
            r#"
struct Inner { int a, int b }
struct Outer { Inner inner, int x }
fn process(Outer o) -> int {
    return match o {
        Outer { inner: Inner { a }, x } => a + x,
    };
}
"#,
        );
        assert!(
            e.contains("missing field") || e.contains("b"),
            "expected nested missing-field error, got: {e:?}"
        );
    }

    #[test]
    fn no_struct_decl_for_pattern_passes() {
        // When no StructDecl is present, we can't check — pass silently.
        check_ok(
            r#"
fn f(int x) -> int {
    return x;
}
"#,
        );
    }
}
