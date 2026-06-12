//! Feature 13/50 - Temporal Type States.
//!
//! `#[typestate(states = "Closed Open Flushed", transitions = "Closed:open->Open Open:flush->Flushed Open:close->Closed Flushed:close->Closed")]`
//! attached to a struct turns it into a typestate type: the value's
//! state evolves across method calls, and calls that violate the
//! state machine are rejected.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use crate::span::Span;
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Clone)]
pub struct TypestateSpec {
    pub struct_name: String,
    pub states: Vec<String>,
    /// Map of (current_state, method) -> next_state.
    pub transitions: HashMap<(String, String), String>,
}

static SPECS: RwLock<Vec<TypestateSpec>> = RwLock::new(Vec::new());

pub fn collect() -> Vec<TypestateSpec> {
    let attrs = crate::feature_attrs::find_kind("typestate");
    let mut out = Vec::with_capacity(attrs.len());

    for (item, rec) in attrs {
        let mut spec = TypestateSpec {
            struct_name: item,
            states: Vec::new(),
            transitions: HashMap::new(),
        };

        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "states" => {
                        spec.states = v.split_whitespace().map(|s| s.to_string()).collect();
                    }
                    "transitions" => {
                        for t in v.split_whitespace() {
                            let Some((lhs, next)) = t.split_once("->") else {
                                continue;
                            };
                            let Some((state, method)) = lhs.split_once(':') else {
                                continue;
                            };
                            spec.transitions
                                .insert((state.to_string(), method.to_string()), next.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }

        out.push(spec);
    }

    out
}

pub fn install(specs: Vec<TypestateSpec>) {
    if let Ok(mut g) = SPECS.write() {
        *g = specs;
    }
}

pub fn validate_call(
    struct_name: &str,
    current_state: &str,
    method: &str,
) -> Result<String, String> {
    let g = SPECS
        .read()
        .map_err(|_| format!("no typestate {struct_name}"))?;
    let spec = g
        .iter()
        .find(|s| s.struct_name == struct_name)
        .ok_or_else(|| format!("no typestate for {struct_name}"))?;
    spec.transitions
        .get(&(current_state.to_string(), method.to_string()))
        .cloned()
        .ok_or_else(|| {
            format!(
                "typestate violation: cannot call `{}` on `{}` in state `{}`",
                method, struct_name, current_state
            )
        })
}

fn diagnostic(source_path: &str, span: Span, message: &str) -> String {
    format!(
        "{}:{}:{}: error: {}",
        source_path, span.start.line, span.start.column, message
    )
}

fn span_of(node: &Node) -> Span {
    match node {
        Node::Identifier { span, .. }
        | Node::IntegerLiteral { span, .. }
        | Node::FloatLiteral { span, .. }
        | Node::StringLiteral { span, .. }
        | Node::StringInternLiteral { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::BytesLiteral { span, .. }
        | Node::CharLiteral { span, .. }
        | Node::InterpolatedString { span, .. }
        | Node::CallExpression { span, .. }
        | Node::ExpressionStatement { span, .. }
        | Node::StructLiteral { span, .. }
        | Node::ArrayLiteral { span, .. }
        | Node::TupleLiteral { span, .. }
        | Node::MapLiteral { span, .. }
        | Node::SetLiteral { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::InfixExpression { span, .. }
        | Node::TryExpression { span, .. }
        | Node::OptionalChain { span, .. }
        | Node::FieldAccess { span, .. }
        | Node::FieldAssignment { span, .. }
        | Node::IndexExpression { span, .. }
        | Node::IndexAssignment { span, .. }
        | Node::Slice { span, .. }
        | Node::Assignment { span, .. }
        | Node::ReturnStatement { span, .. }
        | Node::LetStatement { span, .. }
        | Node::Block { span, .. }
        | Node::WhileStatement { span, .. }
        | Node::ForInStatement { span, .. }
        | Node::IfStatement { span, .. }
        | Node::Match { span, .. }
        | Node::TryCatch { span, .. }
        | Node::Function { span, .. }
        | Node::FunctionLiteral { span, .. }
        | Node::ImplBlock { span, .. }
        | Node::LiveBlock { span, .. }
        | Node::Quantifier { span, .. }
        | Node::InvariantStatement { span, .. }
        | Node::NamedArg { span, .. }
        | Node::LetDestructureStruct { span, .. }
        | Node::StructDecl { span, .. }
        | Node::Extern { span, .. }
        | Node::ModuleDecl { span, .. }
        | Node::NewtypeDecl { span, .. }
        | Node::NewtypeConstruct { span, .. }
        | Node::SupervisorDecl { span, .. }
        | Node::Assert { span, .. }
        | Node::Assume { span, .. }
        | Node::DeferStatement { span, .. } => *span,
        _ => Span::default(),
    }
}

fn arg_kind(node: &Node) -> Option<&'static str> {
    match node {
        Node::StringLiteral { .. }
        | Node::StringInternLiteral { .. }
        | Node::InterpolatedString { .. } => Some("string literal"),
        Node::IntegerLiteral { .. } | Node::FloatLiteral { .. } => Some("number literal"),
        Node::ArrayLiteral { .. } => Some("list literal"),
        Node::StructLiteral { .. } => Some("struct literal"),
        _ => None,
    }
}

fn validate_validate_call(
    function: &Node,
    arguments: &[Node],
    call_span: Span,
    source_path: &str,
) -> Result<(), String> {
    if !matches!(
        function,
        Node::Identifier { name, .. }
            if matches!(name.as_str(), "validate_call" | "typestate_types::validate_call")
    ) {
        return Ok(());
    }

    if arguments.len() != 3 {
        return Err(diagnostic(
            source_path,
            call_span,
            &format!(
                "typestate validate_call expects 3 arguments, got {}",
                arguments.len()
            ),
        ));
    }

    for (idx, arg) in arguments.iter().enumerate() {
        if let Some(kind) = arg_kind(arg)
            && kind != "string literal"
        {
            return Err(diagnostic(
                source_path,
                span_of(arg),
                &format!(
                    "typestate validate_call argument {} must be string literal, got {}",
                    idx + 1,
                    kind
                ),
            ));
        }
    }

    Ok(())
}

fn walk_typestate_calls(node: &Node, source_path: &str) -> Result<(), String> {
    match node {
        Node::Program(stmts) => {
            for stmt in stmts {
                walk_typestate_calls(&stmt.node, source_path)?;
            }
        }
        Node::Block { stmts, .. } => {
            for stmt in stmts {
                walk_typestate_calls(stmt, source_path)?;
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            ..
        } => {
            walk_typestate_calls(body, source_path)?;
            for expr in requires {
                walk_typestate_calls(expr, source_path)?;
            }
            for expr in ensures {
                walk_typestate_calls(expr, source_path)?;
            }
        }
        Node::FunctionLiteral {
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            walk_typestate_calls(body, source_path)?;
            for expr in requires {
                walk_typestate_calls(expr, source_path)?;
            }
            for expr in ensures {
                walk_typestate_calls(expr, source_path)?;
            }
            if let Some(expr) = recovers_to {
                walk_typestate_calls(expr, source_path)?;
            }
        }
        Node::ImplBlock { methods, .. } => {
            for method in methods {
                walk_typestate_calls(method, source_path)?;
            }
        }
        Node::LiveBlock {
            body,
            invariants,
            timeout,
            ..
        } => {
            walk_typestate_calls(body, source_path)?;
            for invariant in invariants {
                walk_typestate_calls(invariant, source_path)?;
            }
            if let Some(timeout) = timeout {
                walk_typestate_calls(timeout, source_path)?;
            }
        }
        Node::ExpressionStatement { expr, .. }
        | Node::ReturnStatement {
            value: Some(expr), ..
        }
        | Node::LetStatement { value: expr, .. }
        | Node::Assignment { value: expr, .. }
        | Node::TryExpression { expr, .. }
        | Node::DeferStatement { expr, .. }
        | Node::Assert {
            condition: expr, ..
        }
        | Node::Assume {
            condition: expr, ..
        }
        | Node::InvariantStatement { expr, .. }
        | Node::NamedArg { value: expr, .. }
        | Node::LetDestructureStruct { value: expr, .. } => {
            walk_typestate_calls(expr, source_path)?;
        }
        Node::CallExpression {
            function,
            arguments,
            span,
        } => {
            walk_typestate_calls(function, source_path)?;
            validate_validate_call(function, arguments, *span, source_path)?;
            for arg in arguments {
                walk_typestate_calls(arg, source_path)?;
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_typestate_calls(condition, source_path)?;
            walk_typestate_calls(consequence, source_path)?;
            if let Some(alt) = alternative {
                walk_typestate_calls(alt, source_path)?;
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            walk_typestate_calls(condition, source_path)?;
            walk_typestate_calls(body, source_path)?;
        }
        Node::ForInStatement { iterable, body, .. } => {
            walk_typestate_calls(iterable, source_path)?;
            walk_typestate_calls(body, source_path)?;
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            walk_typestate_calls(scrutinee, source_path)?;
            for (_, guard, arm_body) in arms {
                if let Some(guard) = guard {
                    walk_typestate_calls(guard, source_path)?;
                }
                walk_typestate_calls(arm_body, source_path)?;
            }
        }
        Node::TryCatch { body, handlers, .. } => {
            for stmt in body {
                walk_typestate_calls(stmt, source_path)?;
            }
            for (_, handler_body) in handlers {
                for stmt in handler_body {
                    walk_typestate_calls(stmt, source_path)?;
                }
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            if let Some(base) = base {
                walk_typestate_calls(base, source_path)?;
            }
            for (_, value) in fields {
                walk_typestate_calls(value, source_path)?;
            }
        }
        Node::ArrayLiteral { items, .. } | Node::TupleLiteral { items, .. } => {
            for item in items {
                walk_typestate_calls(item, source_path)?;
            }
        }
        Node::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                walk_typestate_calls(key, source_path)?;
                walk_typestate_calls(value, source_path)?;
            }
        }
        Node::SetLiteral { items, .. } => {
            for item in items {
                walk_typestate_calls(item, source_path)?;
            }
        }
        Node::FieldAccess { target, .. }
        | Node::IndexExpression { target, .. }
        | Node::Slice { target, .. }
        | Node::OptionalChain { object: target, .. } => {
            walk_typestate_calls(target, source_path)?;
        }
        Node::FieldAssignment { target, value, .. }
        | Node::IndexAssignment { target, value, .. } => {
            walk_typestate_calls(target, source_path)?;
            walk_typestate_calls(value, source_path)?;
        }
        Node::PrefixExpression { right, .. } => {
            walk_typestate_calls(right, source_path)?;
        }
        Node::InfixExpression { left, right, .. } => {
            walk_typestate_calls(left, source_path)?;
            walk_typestate_calls(right, source_path)?;
        }
        _ => {}
    }

    Ok(())
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    walk_typestate_calls(program, source_path)?;

    // RES-1306: gate `install` on non-empty case avoids
    // wasted RwLock write per compilation and removes the
    // wipe-on-empty test race shape documented in RES-1302.
    let specs = collect();
    if !specs.is_empty() {
        install(specs);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_protocol_validates_close_after_open() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "File",
            crate::feature_attrs::AttrRecord {
                name: "typestate".into(),
                args: r#"states = "Closed Open", transitions = "Closed:open->Open Open:close->Closed""#
                    .into(),
                line: 0,
            },
        );
        install(collect());
        assert_eq!(
            validate_call("File", "Closed", "open").unwrap(),
            "Open".to_string()
        );
        assert!(validate_call("File", "Closed", "close").is_err());
        crate::feature_attrs::reset();
    }

    #[test]
    fn invalid_method_in_valid_state_is_rejected() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Lock",
            crate::feature_attrs::AttrRecord {
                name: "typestate".into(),
                args: r#"states = "Locked Unlocked", transitions = "Locked:unlock->Unlocked Unlocked:lock->Locked""#
                    .into(),
                line: 0,
            },
        );
        install(collect());
        assert_eq!(
            validate_call("Lock", "Locked", "unlock").unwrap(),
            "Unlocked"
        );
        assert_eq!(validate_call("Lock", "Unlocked", "lock").unwrap(), "Locked");
        assert!(validate_call("Lock", "Locked", "lock").is_err());
        assert!(validate_call("Lock", "Unlocked", "unlock").is_err());
        crate::feature_attrs::reset();
    }

    #[test]
    fn three_state_machine_full_cycle() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Connection",
            crate::feature_attrs::AttrRecord {
                name: "typestate".into(),
                args: r#"states = "Idle Active Closed", transitions = "Idle:connect->Active Active:send->Active Active:disconnect->Closed""#
                    .into(),
                line: 0,
            },
        );
        install(collect());
        let s1 = validate_call("Connection", "Idle", "connect").unwrap();
        assert_eq!(s1, "Active");
        let s2 = validate_call("Connection", "Active", "send").unwrap();
        assert_eq!(s2, "Active");
        let s3 = validate_call("Connection", "Active", "disconnect").unwrap();
        assert_eq!(s3, "Closed");
        assert!(validate_call("Connection", "Closed", "connect").is_err());
        crate::feature_attrs::reset();
    }

    #[test]
    fn unknown_struct_returns_error() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        install(collect());
        let result = validate_call("Nonexistent", "SomeState", "someMethod");
        assert!(result.is_err(), "unknown struct must return an error");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("Nonexistent"),
            "error must name the unknown struct: {msg}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_ok_without_attributes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn validate_call_accepts_string_arguments() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Lock",
            crate::feature_attrs::AttrRecord {
                name: "typestate".into(),
                args: r#"states = "Locked Unlocked", transitions = "Locked:unlock->Unlocked Unlocked:lock->Locked""#
                    .into(),
                line: 0,
            },
        );
        let src = r#"
