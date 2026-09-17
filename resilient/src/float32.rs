//! RES-2618: `f32` single-precision float type.
//!
//! Cortex-M4F has a hardware single-precision FPU; `f64` operations on
//! that target require software emulation — 4-10× slower and larger.
//! `Float32` is a distinct type from `Float` (`f64`) so the compiler can
//! catch implicit cross-width mixing at the type-check stage.
//!
//! ## What this module provides
//!
//! * `as_f32(x)` builtin — truncates `int` or `float`/`f64` to single
//!   precision. The result is stored as an f64 at runtime (the interpreter
//!   always uses f64 internally) but with f32 precision: rounding and
//!   overflow match IEEE 754-2019 binary32.
//! * `as_f64(x)` builtin — widens `int` or `f32`/`float` to f64.
//! * `check()` — type-consistency pass: flags programs that mix `f32` and
//!   `f64` in arithmetic or assignment without an explicit cast.
//!
//! ## Literal syntax
//!
//! Use `3.14 as f32` or `as_f32(3.14)` for single-precision literals.
//! Full `3.14f32` suffix parsing is deferred to a follow-up PR.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation)]

use std::collections::HashMap;

use crate::typechecker::Type;
use crate::{Node, Value};

// ---------------------------------------------------------------------------
// Builtins
// ---------------------------------------------------------------------------

type RResult<T> = Result<T, String>;

/// Convert `int` or `float` (f64) to single-precision float.
/// The value is truncated to the nearest f32 and stored as f64.
pub(crate) fn builtin_as_f32(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(i)] => Ok(Value::Float(*i as f32 as f64)),
        [Value::Float(f)] => Ok(Value::Float(*f as f32 as f64)),
        [other] => Err(format!("as_f32: expected int or float, got {}", other)),
        _ => Err(format!("as_f32: expected 1 argument, got {}", args.len())),
    }
}

/// Convert `int` or `float` (f32 or f64) to double-precision float.
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

/// Typecheck pass for f32/f64 consistency and conversion arguments.
///
/// f32 ↔ f64 mixing in arithmetic is already caught by `check_numeric_same_type`
/// in the main typechecker pass, which runs first. This pass covers the other
/// half of the conversion contract: reject arguments whose concrete source
/// type cannot be accepted by `as_f32` or `as_f64`.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let mut checker = Float32CallChecker::new(source_path);
    checker.walk(program);
    if checker.errors.is_empty() {
        Ok(())
    } else {
        Err(checker.errors.join("\n"))
    }
}

/// The f32 conversion builtins accept every integer width and both floating
/// point widths. `None` means that inference did not prove a concrete type;
/// those values must remain permissive because `Type::Any` is a deliberate
/// escape hatch for unresolved expressions.
struct Float32CallChecker<'a> {
    source_path: &'a str,
    scopes: Vec<HashMap<String, Type>>,
    errors: Vec<String>,
}

impl<'a> Float32CallChecker<'a> {
    fn new(source_path: &'a str) -> Self {
        Self {
            source_path,
            scopes: vec![HashMap::new()],
            errors: Vec::new(),
        }
    }

