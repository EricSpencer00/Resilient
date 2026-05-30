//! RES-2577: Tuple structs with named constructors.
//!
//! When a tuple struct is declared — `struct Point(int, int);` — the
//! interpreter registers a constructor function `Point` so that callers
//! can write `Point(3, 4)` instead of the more verbose `new Point(3, 4)`.
//!
//! ## Syntax
//! ```text
//! struct Point(int, int);     // declaration
//! let p = Point(3, 4);        // constructor call (NEW — no `new` keyword)
//! let x = p.0;                // positional field access
//! let y = p.1;
//! ```
//!
//! The constructor call is syntactic sugar: `Point(3, 4)` is equivalent
//! to `new Point(3, 4)`, both producing `Value::Struct { name: "Point",
//! fields: { "0": 3, "1": 4 } }`.
//!
//! ## Detection
//!
//! A `StructDecl` is a tuple struct when ALL of its fields are named with
//! consecutive non-negative decimal integers ("0", "1", "2", ...). Named-
//! field structs (`struct Pt { int x, int y }`) have alphabetic field names
//! and are unaffected.
//!
//! ## Registration
//!
//! `register_constructor` is called by the `StructDecl` eval branch in
//! `lib.rs`. It inserts a `Value::Builtin` into the environment keyed by
//! the struct name. The builtin is a closure that captures the struct name
//! and field count; it validates the argument count at construction time
//! and builds the `Value::Struct` result.

use std::rc::Rc;

use crate::{Environment, FunctionValue, Node, Value, span::Span};

/// Returns true when `fields` are all consecutive positional names
/// ("0", "1", ..., "n-1") — the signature of a tuple struct declaration.
pub(crate) fn is_tuple_struct(fields: &[(String, String)]) -> bool {
    if fields.is_empty() {
        return false;
    }
    fields
        .iter()
        .enumerate()
        .all(|(i, (_, name))| name == &i.to_string())
}

/// Build a `Value::Function` constructor for a tuple struct.
///
/// The constructor is a synthetic function with `field_count` parameters
/// (named `__arg0`, `__arg1`, …) and a body that constructs a
/// `StructLiteral` with the appropriate positional fields.
///
/// The returned value should be stored in the interpreter environment
/// under `struct_name` so that `Point(3, 4)` resolves to this constructor.
pub(crate) fn make_constructor(struct_name: String, field_count: usize, env: Environment) -> Value {
    let params: Vec<(String, String)> = (0..field_count)
        .map(|i| ("auto".to_string(), format!("__arg{i}")))
        .collect();

    let fields: Vec<(String, Node)> = (0..field_count)
        .map(|i| {
            (
                i.to_string(),
                Node::Identifier {
                    name: format!("__arg{i}"),
                    span: Span::default(),
                },
            )
        })
        .collect();

    let body = Node::Block {
        stmts: vec![Node::StructLiteral {
            name: struct_name.clone(),
            fields,
            base: None,
            span: Span::default(),
        }],
        span: Span::default(),
    };

    Value::Function(Box::new(FunctionValue {
        parameters: Rc::new(params),
        body: Rc::new(body),
        env,
        requires: Vec::new(),
        ensures: Vec::new(),
        recovers_to: None,
        name: struct_name,
        type_params: Vec::new(),
        fails: Rc::new(Vec::new()),
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> String {
        let r = run_program(src);
        assert!(r.ok, "program failed: {:?}", r.errors);
        r.stdout
    }

    #[test]
    fn tuple_struct_constructor_works() {
        let out = run(r#"
struct Point(int, int);
let p = Point(3, 4);
println(to_string(p.0));
println(to_string(p.1));
"#);
        assert!(out.contains('3'), "expected 3, got: {out:?}");
        assert!(out.contains('4'), "expected 4, got: {out:?}");
    }

    #[test]
    fn tuple_struct_new_keyword_still_works() {
        let out = run(r#"
struct Pair(int, int);
let p = new Pair(10, 20);
println(to_string(p.0 + p.1));
"#);
        assert!(out.contains("30"), "expected 30, got: {out:?}");
    }

    #[test]
    fn named_struct_unaffected() {
        // Named-field structs are NOT given a positional constructor.
        let out = run(r#"
struct Color { int r, int g, int b }
let c = new Color { r: 255, g: 0, b: 128 };
println(to_string(c.r));
"#);
        assert!(out.contains("255"), "expected 255, got: {out:?}");
    }

    #[test]
    fn tuple_struct_three_fields() {
        let out = run(r#"
struct Triple(int, int, int);
let t = Triple(1, 2, 3);
println(to_string(t.0 + t.1 + t.2));
"#);
        assert!(out.contains('6'), "expected 6, got: {out:?}");
    }

    #[test]
    fn tuple_struct_used_in_function() {
        let out = run(r#"
struct Vec2(int, int);
fn length_sq(Vec2 v) -> int {
    v.0 * v.0 + v.1 * v.1
}
let v = Vec2(3, 4);
println(to_string(length_sq(v)));
"#);
        assert!(out.contains("25"), "expected 25 (3^2+4^2), got: {out:?}");
    }
}
