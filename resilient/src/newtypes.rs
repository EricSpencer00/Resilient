//! RES-319: opaque single-field type wrappers — `newtype Meters = Float;`.
//!
//! Type aliases (RES-296) make a name *transparent*: `Meters` and `float`
//! unify everywhere. That is fine for documentation but useless for
//! safety-critical unit safety, where you want the compiler to reject
//! `speed(seconds, meters)` even though both arguments are numerically
//! identical.
//!
//! A `newtype` declaration introduces a *nominal* wrapper:
//!
//! ```text
//! newtype Meters = Float;
//! newtype Volts  = Float;
//!
//! let m: Meters = Meters(3.14);   // construction wraps a Float
//! let v: Volts  = Volts(5.0);
//! let bad: Meters = v;            // type error — Volts is not Meters
//! ```
//!
//! The wrapped value is opaque to ordinary expressions — a `Meters`
//! does NOT participate in arithmetic with a bare `Float`. Arithmetic
//! between two values of the *same* newtype is allowed and preserves
//! the brand: `Meters + Meters -> Meters`, `Meters - Meters -> Meters`,
//! etc. Crossing brands is a hard error: `Meters + Volts` is rejected
//! with a diagnostic that names both types.
//!
//! ## What this module owns
//!
//! - [`check`]: the program-level pass invoked from `<EXTENSION_PASSES>`
//!   in `typechecker.rs`. Walks every top-level `Node::Newtype`,
//!   builds the `name -> base` table on the typechecker, and rejects
//!   - duplicate newtype names,
//!   - self-wrapping newtypes,
//!   - newtype targets whose base type does not resolve.
//! - [`is_newtype`]: pure predicate over the typechecker's table —
//!   consulted from `Node::CallExpression` to decide whether a
//!   bare-identifier callee is a newtype constructor.
//! - [`construct_type`]: returns the nominal `Type::Struct(name)` that a
//!   `Name(arg)` call produces, after verifying the argument matches the
//!   declared base type.
//! - [`infix_result`]: returns the result type of a binary arithmetic
//!   operator on two newtype operands. `Same -> Same`, distinct names
//!   produce a diagnostic.
//!
//! ## What lives in core files (and why)
//!
//! - **Token + keyword + AST node**: `main.rs` (`Token::Newtype`,
//!   `"newtype" => Token::Newtype`, `Node::Newtype { name, target,
//!   span }`) — placed in the `<EXTENSION_*>` blocks to minimise the
//!   merge surface against parallel agents.
//! - **Logos token**: `lexer_logos.rs` (`#[token("newtype")] Newtype`)
//!   — same `<EXTENSION_TOKENS>` block.
//! - **Parser**: `Parser::parse_newtype_decl` in `main.rs`. Mirrors
//!   the `parse_type_alias` shape so the keyword-driven dispatch in
//!   `parse_program_statement` is one extra arm.
//! - **Top-level dispatch**: `Token::Newtype => Some(parse_newtype_decl)`
//!   alongside the existing `Token::Type` arm.
//! - **Constructor / arithmetic plumbing**: `typechecker.rs`'s
//!   `Node::CallExpression` arm calls [`construct_type`] before the
//!   normal callable lookup, and `Node::InfixExpression` consults
//!   [`infix_result`] when one operand is a `Type::Struct(name)` that
//!   resolves to a registered newtype.
//!
//! ## Why we don't need a `Value::Newtype` variant
//!
//! Newtypes are *erased* at runtime. The wrapped value carries no
//! extra tag because the type checker has already proved the program
//! never confuses two newtypes — by the time `eval` runs, every use
//! site has been validated. A constructor call `Meters(3.14)` is
//! lowered to "evaluate the argument and return it"; arithmetic on
//! newtypes is lowered to arithmetic on the wrapped values. This
//! keeps the interpreter, JIT, and embedded runtime untouched.
//!
//! ## Failure modes the pass surfaces
//!
//! - `newtype A = Unknown;` where `Unknown` is neither a builtin
//!   primitive, a registered struct, nor a registered alias →
//!   `unknown base type for newtype A: Unknown`.
//! - Two `newtype Foo = ...` declarations → `duplicate newtype Foo`.
//! - `newtype A = A;` (self-referential) → `newtype A cannot wrap
//!   itself`. (Fundamentally unprintable: a newtype's whole point is
//!   adding nominal identity to *another* type.)
//!
//! Errors carry `<source>:<line>:<col>` prefixes, matching the
//! type-alias module's diagnostic style.

use crate::Node;
use crate::typechecker::{Type, TypeChecker};

