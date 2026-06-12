//! Feature 39/50 - Macros (Compile-Time Substitution).
//!
//! `#[macro(pattern = "...", expansion = "...")]` declares a simple
//! syntactic macro: when the parser sees a call to the macro's name, it
//! substitutes the expansion template with `$arg` placeholders filled in
//! from the call site.
//!
//! This is a textual macro system, not hygienic, and is intended for
//! `assert_eq!`, `format!`, and small DSLs. Hygiene and procedural macros
//! are downstream tickets.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

#[derive(Debug, Clone)]
pub struct MacroDef {
    pub name: String,
    pub pattern: String,
    pub expansion: String,
}

static MACROS: LazyLock<RwLock<HashMap<String, MacroDef>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn collect() -> Vec<MacroDef> {
    let attrs = crate::feature_attrs::find_kind("macro");
    // RES-1764: pre-size `attrs.len()` exactly one push per attribute record.
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        if let Ok(def) = parse_macro_decl(&item, &rec) {
            out.push(def);
        }
    }
    out
}

pub fn install(macros: Vec<MacroDef>) {
    if let Ok(mut g) = MACROS.write() {
        g.clear();
        for m in macros {
            g.insert(m.name.clone(), m);
        }
    }
}

pub fn expand(name: &str, args: &[String]) -> Option<String> {
    let g = MACROS.read().ok()?;
    let def = g.get(name)?;
    let mut out = def.expansion.clone();
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("${}", i + 1), a);
    }
    Some(out)
}

/// AST-level macro expansion pass.
///
/// Called by the lowering pipeline so expanded forms participate in
/// typechecking and evaluation.
///
/// `CallExpression` whose callee matches a registered `#[macro(...)]` name:
/// 1. Serialize arguments back to source strings.
/// 2. Substitute into the expansion template.
/// 3. Re-parse as a single expression with `crate::parse_single_expression`.
/// 4. Replace the node in place.
pub fn lower_program(program: &mut Node) {
    let macros_snapshot: Vec<(String, MacroDef)> = {
        let Ok(g) = MACROS.read() else { return };
        if g.is_empty() {
            // Fast path: no macros installed yet and `check()` has not run.
            drop(g);
            let defs = collect();
            if defs.is_empty() {
                return;
            }
            install(defs.clone());
            defs.into_iter().map(|d| (d.name.clone(), d)).collect()
        } else {
            g.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        }
    };

    if macros_snapshot.is_empty() {
        return;
    }

    let macro_names: std::collections::HashSet<String> =
        macros_snapshot.iter().map(|(k, _)| k.clone()).collect();
    lower_node(program, &macro_names);
}

fn lower_node(node: &mut Node, macro_names: &std::collections::HashSet<String>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            for arg in arguments.iter_mut() {
                lower_node(arg, macro_names);
            }

            if let Node::Identifier { name, .. } = function.as_ref() {
                if macro_names.contains(name) {
                    let arg_strs: Vec<String> = arguments.iter().map(node_to_source).collect();
                    if let Some(expanded) = expand(name, &arg_strs) {
                        if let Some(expanded_node) = crate::parse_single_expression(&expanded) {
                            *node = expanded_node;
                        }
                    }
                }
            }
        }
        Node::Program(items) => {
            for item in items.iter_mut() {
                lower_node(&mut item.node, macro_names);
            }
        }
        Node::Function { body, .. } => lower_node(body, macro_names),
        Node::Block { stmts, .. } => {
            for s in stmts.iter_mut() {
                lower_node(s, macro_names);
            }
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::Const { value, .. }
        | Node::Assignment { value, .. } => lower_node(value, macro_names),
        Node::ReturnStatement { value: Some(v), .. } => lower_node(v, macro_names),
        Node::ExpressionStatement { expr, .. } => lower_node(expr, macro_names),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            lower_node(condition, macro_names);
            lower_node(consequence, macro_names);
            if let Some(alt) = alternative {
                lower_node(alt, macro_names);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            lower_node(condition, macro_names);
            lower_node(body, macro_names);
        }
        Node::ForInStatement { iterable, body, .. } => {
            lower_node(iterable, macro_names);
            lower_node(body, macro_names);
        }
        Node::InfixExpression { left, right, .. } => {
            lower_node(left, macro_names);
            lower_node(right, macro_names);
        }
        Node::PrefixExpression { right, .. } => lower_node(right, macro_names),
        Node::FieldAccess { target, .. } => lower_node(target, macro_names),
        Node::FieldAssignment { target, value, .. } => {
            lower_node(target, macro_names);
            lower_node(value, macro_names);
        }
        Node::IndexExpression { target, index, .. } => {
            lower_node(target, macro_names);
            lower_node(index, macro_names);
        }
        Node::ArrayLiteral { items, .. } => {
            for i in items.iter_mut() {
                lower_node(i, macro_names);
            }
        }
        _ => {}
    }
}

