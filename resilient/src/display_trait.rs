//! RES-2604: `Display` trait for custom string formatting.
//!
//! When a struct implements `Display`:
//!
//! ```text
//! trait Display { fn fmt(self) -> string; }
//!
//! struct Point {
//!     int x,
//!     int y,
//! }
//!
//! impl Display for Point {
//!     fn fmt(self) -> string {
//!         "(" + to_string(self.x) + ", " + to_string(self.y) + ")"
//!     }
//! }
//!
//! let p = new Point { x: 1, y: 2 };
//! println(to_string(p));  // prints "(1, 2)"
//! ```
//!
//! The `to_string` builtin is extended so that when its argument is a
//! `Value::Struct` and that struct has a `<StructName>$fmt` function in
//! scope (placed there by `impl Display for <StructName>`), the runtime
//! calls `fmt` on the instance and returns the resulting string.
//!
//! ## Typecheck pass
//!
//! `check(program, source_path)` validates every `impl Display for T` block:
//! - The `fmt` method must have exactly one parameter (`self`).
//! - The `fmt` method must declare a `string` return type.
//!
//! Both errors are surfaced as compiler diagnostics (line:col:) before
//! the interpreter runs.

use crate::Node;
use crate::span::Span;

/// Typecheck pass for `impl Display for T` blocks.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return Ok(()),
    };

    let mut errors: Vec<String> = Vec::new();

    for spanned in stmts {
        collect_display_errors(&spanned.node, source_path, &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn collect_display_errors(node: &Node, source_path: &str, errors: &mut Vec<String>) {
    match node {
        Node::ImplBlock {
            trait_name: Some(trait_nm),
            struct_name,
            methods,
            span,
            ..
        } if trait_nm == "Display" => {
            validate_display_impl(struct_name, methods, source_path, *span, errors);
        }
        Node::ModuleDecl { body, .. } => {
            for stmt in body {
                collect_display_errors(stmt, source_path, errors);
            }
        }
        _ => {}
    }
}

fn fmt_loc(source_path: &str, span: Span) -> String {
    if span.start.line == 0 {
        source_path.to_string()
    } else {
        format!("{}:{}:{}", source_path, span.start.line, span.start.column)
    }
}

fn validate_display_impl(
    struct_name: &str,
    methods: &[Node],
    source_path: &str,
    span: Span,
    errors: &mut Vec<String>,
) {
    let mangled_fmt = format!("{}$fmt", struct_name);
    let loc = fmt_loc(source_path, span);

    let fmt_method = methods.iter().find(|m| {
        if let Node::Function { name, .. } = m {
            name == &mangled_fmt
        } else {
            false
        }
    });

    let Some(fmt_node) = fmt_method else {
        errors.push(format!(
            "{}: impl Display for `{}` must define a `fmt(self) -> string` method",
            loc, struct_name
        ));
        return;
    };

    if let Node::Function {
        parameters,
        return_type,
        ..
    } = fmt_node
    {
        if parameters.len() != 1 {
            errors.push(format!(
                "{}: `Display::fmt` on `{}` must have exactly one parameter (`self`), got {}",
                loc,
                struct_name,
                parameters.len()
            ));
        }

        let ret = return_type.as_deref().unwrap_or("void");
        if !ret.eq_ignore_ascii_case("string") {
            errors.push(format!(
                "{}: `Display::fmt` on `{}` must return `string`, found `{}`",
                loc, struct_name, ret
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Runtime helper — called from `to_string` dispatch in lib.rs.
// ---------------------------------------------------------------------------

/// Attempt to call `<struct_name>$fmt(val)` through the interpreter.
/// Returns `None` if no Display impl exists for the struct (caller falls back).
pub(crate) fn try_display_fmt(
    interp: &mut crate::Interpreter,
    val: crate::Value,
) -> Option<crate::RResult<crate::Value>> {
    let struct_name = match &val {
        crate::Value::Struct { name, .. } => name.clone(),
        _ => return None,
    };

    let mangled = format!("{}$fmt", struct_name);
    let method_val = interp.env.get(&mangled)?;

    Some(interp.apply_function(&method_val, vec![val]))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::{Lexer, Parser, run_program};

    fn extract_stdout(result: crate::RunResult) -> String {
        result.stdout.trim_end().to_string()
    }

    fn parse_src(src: &str) -> crate::Node {
        let lexer = Lexer::new(src);
        let mut parser = Parser::new(lexer);
        parser.parse_program()
    }

    #[test]
    fn display_to_string_basic() {
        let src = r#"
trait Display { fn fmt(self) -> string; }
struct Point {
    int x,
    int y,
}
impl Display for Point {
    fn fmt(self) -> string {
        "(" + to_string(self.x) + ", " + to_string(self.y) + ")"
    }
}
let p = new Point { x: 3, y: 7 };
println(to_string(p));
"#;
        let out = extract_stdout(run_program(src));
        assert_eq!(out, "(3, 7)", "got: {}", out);
    }

    #[test]
    fn display_to_string_string_field() {
        let src = r#"
trait Display { fn fmt(self) -> string; }
struct Named {
    string name,
}
impl Display for Named {
    fn fmt(self) -> string {
        "Named(" + self.name + ")"
    }
}
let n = new Named { name: "hello" };
println(to_string(n));
"#;
        let out = extract_stdout(run_program(src));
        assert_eq!(out, "Named(hello)", "got: {}", out);
    }

    #[test]
    fn display_method_call_fmt_directly() {
        let src = r#"
trait Display { fn fmt(self) -> string; }
struct Color {
    int r,
    int g,
    int b,
}
impl Display for Color {
    fn fmt(self) -> string {
        "rgb(" + to_string(self.r) + "," + to_string(self.g) + "," + to_string(self.b) + ")"
    }
}
let c = new Color { r: 255, g: 0, b: 128 };
println(c.fmt());
"#;
        let out = extract_stdout(run_program(src));
        assert_eq!(out, "rgb(255,0,128)", "got: {}", out);
    }

    #[test]
    fn display_no_impl_falls_back_to_error() {
        let src = r#"
struct Blob {
    int data,
}
let b = new Blob { data: 42 };
println(to_string(b));
"#;
        let result = run_program(src);
        assert!(!result.errors.is_empty() || result.stdout.is_empty());
    }

    #[test]
    fn display_typecheck_missing_fmt() {
        let src = r#"
trait Display { fn fmt(self) -> string; }
struct Bad {
    int x,
}
impl Display for Bad {}
"#;
        let prog = parse_src(src);
        let err = super::check(&prog, "test.rz").expect_err("expected error for missing fmt");
        assert!(
            err.contains("fmt"),
            "error should mention 'fmt', got: {err}"
        );
    }

    #[test]
    fn display_typecheck_wrong_return_type() {
        let src = r#"
trait Display { fn fmt(self) -> string; }
struct Bad {
    int x,
}
impl Display for Bad {
    fn fmt(self) -> int { 42 }
}
"#;
        let prog = parse_src(src);
        let err = super::check(&prog, "test.rz").expect_err("expected error for wrong return type");
        assert!(
            err.contains("string") || err.contains("int"),
            "error should mention the return type, got: {err}"
        );
    }
}
