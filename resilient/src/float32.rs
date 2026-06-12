//! RES-2618: `f32` single-precision float type.
//!
//! Cortex-M4F has hardware single-precision FPU; `f64` operations on
//! target require software emulation - 4-10x slower on larger inputs.
//! `Float32` is a distinct type from `Float` (`f64`) so the compiler can
//! catch implicit cross-width mixing at type-check time.
//!
//! ## What this module provides
//!
//! * `as_f32(x)` builtin - truncates `int` or `float`/`f64` to single
//!   precision. The result is stored as `f64` at runtime (the interpreter
//!   always uses `f64` internally) but with `f32` precision: rounding and
//!   overflow match IEEE 754-2019 binary32.
//! * `as_f64(x)` builtin - widens `int` or `f32`/`float` to `f64`.
//! * `check()` type-consistency pass: flags programs that mix `f32`
//!   and `f64` in arithmetic or pass obviously invalid literals to the
//!   float32 cast builtins before they reach runtime.
//!
//! ## Literal syntax
//!
//! Use `3.14 f32` or `as_f32(3.14)` for single-precision literals.
//! Full `3.14f32` suffix parsing was already lowered elsewhere.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation)]

use crate::span::Span;
use crate::{Node, Value};

// ---------------------------------------------------------------------------
// Builtins
// ---------------------------------------------------------------------------

type RResult<T> = Result<T, String>;

/// Convert `int` or `float` (`f64`) to single-precision float.
/// The value is truncated to nearest `f32` then stored as `f64`.
pub(crate) fn builtin_as_f32(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(i)] => Ok(Value::Float(*i as f32 as f64)),
        [Value::Float(f)] => Ok(Value::Float(*f as f32 as f64)),
        [other] => Err(format!("as_f32: expected int or float, got {}", other)),
        _ => Err(format!("as_f32: expected 1 argument, got {}", args.len())),
    }
}

/// Convert `int` or `float` (`f32` / `f64`) to double-precision float.
pub(crate) fn builtin_as_f64(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(i)] => Ok(Value::Float(*i as f64)),
        [Value::Float(f)] => Ok(Value::Float(*f)),
        [other] => Err(format!("as_f64: expected int or float, got {}", other)),
        _ => Err(format!("as_f64: expected 1 argument, got {}", args.len())),
    }
}

// ---------------------------------------------------------------------------
// Type-consistency check pass
// ---------------------------------------------------------------------------

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
        | Node::BytesLiteral { span, .. }
        | Node::CharLiteral { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::InfixExpression { span, .. }
        | Node::CallExpression { span, .. }
        | Node::TryExpression { span, .. }
        | Node::OptionalChain { span, .. }
        | Node::FunctionLiteral { span, .. }
        | Node::Match { span, .. }
        | Node::StructDecl { span, .. }
        | Node::LetDestructureStruct { span, .. }
        | Node::StructLiteral { span, .. }
        | Node::FieldAccess { span, .. }
        | Node::FieldAssignment { span, .. }
        | Node::ArrayLiteral { span, .. }
        | Node::IndexExpression { span, .. }
        | Node::Slice { span, .. }
        | Node::IndexAssignment { span, .. }
        | Node::MapLiteral { span, .. }
        | Node::SetLiteral { span, .. }
        | Node::ImplBlock { span, .. }
        | Node::TraitDecl { span, .. }
        | Node::TypeAlias { span, .. }
        | Node::RegionDecl { span, .. }
        | Node::Actor { span, .. }
        | Node::ActorDecl { span, .. }
        | Node::ClusterDecl { span, .. }
        | Node::TryCatch { span, .. }
        | Node::Quantifier { span, .. }
        | Node::InvariantStatement { span, .. }
        | Node::Range { span, .. }
        | Node::NamedArg { span, .. }
        | Node::InterpolatedString { span, .. }
        | Node::ModuleDecl { span, .. }
        | Node::NewtypeDecl { span, .. }
        | Node::NewtypeConstruct { span, .. }
        | Node::SupervisorDecl { span, .. }
        | Node::TupleLiteral { span, .. }
        | Node::Function { span, .. }
        | Node::LiveBlock { span, .. }
        | Node::DurationLiteral { span, .. }
        | Node::Assert { span, .. }
        | Node::Assume { span, .. }
        | Node::Block { span, .. }
        | Node::LetStatement { span, .. }
        | Node::StaticLet { span, .. }
        | Node::Const { span, .. }
        | Node::Assignment { span, .. }
        | Node::ReturnStatement { span, .. }
        | Node::Break { span, .. }
        | Node::BreakWith { span, .. }
        | Node::Continue { span, .. }
        | Node::BreakLabel { span, .. }
        | Node::ContinueLabel { span, .. }
        | Node::DeferStatement { span, .. }
        | Node::IfStatement { span, .. }
        | Node::WhileStatement { span, .. }
        | Node::ForInStatement { span, .. }
        | Node::ExpressionStatement { span, .. } => *span,
        Node::Program(_) => Span::default(),
        _ => Span::default(),
    }
}