/// Serialize an expression node back to a minimal source string so it can be
/// substituted into a macro expansion template.
fn node_to_source(node: &Node) -> String {
    match node {
        Node::IntegerLiteral { value, .. } => value.to_string(),
        Node::FloatLiteral { value, .. } => value.to_string(),
        Node::BooleanLiteral { value, .. } => value.to_string(),
        Node::StringLiteral { value, .. } => format!("\"{}\"", value.replace('"', "\\\"")),
        Node::Identifier { name, .. } => name.clone(),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => format!(
            "({} {} {})",
            node_to_source(left),
            operator,
            node_to_source(right)
        ),
        Node::PrefixExpression {
            operator, right, ..
        } => {
            format!("{}{}", operator, node_to_source(right))
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            let fname = node_to_source(function);
            let args: Vec<String> = arguments.iter().map(node_to_source).collect();
            format!("{}({})", fname, args.join(", "))
        }
        Node::FieldAccess { target, field, .. } => {
            format!("{}.{}", node_to_source(target), field)
        }
        _ => "__macro_arg__".to_string(),
    }
}

fn macro_diagnostic(source_path: &str, line: usize, message: &str) -> String {
    format!("{source_path}:{line}:0: error: {message}")
}

fn macro_location(source_path: &str, line: usize) -> String {
    format!("{source_path}:{line}:0")
}

fn parse_macro_decl(
    item: &str,
    rec: &crate::feature_attrs::AttrRecord,
) -> Result<MacroDef, String> {
    let mut pattern: Option<String> = None;
    let mut expansion: Option<String> = None;
    let args = rec.args.trim();

    if !args.is_empty() {
        let mut start = 0usize;
        let mut in_string = false;
        let mut escaped = false;

        for (idx, ch) in rec.args.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' if in_string => escaped = true,
                '"' => in_string = !in_string,
                ',' if !in_string => {
                    parse_macro_part(item, &rec.args[start..idx], &mut pattern, &mut expansion)?;
                    start = idx + ch.len_utf8();
                }
                _ => {}
            }
        }

        if in_string {
            return Err("unterminated quoted string".to_string());
        }

        let tail = rec.args[start..].trim();
        if !tail.is_empty() {
            parse_macro_part(item, tail, &mut pattern, &mut expansion)?;
        } else if rec.args.trim_end().ends_with(',') {
            return Err("trailing comma in declaration".to_string());
        }
    }

    let pattern = pattern.ok_or_else(|| "missing required `pattern` field".to_string())?;
    let expansion = expansion.ok_or_else(|| "missing required `expansion` field".to_string())?;

    Ok(MacroDef {
        name: item.to_string(),
        pattern,
        expansion,
    })
}

fn parse_macro_part(
    item: &str,
    part: &str,
    pattern: &mut Option<String>,
    expansion: &mut Option<String>,
) -> Result<(), String> {
    let part = part.trim();
    if part.is_empty() {
        return Err("empty declaration entry".to_string());
    }

    let (key, value) = part
        .split_once('=')
        .ok_or_else(|| format!("malformed entry `{part}`; expected `key = \"value\"`"))?;
    let key = key.trim();
    let value = value.trim();

    if key.is_empty() {
        return Err(format!("malformed entry `{part}`; missing field name"));
    }
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return Err(format!(
            "malformed entry `{part}`; expected quoted string value"
        ));
    }

    let value = unescape_macro_value(&value[1..value.len() - 1])?;
    match key {
        "pattern" => {
            if pattern.replace(value).is_some() {
                return Err("duplicate `pattern` field".to_string());
            }
        }
        "expansion" => {
            if expansion.replace(value).is_some() {
                return Err("duplicate `expansion` field".to_string());
            }
        }
        other => {
            return Err(format!(
                "unknown field `{other}` in macro declaration for `{item}`"
            ));
        }
    }

    Ok(())
}