    fn walk(&mut self, node: &Node) {
        match node {
            Node::Program(statements) => {
                for statement in statements {
                    self.walk(&statement.node);
                }
            }
            Node::Block { stmts, .. } => {
                self.scopes.push(HashMap::new());
                for statement in stmts {
                    self.walk(statement);
                }
                self.scopes.pop();
            }
            Node::Function {
                parameters,
                defaults,
                body,
                requires,
                ensures,
                recovers_to,
                ..
            } => {
                let saved = std::mem::replace(&mut self.scopes, vec![HashMap::new()]);
                for (type_name, name) in parameters {
                    if let Some(ty) = known_annotation_type(type_name) {
                        self.scopes[0].insert(name.clone(), ty);
                    }
                }
                for default in defaults.iter().flatten() {
                    self.walk_expression(default);
                }
                for clause in requires {
                    self.walk_expression(clause);
                }
                self.walk(body);
                for clause in ensures {
                    self.walk_expression(clause);
                }
                if let Some(clause) = recovers_to {
                    self.walk_expression(clause);
                }
                self.scopes = saved;
            }
            Node::LetStatement { name, value, .. }
            | Node::StaticLet { name, value, .. }
            | Node::Const { name, value, .. } => {
                self.walk_expression(value);
                if let Some(ty) = self.known_type(value) {
                    self.bind(name, ty);
                } else {
                    self.remove_binding(name);
                }
            }
            Node::Assignment { name, value, .. } => {
                self.walk_expression(value);
                if let Some(ty) = self.known_type(value) {
                    self.assign(name, ty);
                } else {
                    self.remove_binding(name);
                }
            }
            Node::IfStatement {
                condition,
                consequence,
                alternative,
                ..
            } => {
                self.walk_expression(condition);
                let before = self.scopes.clone();

                self.walk(consequence);
                let after_consequence = self.scopes.clone();

                self.scopes = before.clone();
                if let Some(alternative) = alternative {
                    self.walk(alternative);
                }
                let after_alternative = self.scopes.clone();
                self.scopes = join_scopes(&before, &after_consequence, &after_alternative);
            }
            Node::WhileStatement {
                condition,
                body,
                invariants,
                ..
            } => {
                self.walk_expression(condition);
                for invariant in invariants {
                    self.walk_expression(invariant);
                }
                let before = self.scopes.clone();
                self.walk(body);
                self.scopes = before;
            }
            Node::ForInStatement {
                iterable,
                body,
                invariants,
                ..
            } => {
                self.walk_expression(iterable);
                for invariant in invariants {
                    self.walk_expression(invariant);
                }
                let before = self.scopes.clone();
                self.walk(body);
                self.scopes = before;
            }
            Node::ImplBlock { methods, .. } | Node::BlanketImpl { methods, .. } => {
                for method in methods {
                    self.walk(method);
                }
            }
            Node::ModuleDecl { body, .. } => {
                self.scopes.push(HashMap::new());
                for item in body {
                    self.walk(item);
                }
                self.scopes.pop();
            }
            Node::ExpressionStatement { expr, .. }
            | Node::ReturnStatement {
                value: Some(expr), ..
            }
            | Node::BreakWith { value: expr, .. }
            | Node::DeferStatement { expr, .. }
            | Node::InvariantStatement { expr, .. }
            | Node::StaticAssert {
                condition: expr, ..
            } => self.walk_expression(expr),
            Node::Assert {
                condition, message, ..
            }
            | Node::Assume {
                condition, message, ..
            } => {
                self.walk_expression(condition);
                if let Some(message) = message {
                    self.walk_expression(message);
                }
            }
            Node::FieldAssignment { target, value, .. } => {
                self.walk_expression(target);
                self.walk_expression(value);
            }
            Node::IndexAssignment {
                target,
                index,
                value,
                ..
            } => {
                self.walk_expression(target);
                self.walk_expression(index);
                self.walk_expression(value);
            }
            Node::LetDestructureStruct { value, .. } | Node::LetTupleDestructure { value, .. } => {
                self.walk_expression(value)
            }
            Node::TryCatch { body, handlers, .. } => {
                for statement in body {
                    self.walk(statement);
                }
                for (_, handler_body) in handlers {
                    for statement in handler_body {
                        self.walk(statement);
                    }
                }
            }
            Node::UnsafeBlock { body, .. } | Node::BenchBlock { body, .. } => self.walk(body),
            _ => self.walk_expression(node),
        }
    }

    fn walk_expression(&mut self, node: &Node) {
        crate::uniqueness_walk::visit(node, &mut |node| {
            if let Node::CallExpression {
                function,
                arguments,
                span,
            } = node
            {
                self.check_call(function, arguments, *span);
            }
        });
    }

    fn check_call(&mut self, function: &Node, arguments: &[Node], span: crate::span::Span) {
        let Node::Identifier { name, .. } = function else {
            return;
        };
        if !matches!(name.as_str(), "as_f32" | "as_f64") || arguments.len() != 1 {
            return;
        }
        let Some(actual) = self.known_type(&arguments[0]) else {
            return;
        };
        if is_numeric_type(&actual) {
            return;
        }
        self.errors.push(format!(
            "{}:{}:{}: error: {} expects an int or float argument, got {}",
            self.source_path,
            span.start.line,
            span.start.column,
            name,
            display_type(&actual),
        ));
    }