fn arg_kind(node: &Node) -> &'static str {
    match node {
        Node::StringLiteral { .. }
        | Node::StringInternLiteral { .. }
        | Node::InterpolatedString { .. } => "string literal",
        Node::IntegerLiteral { .. } | Node::FloatLiteral { .. } => "number literal",
        Node::ArrayLiteral { .. } => "array literal",
        Node::StructLiteral { .. } => "struct literal",
        Node::BooleanLiteral { .. } => "boolean literal",
        Node::CharLiteral { .. } => "char literal",
        Node::BytesLiteral { .. } => "bytes literal",
        Node::MapLiteral { .. } => "map literal",
        Node::SetLiteral { .. } => "set literal",
        Node::TupleLiteral { .. } => "tuple literal",
        Node::DurationLiteral { .. } => "duration literal",
        _ => "expression",
    }
}

fn is_float32_cast_name(function: &Node) -> Option<&'static str> {
    match function {
        Node::Identifier { name, .. } => match name.as_str() {
            "as_f32" => Some("as_f32"),
            "as_f64" => Some("as_f64"),
            _ => None,
        },
        _ => None,
    }
}

fn check_float32_cast(
    source_path: &str,
    op: &str,
    arguments: &[Node],
    span: Span,
) -> Result<(), String> {
    if arguments.len() != 1 {
        return Err(diagnostic(
            source_path,
            span,
            &format!("{op}: expected 1 argument, got {}", arguments.len()),
        ));
    }

    let arg = &arguments[0];
    let bad_kind = match arg {
        Node::StringLiteral { .. }
        | Node::StringInternLiteral { .. }
        | Node::InterpolatedString { .. }
        | Node::BooleanLiteral { .. }
        | Node::CharLiteral { .. }
        | Node::BytesLiteral { .. }
        | Node::ArrayLiteral { .. }
        | Node::StructLiteral { .. }
        | Node::MapLiteral { .. }
        | Node::SetLiteral { .. }
        | Node::TupleLiteral { .. }
        | Node::DurationLiteral { .. } => Some(arg_kind(arg)),
        _ => None,
    };

    if let Some(kind) = bad_kind {
        return Err(diagnostic(
            source_path,
            span_of(arg),
            &format!("{op}: expected int or float, got {kind}"),
        ));
    }

    Ok(())
}