fn unescape_macro_value(value: &str) -> Result<String, String> {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    let slash = char::from(92);

    while let Some(ch) = chars.next() {
        if ch != slash {
            out.push(ch);
            continue;
        }

        let Some(next) = chars.next() else {
            return Err("unterminated escape sequence in quoted string".to_string());
        };

        match next {
            c if c == slash => out.push(slash),
            '"' => out.push('"'),
            other => {
                out.push(slash);
                out.push(other);
            }
        }
    }

    Ok(out)
}

fn scan_placeholders(field: &str, text: &str) -> Result<Vec<usize>, String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }

        let start = i + 1;
        if start >= bytes.len() || !bytes[start].is_ascii_digit() {
            return Err(format!(
                "invalid placeholder `$` in `{field}`: expected `$N`"
            ));
        }

        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }

        let digits = &text[start..end];
        if digits.len() > 1 {
            if digits.starts_with('0') {
                return Err(format!(
                    "invalid placeholder `${digits}` in `{field}`: leading zeroes are not allowed"
                ));
            }
            return Err(format!(
                "invalid placeholder `${digits}` in `{field}`: multi-digit placeholders are not supported"
            ));
        }

        let idx = digits.parse::<usize>().unwrap();
        if idx == 0 {
            return Err(format!(
                "invalid placeholder `$0` in `{field}`: placeholder indices start at 1"
            ));
        }
        out.push(idx);
        i = end;
    }

    Ok(out)
}

fn validate_macro_decl(
    item: &str,
    rec: &crate::feature_attrs::AttrRecord,
) -> Result<MacroDef, String> {
    let def = parse_macro_decl(item, rec)?;

    let pattern_placeholders = scan_placeholders("pattern", &def.pattern)?;
    let pattern_arity = pattern_placeholders.iter().copied().max().unwrap_or(0);

    let expansion_placeholders = scan_placeholders("expansion", &def.expansion)?;
    if let Some(bad_idx) = expansion_placeholders
        .iter()
        .copied()
        .find(|idx| *idx > pattern_arity)
    {
        return Err(format!(
            "expansion references placeholder `${bad_idx}` but `pattern` only declares `$1..${pattern_arity}`"
        ));
    }

    let parsed_expansion = if pattern_arity == 0 {
        def.expansion.clone()
    } else {
        let mut expanded = def.expansion.clone();
        for idx in 1..=pattern_arity {
            expanded = expanded.replace(&format!("${idx}"), &format!("__macro_arg_{idx}__"));
        }
        expanded
    };

    if crate::parse_single_expression(&parsed_expansion).is_none() {
        return Err("expansion does not parse after placeholder substitution".to_string());
    }

    Ok(def)
}