fn main() -> int {
    validate_call("Lock", "Locked", "unlock");
    return 0;
}
"#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn validate_call_rejects_wrong_arity() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Lock",
            crate::feature_attrs::AttrRecord {
                name: "typestate".into(),
                args: r#"states = "Locked Unlocked", transitions = "Locked:unlock->Unlocked Unlocked:lock->Locked""#
                    .into(),
                line: 0,
            },
        );
        let src = r#"
fn main() -> int {
    validate_call("Lock", "Locked");
    return 0;
}
"#;
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "test").expect_err("arity should be rejected");
        assert!(
            err.contains("expects 3 arguments, got 2"),
            "unexpected error: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn validate_call_rejects_non_string_arguments() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Lock",
            crate::feature_attrs::AttrRecord {
                name: "typestate".into(),
                args: r#"states = "Locked Unlocked", transitions = "Locked:unlock->Unlocked Unlocked:lock->Locked""#
                    .into(),
                line: 0,
            },
        );
        let src = r#"
fn main() -> int {
    validate_call(1, ["Locked"], new Lock {});
    return 0;
}
"#;
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "test").expect_err("kind contract should be rejected");
        assert!(
            err.contains("argument 1 must be string literal, got number literal")
                || err.contains("argument 2 must be string literal, got list literal")
                || err.contains("argument 3 must be string literal, got struct literal"),
            "unexpected error: {err}"
        );
        crate::feature_attrs::reset();
    }
}