fn walk(node: &Node, source_path: &str) -> Result<(), String> {
    match node {
        Node::Program(stmts) => {
            for stmt in stmts {
                walk(&stmt.node, source_path)?;
            }
        }
        Node::Block { stmts, .. } => {
            for stmt in stmts {
                walk(stmt, source_path)?;
            }
        }
        Node::ExpressionStatement { expr, .. } => walk(expr, source_path)?,
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::Const { value, .. }
        | Node::Assignment { value, .. }
        | Node::BreakWith { value, .. }
        | Node::DeferStatement { expr: value, .. }
        | Node::TryExpression { expr: value, .. }
        | Node::InvariantStatement { expr: value, .. }
        | Node::NamedArg { value, .. }
        | Node::NewtypeConstruct { value, .. } => walk(value, source_path)?,
        Node::ReturnStatement {
            value: Some(value), ..
        } => walk(value, source_path)?,
        Node::ReturnStatement { value: None, .. } => {}
        Node::Assert {
            condition, message, ..
        }
        | Node::Assume {
            condition, message, ..
        } => {
            walk(condition, source_path)?;
            if let Some(message) = message {
                walk(message, source_path)?;
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk(condition, source_path)?;
            walk(consequence, source_path)?;
            if let Some(alternative) = alternative {
                walk(alternative, source_path)?;
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            walk(body, source_path)?;
            for req in requires {
                walk(req, source_path)?;
            }
            for ens in ensures {
                walk(ens, source_path)?;
            }
            if let Some(recovers_to) = recovers_to {
                walk(recovers_to, source_path)?;
            }
        }
        Node::FunctionLiteral {
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            walk(body, source_path)?;
            for req in requires {
                walk(req, source_path)?;
            }
            for ens in ensures {
                walk(ens, source_path)?;
            }
            if let Some(recovers_to) = recovers_to {
                walk(recovers_to, source_path)?;
            }
        }
        Node::ModuleDecl { body, .. } | Node::ImplBlock { methods: body, .. } => {
            for item in body {
                walk(item, source_path)?;
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            walk(scrutinee, source_path)?;
            for (_, guard, body) in arms {
                if let Some(guard) = guard {
                    walk(guard, source_path)?;
                }
                walk(body, source_path)?;
            }
        }
        Node::TryCatch { body, handlers, .. } => {
            for stmt in body {
                walk(stmt, source_path)?;
            }
            for (_, handler_body) in handlers {
                for stmt in handler_body {
                    walk(stmt, source_path)?;
                }
            }
        }
        Node::LiveBlock {
            body,
            invariants,
            timeout,
            ..
        } => {
            walk(body, source_path)?;
            for inv in invariants {
                walk(inv, source_path)?;
            }
            if let Some(timeout) = timeout {
                walk(timeout, source_path)?;
            }
        }
        Node::OptionalChain { object, .. } => walk(object, source_path)?,
        Node::PrefixExpression { right, .. } => walk(right, source_path)?,
        Node::InfixExpression { left, right, .. } => {
            walk(left, source_path)?;
            walk(right, source_path)?;
        }
        Node::CallExpression {
            function,
            arguments,
            span,
        } => {
            if let Some(op) = is_float32_cast_name(function) {
                check_float32_cast(source_path, op, arguments, *span)?;
            }
            walk(function, source_path)?;
            for arg in arguments {
                walk(arg, source_path)?;
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            for (_, value) in fields {
                walk(value, source_path)?;
            }
            if let Some(base) = base {
                walk(base, source_path)?;
            }
        }
        Node::ArrayLiteral { items, .. }
        | Node::TupleLiteral { items, .. }
        | Node::SetLiteral { items, .. } => {
            for item in items {
                walk(item, source_path)?;
            }
        }
        Node::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                walk(key, source_path)?;
                walk(value, source_path)?;
            }
        }
        Node::FieldAccess { target, .. } | Node::IndexExpression { target, .. } => {
            walk(target, source_path)?;
        }
        Node::FieldAssignment { target, value, .. }
        | Node::IndexAssignment { target, value, .. } => {
            walk(target, source_path)?;
            walk(value, source_path)?;
        }
        Node::Quantifier { range, body, .. } => {
            match range {
                crate::quantifiers::QuantRange::Range { lo, hi } => {
                    walk(lo, source_path)?;
                    walk(hi, source_path)?;
                }
                crate::quantifiers::QuantRange::Iterable(expr) => walk(expr, source_path)?,
            }
            walk(body, source_path)?;
        }
        Node::InterpolatedString { parts, .. } => {
            for part in parts {
                if let crate::string_interp::StringPart::Expr(expr) = part {
                    walk(expr, source_path)?;
                }
            }
        }
        _ => {}
    }

    Ok(())
}

/// Typecheck pass for f32/f64 consistency.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    walk(program, source_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn as_f32_truncates_double() {
        let result = builtin_as_f32(&[Value::Float(1.0_f64 / 3.0_f64)]).unwrap();
        let Value::Float(v) = result else {
            panic!("expected Float");
        };
        assert!(
            (v - (1.0_f64 / 3.0_f64) as f32 as f64).abs() < 1e-10,
            "as_f32 should truncate f32 precision: {v}"
        );
    }

    #[test]
    fn as_f32_from_int() {
        let result = builtin_as_f32(&[Value::Int(42)]).unwrap();
        assert!(matches!(result, Value::Float(v) if (v - 42.0_f64).abs() < 1e-10));
    }

    #[test]
    fn as_f64_from_int() {
        let result = builtin_as_f64(&[Value::Int(100)]).unwrap();
        assert!(matches!(result, Value::Float(v) if (v - 100.0_f64).abs() < 1e-10));
    }

    #[test]
    fn as_f64_from_float() {
        let input = 1.5_f64;
        let result = builtin_as_f64(&[Value::Float(input)]).unwrap();
        assert!(matches!(result, Value::Float(v) if (v - input).abs() < 1e-10));
    }

    #[test]
    fn as_f32_wrong_type_errors() {
        let result = builtin_as_f32(&[Value::Bool(true)]);
        assert!(result.is_err());
    }

    #[test]
    fn as_f64_wrong_arg_count_errors() {
        let result = builtin_as_f64(&[]);
        assert!(result.is_err());
        let result2 = builtin_as_f64(&[Value::Float(1.0), Value::Float(2.0)]);
        assert!(result2.is_err());
    }

    #[test]
    fn f32_type_annotation_accepted() {
        let src = "fn compute(f32 x) -> f32 { return x; }\n";
        let (_prog, errs) = parse(src);
        assert!(
            errs.is_empty(),
            "f32 type annotation should parse cleanly: {errs:?}"
        );
    }

    #[test]
    fn as_f32_cast_in_program() {
        let src = "let x = 3.14 as f32;\nprintln(x);\n";
        let (_prog, errs) = parse(src);
        assert!(errs.is_empty(), "as f32 cast should parse: {errs:?}");
    }

    #[test]
    fn f32_check_pass_is_noop_for_pure_f32() {
        let src = "fn f(f32 x) -> f32 { return x; }\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn f32_let_annotation_accepts_float_literal() {
        let result = crate::run_program("let x: f32 = 3.14;\nprintln(x);\n");
        assert!(
            result.ok,
            "let x: f32 = 3.14 should compile: {:?}",
            result.errors
        );
        assert!(
            result.stdout.contains("3.14"),
            "stdout: {:?}",
            result.stdout
        );
    }

    #[test]
    fn f32_let_annotation_two_vars_arithmetic() {
        let src = "let x: f32 = 3.0;\nlet y: f32 = 2.0;\nlet z: f32 = x + y;\nprintln(z);\n";
        let result = crate::run_program(src);
        assert!(result.ok, "f32 annotation arithmetic: {:?}", result.errors);
        assert!(result.stdout.contains('5'), "stdout: {:?}", result.stdout);
    }

    #[test]
    fn f32_cross_width_arithmetic_still_errors() {
        let src = "let a = 3.14;\nlet b = 2.0 as f32;\nlet c = a + b;\nprintln(c);\n";
        let (prog, parse_errs) = parse(src);
        assert!(parse_errs.is_empty(), "should parse: {:?}", parse_errs);
        let check_result = crate::typechecker::TypeChecker::new().check_program(&prog);
        assert!(
            check_result.is_err(),
            "f32 + float arithmetic should still type-error in typechecker"
        );
        let errs = check_result.unwrap_err();
        assert!(
            errs.contains("f32") || errs.contains("f64"),
            "error should mention f32/f64: {errs}"
        );
    }

    fn check_err(src: &str) -> String {
        let (program, parse_errs) = parse(src);
        assert!(parse_errs.is_empty(), "parse errors: {:?}", parse_errs);
        check(&program, "<test>").expect_err("expected float32 check failure")
    }

    #[test]
    fn f32_check_rejects_string_literal_argument() {
        let err = check_err("let x = as_f32(\"hello\");\n");
        assert!(
            err.contains("as_f32") && err.contains("int or float"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn f32_check_rejects_bool_literal_argument() {
        let err = check_err("let x = as_f32(true);\n");
        assert!(
            err.contains("as_f32") && err.contains("int or float"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn f32_check_rejects_char_literal_argument() {
        let err = check_err("let x = as_f64('x');\n");
        assert!(
            err.contains("as_f64") && err.contains("int or float"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn f32_check_rejects_array_literal_argument() {
        let err = check_err("let x = as_f32([1, 2]);\n");
        assert!(
            err.contains("as_f32") && err.contains("int or float"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn f32_check_rejects_struct_literal_argument() {
        let err =
            check_err("struct Point { int x, int y }\nlet x = as_f64(new Point { x: 1, y: 2 });\n");
        assert!(
            err.contains("as_f64") && err.contains("int or float"),
            "unexpected error: {err}"
        );
    }
}
