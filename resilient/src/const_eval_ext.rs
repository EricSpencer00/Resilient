//! RES-2580: extended compile-time constant evaluation.
//!
//! Extends `Interpreter::eval_const_expr` with the following forms that
//! were previously rejected with "not a valid constant expression":
//!
//! - **String concatenation**: `const GREETING = "Hello, " + NAME;`
//! - **String ordering**: `const OK = "alpha" < "beta";`
//! - **Bitwise operators**: `const MASK = FLAGS & 0xFF;`, `|`, `^`, `<<`, `>>`
//! - **Conditional expressions**: `const MAX = if A > B { A } else { B };`
//! - **Single-expression blocks**: `const X = { 1 + 2 };`
//! - **Tuple literals**: `const PAIR = (1, 2);`
//!
//! All new cases live in `Interpreter::eval_const_expr` in `lib.rs`.
//! This module provides the typecheck registration hook (no-op — the
//! extension is in the evaluator, not the typechecker) and unit tests.

use crate::Node;

/// No-op typecheck pass — the extension is purely in the const evaluator.
/// Registered in `<EXTENSION_PASSES>` to give const_eval_ext a CI-visible
/// footprint and to allow future validation to be added here.
pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    Ok(())
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

    fn run_expect_err(src: &str) -> String {
        let r = run_program(src);
        assert!(!r.ok, "expected error but program succeeded");
        r.errors.join("\n")
    }

    #[test]
    fn const_string_concat() {
        let out = run(r#"
const FIRST = "Hello";
const REST = ", world";
const FULL = FIRST + REST;
println(FULL);
"#);
        assert!(out.contains("Hello, world"), "got: {out:?}");
    }

    #[test]
    fn const_string_ordering() {
        let out = run(r#"
const A = "alpha";
const B = "beta";
const ORDERED = A < B;
println(to_string(ORDERED));
"#);
        assert!(out.contains("true"), "got: {out:?}");
    }

    #[test]
    fn const_bitwise_and() {
        let out = run(r#"
const FLAGS = 0xFF;
const MASK = 0x0F;
const LOWER = FLAGS & MASK;
println(to_string(LOWER));
"#);
        assert!(out.contains("15"), "got: {out:?}");
    }

    #[test]
    fn const_bitwise_or() {
        let out = run(r#"
const A = 0b1010;
const B = 0b0101;
const C = A | B;
println(to_string(C));
"#);
        assert!(out.contains("15"), "got: {out:?}");
    }

    #[test]
    fn const_bitwise_xor() {
        let out = run(r#"
const A = 0xFF;
const B = 0xF0;
const C = A ^ B;
println(to_string(C));
"#);
        assert!(out.contains("15"), "got: {out:?}");
    }

    #[test]
    fn const_shift() {
        let out = run(r#"
const BASE = 1;
const SHIFTED = BASE << 4;
println(to_string(SHIFTED));
"#);
        assert!(out.contains("16"), "got: {out:?}");
    }

    #[test]
    fn const_conditional_true_branch() {
        let out = run(r#"
const A = 10;
const B = 5;
const MAX = if A > B { A } else { B };
println(to_string(MAX));
"#);
        assert!(out.contains("10"), "got: {out:?}");
    }

    #[test]
    fn const_conditional_false_branch() {
        let out = run(r#"
const A = 3;
const B = 7;
const MAX = if A > B { A } else { B };
println(to_string(MAX));
"#);
        assert!(out.contains("7"), "got: {out:?}");
    }

    #[test]
    fn const_tuple() {
        let out = run(r#"
const PAIR = (1, 2);
let (a, b) = PAIR;
println(to_string(a + b));
"#);
        assert!(out.contains("3"), "got: {out:?}");
    }

    #[test]
    fn const_circular_reference_errors() {
        let err = run_expect_err("const X = X;");
        assert!(
            err.contains("circular"),
            "expected circular error, got: {err:?}"
        );
    }
}