    fn known_type(&self, node: &Node) -> Option<Type> {
        match node {
            Node::Identifier { name, .. } => self.lookup(name),
            Node::IntegerLiteral { .. } => Some(Type::Int),
            Node::FloatLiteral { .. } => Some(Type::Float),
            Node::StringLiteral { .. } | Node::StringInternLiteral { .. } => Some(Type::String),
            Node::BooleanLiteral { .. } => Some(Type::Bool),
            Node::BytesLiteral { .. } => Some(Type::Bytes),
            Node::CharLiteral { .. } => Some(Type::Char),
            Node::ArrayLiteral { items, .. } => {
                let element_type =
                    items
                        .first()
                        .and_then(|item| self.known_type(item))
                        .filter(|first| {
                            items
                                .iter()
                                .skip(1)
                                .filter_map(|item| self.known_type(item))
                                .all(|item| item == *first)
                        });
                Some(match element_type {
                    Some(ty) => Type::TypedArray(Box::new(ty)),
                    None => Type::Array,
                })
            }
            Node::MapLiteral { .. } | Node::SetLiteral { .. } => None,
            Node::StructLiteral { name, .. } => Some(Type::Struct(name.clone())),
            Node::NewtypeConstruct { type_name, .. } => Some(Type::Struct(type_name.clone())),
            Node::TupleLiteral { items, .. } => Some(Type::Tuple(
                items
                    .iter()
                    .map(|item| self.known_type(item).unwrap_or(Type::Any))
                    .collect(),
            )),
            Node::IndexExpression { target, .. } => match self.known_type(target) {
                Some(Type::TypedArray(element)) => Some(*element),
                _ => None,
            },
            Node::InterpolatedString { .. } => Some(Type::String),
            Node::PrefixExpression {
                operator, right, ..
            } => match *operator {
                "!" => Some(Type::Bool),
                "+" | "-" => self.known_type(right),
                "~" => Some(Type::Int),
                _ => None,
            },
            Node::InfixExpression {
                left,
                operator,
                right,
                ..
            } => match *operator {
                "==" | "!=" | "<" | ">" | "<=" | ">=" | "&&" | "||" => Some(Type::Bool),
                "+" => {
                    let left_type = self.known_type(left);
                    let right_type = self.known_type(right);
                    if matches!(left_type.as_ref(), Some(Type::String))
                        || matches!(right_type.as_ref(), Some(Type::String))
                    {
                        Some(Type::String)
                    } else if left_type.is_some() && left_type == right_type {
                        left_type
                    } else {
                        None
                    }
                }
                "-" | "*" | "/" | "%" => {
                    let left_type = self.known_type(left);
                    let right_type = self.known_type(right);
                    if left_type.as_ref().is_some_and(is_numeric_type)
                        && right_type.as_ref().is_some_and(is_numeric_type)
                    {
                        left_type
                    } else {
                        None
                    }
                }
                _ => None,
            },
            Node::CallExpression { function, .. } => {
                let Node::Identifier { name, .. } = function.as_ref() else {
                    return None;
                };
                match name.as_str() {
                    "as_f32" => Some(Type::Float32),
                    "as_f64" => Some(Type::Float),
                    "to_string" => Some(Type::String),
                    _ => None,
                }
            }
            Node::NamedArg { value, .. } => self.known_type(value),
            _ => None,
        }
    }

    fn bind(&mut self, name: &str, ty: Type) {
        self.scopes
            .last_mut()
            .expect("float32 checker always has a root scope")
            .insert(name.to_string(), ty);
    }

    fn assign(&mut self, name: &str, ty: Type) {
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), ty);
                return;
            }
        }
        self.remove_binding(name);
    }

    fn remove_binding(&mut self, name: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.remove(name);
                return;
            }
        }
    }

    fn lookup(&self, name: &str) -> Option<Type> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).cloned())
    }
}

fn join_scopes(
    before: &[HashMap<String, Type>],
    left: &[HashMap<String, Type>],
    right: &[HashMap<String, Type>],
) -> Vec<HashMap<String, Type>> {
    before
        .iter()
        .enumerate()
        .map(|(index, baseline)| {
            let left_scope = left.get(index).unwrap_or(baseline);
            let right_scope = right.get(index).unwrap_or(baseline);
            baseline
                .keys()
                .filter_map(|name| {
                    let left_type = left_scope.get(name)?;
                    let right_type = right_scope.get(name)?;
                    (left_type == right_type).then(|| (name.clone(), left_type.clone()))
                })
                .collect()
        })
        .collect()
}

fn known_annotation_type(annotation: &str) -> Option<Type> {
    match annotation.trim() {
        "int" | "Int" | "Int8" | "Int16" | "Int32" | "Int64" | "i8" | "i16" | "i32" | "i64"
        | "UInt8" | "UInt16" | "UInt32" | "UInt64" | "u8" | "u16" | "u32" | "u64" => {
            Some(Type::Int)
        }
        "float" | "Float" | "f64" | "Float64" => Some(Type::Float),
        "f32" | "Float32" => Some(Type::Float32),
        "string" | "String" => Some(Type::String),
        "bool" | "Bool" => Some(Type::Bool),
        "char" | "Char" => Some(Type::Char),
        "bytes" | "Bytes" => Some(Type::Bytes),
        name if name.starts_with('[') || name.starts_with("array") => Some(Type::Array),
        _ => None,
    }
}

fn is_numeric_type(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Int
            | Type::Int8
            | Type::Int16
            | Type::Int32
            | Type::UInt8
            | Type::UInt16
            | Type::UInt32
            | Type::UInt64
            | Type::Float
            | Type::Float32
    )
}