pub(crate) fn check(_program: &Node, source_path: &str) -> Result<(), String> {
    let attrs = crate::feature_attrs::find_kind("macro");
    let mut macros: Vec<(usize, MacroDef)> = Vec::with_capacity(attrs.len());
    let mut seen: HashMap<String, usize> = HashMap::with_capacity(attrs.len());

    for (item, rec) in attrs {
        let macro_def = validate_macro_decl(&item, &rec).map_err(|msg| {
            macro_diagnostic(
                source_path,
                rec.line,
                &format!("invalid #[macro] declaration for `{item}`: {msg}"),
            )
        })?;

        if let Some(&prev_idx) = seen.get(&item) {
            let (prev_line, prev_def) = &macros[prev_idx];
            let kind = if prev_def.pattern == macro_def.pattern
                && prev_def.expansion == macro_def.expansion
            {
                "duplicate"
            } else {
                "conflicting"
            };
            let prev_loc = macro_location(source_path, *prev_line);
            let current_loc = macro_location(source_path, rec.line);
            return Err(format!(
                "{current_loc}: error: {kind} #[macro] declaration `{item}`; first declared at {prev_loc}, second declared at {current_loc}"
            ));
        }

        seen.insert(item.clone(), macros.len());
        macros.push((rec.line, macro_def));
    }

    if macros.is_empty() {
        return Ok(());
    }

    install(macros.into_iter().map(|(_, def)| def).collect());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_macro_decl(args: &str) -> Result<(), String> {
        check_macro_decls(&[("macro_target", 0, args)])
    }

    fn check_macro_decls(decls: &[(&str, usize, &str)]) -> Result<(), String> {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        if let Ok(mut g) = MACROS.write() {
            g.clear();
        }

        for (item_name, line, args) in decls {
            crate::feature_attrs::record(
                item_name,
                crate::feature_attrs::AttrRecord {
                    name: "macro".into(),
                    args: (*args).into(),
                    line: *line,
                },
            );
        }

        let program = Node::Program(vec![]);
        let result = check(&program, "test.rz");

        if let Ok(mut g) = MACROS.write() {
            g.clear();
        }
        crate::feature_attrs::reset();
        result
    }

    fn check_macro_decl_err(args: &str) -> String {
        check_macro_decl(args).unwrap_err()
    }

    #[test]
    fn expands_assert_eq() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "assert_eq",
            crate::feature_attrs::AttrRecord {
                name: "macro".into(),
                args: r#"pattern = "$1, $2", expansion = "if $1 != $2 { panic(\"not equal\") }""#
                    .into(),
                line: 0,
            },
        );
        install(collect());
        let exp = expand("assert_eq", &["x".into(), "5".into()]).unwrap();
        assert!(exp.contains("if x != 5"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn node_to_source_handles_integer() {
        let n = Node::IntegerLiteral {
            value: 42,
            span: Default::default(),
        };
        assert_eq!(node_to_source(&n), "42");
    }

    #[test]
    fn node_to_source_handles_bool() {
        let n = Node::BooleanLiteral {
            value: true,
            span: Default::default(),
        };
        assert_eq!(node_to_source(&n), "true");
    }

    #[test]
    fn node_to_source_handles_string_escaping() {
        let n = Node::StringLiteral {
            value: r#"say "hi""#.to_string(),
            span: Default::default(),
        };
        let s = node_to_source(&n);
        assert!(s.starts_with('"') && s.ends_with('"'));
        assert!(s.contains("\\\""));
    }

    #[test]
    fn node_to_source_handles_identifier() {
        let n = Node::Identifier {
            name: "foo".into(),
            span: Default::default(),
        };
        assert_eq!(node_to_source(&n), "foo");
    }

    #[test]
    fn lower_program_is_noop_when_no_macros() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        // MACROS registry.
        if let Ok(mut g) = MACROS.write() {
            g.clear();
        }
        let mut program = Node::Program(vec![]);
        lower_program(&mut program); // must not panic
        crate::feature_attrs::reset();
    }

    #[test]
    fn lower_program_expands_call_to_registered_macro() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();

        // Register trivial identity macro: `id(x)` => `x`
        install(vec![MacroDef {
            name: "id".into(),
            pattern: "$1".into(),
            expansion: "$1".into(),
        }]);

        // Build minimal program: `id(99)`
        let call = Node::CallExpression {
            function: Box::new(Node::Identifier {
                name: "id".into(),
                span: Default::default(),
            }),
            arguments: vec![Node::IntegerLiteral {
                value: 99,
                span: Default::default(),
            }],
            span: Default::default(),
        };
        let mut program = Node::Program(vec![crate::Spanned {
            node: Node::ExpressionStatement {
                expr: Box::new(call),
                span: Default::default(),
            },
            span: Default::default(),
        }]);

        lower_program(&mut program);

        // After lowering, ExpressionStatement's expr should be
        // IntegerLiteral(99), not CallExpression.
        if let Node::Program(stmts) = &program {
            if let Node::ExpressionStatement { expr, .. } = &stmts[0].node {
                assert!(
                    matches!(expr.as_ref(), Node::IntegerLiteral { value: 99, .. }),
                    "expected IntegerLiteral(99), got: {:?}",
                    expr
                );
            } else {
                panic!("expected ExpressionStatement");
            }
        } else {
            panic!("expected Program");
        }

        // Cleanup.
        if let Ok(mut g) = MACROS.write() {
            g.clear();
        }
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_accepts_well_formed_macro_decl() {
        let result = check_macro_decl(
            r#"pattern = "$1, $2", expansion = "if $1 != $2 { panic(\"not equal\") }""#,
        );
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn check_rejects_missing_pattern() {
        let result = check_macro_decl(r#"expansion = "$1""#).unwrap_err();
        assert_eq!(
            result,
            "test.rz:0:0: error: invalid #[macro] declaration for `macro_target`: missing required `pattern` field"
        );
    }

    #[test]
    fn check_rejects_missing_expansion() {
        let result = check_macro_decl(r#"pattern = "$1""#).unwrap_err();
        assert_eq!(
            result,
            "test.rz:0:0: error: invalid #[macro] declaration for `macro_target`: missing required `expansion` field"
        );
    }

    #[test]
    fn check_rejects_malformed_entry() {
        let result = check_macro_decl(r#"pattern "$1", expansion = "$1""#).unwrap_err();
        assert_eq!(
            result,
            "test.rz:0:0: error: invalid #[macro] declaration for `macro_target`: malformed entry `pattern \"$1\"`; expected `key = \"value\"`"
        );
    }

    #[test]
    fn check_rejects_duplicate_pattern() {
        let result =
            check_macro_decl(r#"pattern = "$1", pattern = "$2", expansion = "$3""#).unwrap_err();
        assert_eq!(
            result,
            "test.rz:0:0: error: invalid #[macro] declaration for `macro_target`: duplicate `pattern` field"
        );
    }

    #[test]
    fn check_rejects_duplicate_macro_registration() {
        let result = check_macro_decls(&[
            (
                "macro_target",
                12,
                r#"pattern = "$1", expansion = "prefix($1)""#,
            ),
            (
                "macro_target",
                34,
                r#"pattern = "$1", expansion = "prefix($1)""#,
            ),
        ])
        .unwrap_err();
        assert_eq!(
            result,
            "test.rz:34:0: error: duplicate #[macro] declaration `macro_target`; first declared at test.rz:12:0, second declared at test.rz:34:0"
        );
    }

    #[test]
    fn check_rejects_conflicting_macro_registration() {
        let result = check_macro_decls(&[
            (
                "macro_target",
                12,
                r#"pattern = "$1", expansion = "prefix($1)""#,
            ),
            (
                "macro_target",
                34,
                r#"pattern = "$1", expansion = "suffix($1)""#,
            ),
        ])
        .unwrap_err();
        assert_eq!(
            result,
            "test.rz:34:0: error: conflicting #[macro] declaration `macro_target`; first declared at test.rz:12:0, second declared at test.rz:34:0"
        );
    }

    #[test]
    fn check_rejects_unknown_field() {
        let result = check_macro_decl(r#"pattern = "$1", replacement = "$2", expansion = "$3""#)
            .unwrap_err();
        assert_eq!(
            result,
            "test.rz:0:0: error: invalid #[macro] declaration for `macro_target`: unknown field `replacement` in macro declaration for `macro_target`"
        );
    }

    #[test]
    fn check_rejects_bare_placeholder_marker() {
        let result = check_macro_decl_err(r#"pattern = "$1", expansion = "foo $""#);
        assert_eq!(
            result,
            "test.rz:0:0: error: invalid #[macro] declaration for `macro_target`: invalid placeholder `$` in `expansion`: expected `$N`"
        );
    }

    #[test]
    fn check_rejects_zero_placeholder_index() {
        let result = check_macro_decl_err(r#"pattern = "$0", expansion = "$0""#);
        assert_eq!(
            result,
            "test.rz:0:0: error: invalid #[macro] declaration for `macro_target`: invalid placeholder `$0` in `pattern`: placeholder indices start at 1"
        );
    }

    #[test]
    fn check_rejects_leading_zero_placeholder_index() {
        let result = check_macro_decl_err(r#"pattern = "$01", expansion = "$01""#);
        assert_eq!(
            result,
            "test.rz:0:0: error: invalid #[macro] declaration for `macro_target`: invalid placeholder `$01` in `pattern`: leading zeroes are not allowed"
        );
    }

    #[test]
    fn check_rejects_multi_digit_placeholder_index() {
        let result = check_macro_decl_err(r#"pattern = "$10", expansion = "$10""#);
        assert_eq!(
            result,
            "test.rz:0:0: error: invalid #[macro] declaration for `macro_target`: invalid placeholder `$10` in `pattern`: multi-digit placeholders are not supported"
        );
    }

    #[test]
    fn check_rejects_expansion_placeholder_past_arity() {
        let result = check_macro_decl_err(r#"pattern = "$1", expansion = "$2""#);
        assert_eq!(
            result,
            "test.rz:0:0: error: invalid #[macro] declaration for `macro_target`: expansion references placeholder `$2` but `pattern` only declares `$1..$1`"
        );
    }

    #[test]
    fn check_rejects_unparseable_expansion_after_substitution() {
        let result = check_macro_decl_err(r#"pattern = "$1", expansion = "$1 +""#);
        assert_eq!(
            result,
            "test.rz:0:0: error: invalid #[macro] declaration for `macro_target`: expansion does not parse after placeholder substitution"
        );
    }
}