/// RES-319: top-level pass — validate every `newtype X = T;` and reject
/// the structural failure modes (duplicate name, self-wrapping, unknown
/// base type).
///
/// The typechecker's `check_program_with_source` already hoists every
/// `Node::Newtype` into the `newtypes` table during its declaration
/// pre-pass, so by the time this pass runs the table is populated.
/// We re-walk the AST to:
///
/// 1. Reject self-wraps (`newtype A = A;`) eagerly.
/// 2. Reject duplicates by tracking the names we've validated so far.
/// 3. Verify each target type resolves through `parse_type_name`.
pub(crate) fn check(tc: &mut TypeChecker, program: &Node, source_path: &str) -> Result<(), String> {
    let Node::Program(statements) = program else {
        return Ok(());
    };

    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for spanned in statements {
        if let Node::Newtype { name, target, span } = &spanned.node {
            if name.is_empty() {
                // Parser recovery already emitted a diagnostic; skip.
                continue;
            }

            // Self-wrap is meaningless — `newtype A = A;` would have to
            // resolve `A` to itself, which loops at the typechecker.
            if name == target {
                return Err(format!(
                    "{}:{}:{}: newtype {} cannot wrap itself",
                    source_path, span.start.line, span.start.column, name
                ));
            }

            if !seen.insert(name.clone()) {
                return Err(format!(
                    "{}:{}:{}: duplicate newtype {}",
                    source_path, span.start.line, span.start.column, name
                ));
            }

            // The target must resolve to a known type. We delegate to
            // the typechecker's `parse_type_name`; aliases and other
            // newtypes are already registered by the time this pass
            // runs (the hoist pass populates both maps), so
            // `newtype Foo = MyAlias;` and `newtype Inches = Meters;`
            // both work.
            tc.parse_type_name_pub(target).map_err(|e| {
                format!(
                    "{}:{}:{}: unknown base type for newtype {}: {} ({})",
                    source_path, span.start.line, span.start.column, name, target, e
                )
            })?;

            // The hoist pass already inserted; ensure the registration
            // matches what we just validated (a no-op when the input
            // is well-formed, but keeps the API symmetric).
            tc.newtypes_insert(name.clone(), target.clone());
        }
    }

    Ok(())
}

/// RES-319: predicate over the typechecker's newtype table. The
/// `Node::CallExpression` arm calls this on a bare-identifier callee
/// to decide whether to treat the call as a newtype constructor.
pub(crate) fn is_newtype(tc: &TypeChecker, name: &str) -> bool {
    tc.newtypes_get(name).is_some()
}

/// RES-319: validate a `Name(arg)` constructor call and return the
/// nominal newtype as `Type::Struct(name)`. The single argument must
/// type-check against the declared base type.
///
/// The caller is responsible for having already verified that
/// `is_newtype(tc, name)` returned true; this function panics in
/// debug builds if the name is not registered.
pub(crate) fn construct_type(
    tc: &mut TypeChecker,
    name: &str,
    arguments: &[Node],
) -> Result<Type, String> {
    let target = tc.newtypes_get(name).cloned().expect(
        "construct_type called for non-newtype identifier — caller must check is_newtype first",
    );

    if arguments.len() != 1 {
        return Err(format!(
            "newtype {} constructor takes exactly 1 argument, got {}",
            name,
            arguments.len()
        ));
    }

    let arg_type = tc.check_node(&arguments[0])?;
    let expected = tc.parse_type_name_pub(&target)?;

    // Reuse the typechecker's relaxed compatibility rule (Any wildcard,
    // integer-literal coercion to pinned widths). This matches what
    // ordinary fn calls accept for the same parameter type.
    if !crate::typechecker::compatible_pub(&arg_type, &expected) {
        return Err(format!(
            "newtype {} expects a {} argument, got {}",
            name, expected, arg_type
        ));
    }

    Ok(Type::Struct(name.to_string()))
}

/// RES-319: compute the result type of a binary arithmetic op when at
/// least one operand is a registered newtype.
///
/// Returns:
/// - `Some(Ok(Type::Struct(name)))` when both sides are the same
///   newtype — preserve the brand.
/// - `Some(Err(...))` when the two sides are different newtypes, or a
///   newtype is mixed with a non-newtype scalar — the brand is opaque,
///   so users must explicitly construct one side.
/// - `None` when neither side is a newtype — the caller's normal
///   numeric-compatibility check handles it.
pub(crate) fn infix_result(
    tc: &TypeChecker,
    operator: &str,
    left: &Type,
    right: &Type,
) -> Option<Result<Type, String>> {
    let l_nt = newtype_name(left).filter(|n| is_newtype(tc, n));
    let r_nt = newtype_name(right).filter(|n| is_newtype(tc, n));

    match (l_nt, r_nt) {
        (None, None) => None,
        (Some(l), Some(r)) if l == r => {
            // Same brand on both sides — the result inherits the brand.
            // Comparisons (`<`, `==`, etc.) still need a Bool; the
            // caller filters on operator before reaching here, so any
            // operator we see is a value-returning arithmetic op.
            Some(Ok(Type::Struct(l.to_string())))
        }
        (Some(l), Some(r)) => Some(Err(format!(
            "cannot apply '{}' to mixed newtypes {} and {} — wrap one side in the other's constructor",
            operator, l, r
        ))),
        (Some(l), None) => Some(Err(format!(
            "cannot apply '{}' to newtype {} and bare {} — wrap the right operand in {}(...)",
            operator, l, right, l
        ))),
        (None, Some(r)) => Some(Err(format!(
            "cannot apply '{}' to bare {} and newtype {} — wrap the left operand in {}(...)",
            operator, left, r, r
        ))),
    }
}