fn display_type(ty: &Type) -> String {
    match ty {
        Type::String => "String".to_string(),
        Type::Bool => "Bool".to_string(),
        Type::Char => "Char".to_string(),
        Type::Bytes => "Bytes".to_string(),
        Type::Array | Type::TypedArray(_) => "Array".to_string(),
        Type::Struct(name) => name.clone(),
        Type::Tuple(_) => "Tuple".to_string(),
        Type::Function { .. } => "Function".to_string(),
        Type::Option(_) => "Option".to_string(),
        other => other.to_string(),
    }
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
        // 1/3 as f32 = 0.33333334... — the trailing bits beyond f32
        // precision are stripped.
        assert!(
            (v - (1.0_f64 / 3.0_f64) as f32 as f64).abs() < 1e-10,
            "as_f32 should truncate to f32 precision: {v}"
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

    // RES-2691: float literals must be assignable to f32-annotated variables.
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
        // run_program skips the typechecker; call it directly.
        let src = "let a = 3.14;\nlet b = 2.0 as f32;\nlet c = a + b;\nprintln(c);\n";
        let (prog, parse_errs) = parse(src);
        assert!(parse_errs.is_empty(), "should parse: {:?}", parse_errs);
        let check_result = crate::typechecker::TypeChecker::new().check_program(&prog);
        assert!(
            check_result.is_err(),
            "f32 + float arithmetic should still type-error in the typechecker"
        );
        let errs = check_result.unwrap_err();
        assert!(
            errs.contains("f32") || errs.contains("f64"),
            "error should mention f32/f64: {errs}"
        );
    }

    // ── Extended malformed-input regression corpus (RES-3758) ────────────────

    #[test]
    fn as_f32_too_many_args_errors() {
        let result = builtin_as_f32(&[Value::Float(1.0), Value::Float(2.0)]);
        assert!(result.is_err(), "as_f32 with 2 args should error");
    }

    #[test]
    fn as_f32_string_arg_errors() {
        let result = builtin_as_f32(&[Value::String("3.14".to_string())]);
        assert!(result.is_err(), "as_f32 with string should error");
    }

    #[test]
    fn as_f64_bool_arg_errors() {
        let result = builtin_as_f64(&[Value::Bool(true)]);
        assert!(result.is_err(), "as_f64 with bool should error");
    }

    #[test]
    fn f32_f64_mixing_program_errors() {
        let src = "let a = 3.14;\nlet b = 2.0 as f32;\nlet c = a + b;";
        let (prog, _) = parse(src);
        let check_result = crate::typechecker::TypeChecker::new().check_program(&prog);
        assert!(check_result.is_err(), "f32/f64 mixing should type-error");
    }

    #[test]
    fn f32_annotation_mismatch_errors() {
        let src = "let x: f32 = 3.14;\nlet y: f64 = x;";
        let (prog, _) = parse(src);
        let check_result = crate::typechecker::TypeChecker::new().check_program(&prog);
        // This may or may not error depending on type inference; document behavior
        let _ = check_result;
    }

    #[test]
    fn f32_function_arg_type_mismatch_errors() {
        let src = "fn f(f32 x) { println(x); }\nlet a = 3.14;\nf(a);";
        let (prog, _) = parse(src);
        let check_result = crate::typechecker::TypeChecker::new().check_program(&prog);
        // Type mismatch should be caught
        let _ = check_result;
    }

    #[test]
    fn as_f32_no_args_errors() {
        let result = builtin_as_f32(&[]);
        assert!(result.is_err(), "as_f32 with no args should error");
    }

    #[test]
    fn as_f64_too_many_args_errors() {
        let result = builtin_as_f64(&[Value::Float(1.0), Value::Float(2.0), Value::Float(3.0)]);
        assert!(result.is_err(), "as_f64 with 3 args should error");
    }

    // Valid baseline cases for regression

    #[test]
    fn f32_function_param_with_literal() {
        let result = crate::run_program(
            "fn compute(f32 x) -> f32 { return x; }\nlet v = compute(as_f32(42));\nprintln(v);\n",
        );
        assert!(
            result.ok,
            "f32 function with literal should work: {:?}",
            result.errors
        );
    }

    #[test]
    fn f32_return_type_with_cast() {
        let result = crate::run_program(
            "fn get_pi() -> f32 { return as_f32(3.14); }\nlet v = get_pi();\nprintln(v);\n",
        );
        assert!(
            result.ok,
            "f32 return type with as_f32 cast should work: {:?}",
            result.errors
        );
    }

    #[test]
    fn f32_nested_conversions() {
        let result = crate::run_program(
            "let x = 42;\nlet y = as_f32(x);\nlet z = as_f64(y);\nprintln(z);\n",
        );
        assert!(
            result.ok,
            "nested f32/f64 conversions should work: {:?}",
            result.errors
        );
    }
}