fn newtype_name(t: &Type) -> Option<&str> {
    match t {
        Type::Struct(n) => Some(n.as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn run_full_check(src: &str) -> Result<(), String> {
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = TypeChecker::new();
        tc.check_program_with_source(&program, "<test>").map(|_| ())
    }

    #[test]
    fn parse_simple_newtype() {
        let src = "newtype Meters = float;\n";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let Node::Program(stmts) = program else {
            panic!("expected program");
        };
        let nt = stmts.iter().find_map(|s| match &s.node {
            Node::Newtype { name, target, .. } => Some((name.clone(), target.clone())),
            _ => None,
        });
        assert_eq!(nt, Some(("Meters".to_string(), "float".to_string())));
    }

    #[test]
    fn newtype_construction_typechecks() {
        let src = "\
            newtype Meters = float;\n\
            let m: Meters = Meters(3.14);\n\
        ";
        run_full_check(src).expect("constructor with matching arg type must check");
    }

    #[test]
    fn cross_newtype_assignment_is_rejected() {
        // The two newtypes share the same base (Float) but the
        // typechecker must keep them distinct. Assigning a Volts where
        // a Meters is declared is the canonical unit-safety bug.
        let src = "\
            newtype Meters = float;\n\
            newtype Volts = float;\n\
            let v: Volts = Volts(5.0);\n\
            let m: Meters = v;\n\
        ";
        let err = run_full_check(src).expect_err("cross-newtype assignment must be rejected");
        // Diagnostic must mention both type names so the user can
        // diagnose the unit confusion.
        assert!(
            err.contains("Meters") && err.contains("Volts"),
            "diagnostic must name both newtypes, got: {}",
            err
        );
    }

    #[test]
    fn cross_newtype_call_argument_is_rejected() {
        let src = "\
            newtype Meters = float;\n\
            newtype Volts = float;\n\
            fn travel(Meters d) -> Meters { return d; }\n\
            let v: Volts = Volts(2.0);\n\
            let bad = travel(v);\n\
        ";
        let err = run_full_check(src).expect_err("Volts -> Meters must be rejected at the call");
        assert!(
            err.contains("Meters") && err.contains("Volts"),
            "got: {}",
            err
        );
    }

    #[test]
    fn newtype_constructor_arg_type_is_checked() {
        // `Meters` wraps a Float — passing a String is a hard type
        // error at the constructor call site.
        let src = "\
            newtype Meters = float;\n\
            let bad: Meters = Meters(\"hi\");\n\
        ";
        let err = run_full_check(src).expect_err("string into Meters(float) must be rejected");
        assert!(
            err.contains("Meters") || err.contains("float"),
            "got: {}",
            err
        );
    }

    #[test]
    fn same_brand_arithmetic_preserves_brand() {
        // Two Meters values can be added; the result is still Meters,
        // so the parameter binding still type-checks.
        let src = "\
            newtype Meters = float;\n\
            let a: Meters = Meters(1.0);\n\
            let b: Meters = Meters(2.0);\n\
            let c: Meters = a + b;\n\
        ";
        run_full_check(src).expect("Meters + Meters must return Meters");
    }

    #[test]
    fn cross_brand_arithmetic_is_rejected() {
        let src = "\
            newtype Meters = float;\n\
            newtype Volts = float;\n\
            let m: Meters = Meters(1.0);\n\
            let v: Volts = Volts(2.0);\n\
            let bad = m + v;\n\
        ";
        let err = run_full_check(src).expect_err("Meters + Volts must be rejected");
        assert!(
            err.contains("Meters") && err.contains("Volts"),
            "got: {}",
            err
        );
    }

    #[test]
    fn duplicate_newtype_is_rejected() {
        let src = "\
            newtype Meters = float;\n\
            newtype Meters = float;\n\
        ";
        let err = run_full_check(src).expect_err("duplicate newtype must be a hard error");
        assert!(err.contains("duplicate newtype"), "got: {}", err);
    }

    #[test]
    fn self_wrap_is_rejected() {
        let src = "newtype A = A;\n";
        let err = run_full_check(src).expect_err("self-wrap must be a hard error");
        assert!(err.contains("cannot wrap itself"), "got: {}", err);
    }

    #[test]
    fn newtype_diagnostic_carries_source_position() {
        let src = "\
            newtype Meters = float;\n\
            newtype Meters = float;\n\
        ";
        let err = run_full_check(src).expect_err("duplicate");
        assert!(err.starts_with("<test>:"), "got: {}", err);
    }
}
