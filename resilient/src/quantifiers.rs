//! RES-330: `forall` and `exists` quantifier expressions.
//!
//! Two surface forms:
//! ```text
//! forall i in lo..hi: bool_expr     // bounded integer range, half-open [lo, hi)
//! exists x in iterable: bool_expr   // any iterable expression — array, set, range
//! ```
//!
//! Runtime: `forall` short-circuits on the first false witness; `exists`
//! short-circuits on the first true witness. Quantified variables do
//! not escape the body's scope.
//!
//! Z3 (with `--features z3`): the bounded-range form is encoded as a
//! universally / existentially quantified Int with the range constraint
//! as the antecedent. Iterable quantifiers are *not* discharged
//! statically — the caller falls back to the runtime check.

use crate::span;
use crate::typechecker::{Type, TypeChecker};
use crate::{Interpreter, Node, Parser, RResult, Token, Value};

/// Universal vs. existential quantifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantifierKind {
    /// `forall id in range: body` — body must hold for every witness.
    Forall,
    /// `exists id in range: body` — body must hold for at least one witness.
    Exists,
}

impl QuantifierKind {
    pub fn keyword(self) -> &'static str {
        match self {
            QuantifierKind::Forall => "forall",
            QuantifierKind::Exists => "exists",
        }
    }
}

/// Range / iterable an `id` is quantified over.
#[derive(Debug, Clone)]
pub enum QuantRange {
    /// `lo..hi` — half-open integer range. The Z3 backend recognizes
    /// this form; everything else falls back to runtime evaluation.
    Range { lo: Box<Node>, hi: Box<Node> },
    /// Any iterable expression (array, set, map keys, ...). Runtime
    /// only — Z3 cannot universally quantify over an arbitrary
    /// program value without first-class array theory bindings,
    /// which the verifier doesn't model yet.
    Iterable(Box<Node>),
}

// -------------------------------------------------------------------
// Parser
// -------------------------------------------------------------------

/// Parse `(forall|exists) IDENT in <range_expr>: <bool_expr>`.
///
/// On entry `parser.current_token` is `Forall` or `Exists`. On return
/// the parser leaves `current_token` pointing at the last token of
/// the body — matching the convention of every other prefix parser.
pub(crate) fn parse_quantifier(parser: &mut Parser) -> Option<Node> {
    let head_span = parser.span_at_current();
    let kind = match &parser.current_token {
        Token::Forall => QuantifierKind::Forall,
        Token::Exists => QuantifierKind::Exists,
        _ => return None,
    };
    parser.next_token(); // skip `forall` / `exists`

    let var = match &parser.current_token {
        Token::Identifier(name) => name.clone(),
        other => {
            let tok = other.clone();
            parser.record_error(format!(
                "Expected identifier after `{}`, found {}",
                kind.keyword(),
                tok
            ));
            return None;
        }
    };
    parser.next_token(); // skip identifier

    if parser.current_token != Token::In {
        let tok = parser.current_token.clone();
        parser.record_error(format!(
            "Expected `in` after `{} {}`, found {}",
            kind.keyword(),
            var,
            tok
        ));
        return None;
    }
    parser.next_token(); // skip `in`

    // Parse the range expression. We stop at `..` or `:` so the lower
    // bound doesn't accidentally consume the range separator. The
    // generic precedence-climbing parser folds `..` (precedence 0) at
    // its top level, so we can just call `parse_expression(0)` and
    // then look at the *current* token: if it's `..`, treat the
    // expression as the lower bound of a `lo..hi` range; otherwise
    // it's an iterable.
    let lo_or_iter = parser.parse_expression(0).unwrap_or(Node::IntegerLiteral {
        value: 0,
        span: head_span,
    });
    parser.next_token(); // step past tail of the parsed expression

    let range = if parser.current_token == Token::DotDot {
        parser.next_token(); // skip `..`
        let hi = parser.parse_expression(0).unwrap_or(Node::IntegerLiteral {
            value: 0,
            span: head_span,
        });
        parser.next_token(); // step past tail of hi
        QuantRange::Range {
            lo: Box::new(lo_or_iter),
            hi: Box::new(hi),
        }
    } else {
        QuantRange::Iterable(Box::new(lo_or_iter))
    };

    if parser.current_token != Token::Colon {
        let tok = parser.current_token.clone();
        parser.record_error(format!(
            "Expected `:` after `{} {} in <range>`, found {}",
            kind.keyword(),
            var,
            tok
        ));
        return None;
    }
    parser.next_token(); // skip `:`

    let body = parser.parse_expression(0).unwrap_or(Node::BooleanLiteral {
        value: true,
        span: head_span,
    });

    Some(Node::Quantifier {
        kind,
        var,
        range,
        body: Box::new(body),
        span: head_span,
    })
}

// -------------------------------------------------------------------
// Interpreter
// -------------------------------------------------------------------

/// Runtime evaluation. Iterates the range, binding `var` in a fresh
/// inner scope, and short-circuits per `kind`.
pub(crate) fn eval_quantifier(
    interp: &mut Interpreter,
    kind: QuantifierKind,
    var: &str,
    range: &QuantRange,
    body: &Node,
) -> RResult<Value> {
    let witnesses = collect_witnesses(interp, range)?;
    let saved_env = interp.env.clone();
    let inner_env = crate::Environment::new_enclosed(saved_env.clone());
    interp.env = inner_env;

    let mut result = match kind {
        QuantifierKind::Forall => true,
        QuantifierKind::Exists => false,
    };

    for witness in witnesses {
        interp.env.set(var.to_string(), witness);
        let val = interp.eval(body)?;
        let b = match val {
            Value::Bool(b) => b,
            other => {
                interp.env = saved_env;
                return Err(format!(
                    "{} body must evaluate to Bool, got {}",
                    kind.keyword(),
                    other
                ));
            }
        };
        match kind {
            QuantifierKind::Forall => {
                if !b {
                    result = false;
                    break;
                }
            }
            QuantifierKind::Exists => {
                if b {
                    result = true;
                    break;
                }
            }
        }
    }

    interp.env = saved_env;
    Ok(Value::Bool(result))
}

fn collect_witnesses(interp: &mut Interpreter, range: &QuantRange) -> RResult<Vec<Value>> {
    match range {
        QuantRange::Range { lo, hi } => {
            let lo_v = expect_int(interp.eval(lo)?, "range lower bound")?;
            let hi_v = expect_int(interp.eval(hi)?, "range upper bound")?;
            if hi_v < lo_v {
                return Ok(Vec::new());
            }
            let len = (hi_v - lo_v) as usize;
            let mut out = Vec::with_capacity(len);
            for i in lo_v..hi_v {
                out.push(Value::Int(i));
            }
            Ok(out)
        }
        QuantRange::Iterable(expr) => {
            let v = interp.eval(expr)?;
            iterable_witnesses(v)
        }
    }
}

fn iterable_witnesses(v: Value) -> RResult<Vec<Value>> {
    match v {
        Value::Array(items) => Ok(items),
        Value::Set(set) => Ok(set
            .into_iter()
            .map(|k| match k {
                crate::MapKey::Int(i) => Value::Int(i),
                crate::MapKey::Str(s) => Value::String(s),
                crate::MapKey::Bool(b) => Value::Bool(b),
            })
            .collect()),
        Value::Bytes(bs) => Ok(bs.into_iter().map(|b| Value::Int(b as i64)).collect()),
        other => Err(format!(
            "quantifier range must be a `lo..hi` range, array, set, or bytes — got {}",
            other
        )),
    }
}

fn expect_int(v: Value, what: &str) -> RResult<i64> {
    match v {
        Value::Int(i) => Ok(i),
        other => Err(format!("{} must evaluate to Int, got {}", what, other)),
    }
}

// -------------------------------------------------------------------
// Type checker
// -------------------------------------------------------------------

/// Type-check a quantifier expression. Binds `var` in a fresh scope
/// while checking the body, asserts the body is `Bool`, then returns
/// `Bool` as the quantifier's type.
pub(crate) fn typecheck_quantifier(
    tc: &mut TypeChecker,
    var: &str,
    range: &QuantRange,
    body: &Node,
) -> Result<Type, String> {
    let var_ty = match range {
        QuantRange::Range { lo, hi } => {
            let lo_ty = tc.check_node(lo)?;
            let hi_ty = tc.check_node(hi)?;
            require_int(&lo_ty, "range lower bound")?;
            require_int(&hi_ty, "range upper bound")?;
            Type::Int
        }
        QuantRange::Iterable(expr) => {
            let _ = tc.check_node(expr)?;
            // Element type is not tracked yet (RES-053 / RES-055). Use
            // `Any` so the body's references to `var` typecheck through
            // until typed arrays land.
            Type::Any
        }
    };

    let body_ty = tc.with_quantifier_binding(var, var_ty, body)?;
    if body_ty != Type::Bool && body_ty != Type::Any {
        return Err(format!(
            "quantifier body must evaluate to Bool, got {}",
            body_ty
        ));
    }
    Ok(Type::Bool)
}

fn require_int(ty: &Type, what: &str) -> Result<(), String> {
    if matches!(ty, Type::Int | Type::Any) {
        Ok(())
    } else {
        Err(format!("{} must be Int, got {}", what, ty))
    }
}

// -------------------------------------------------------------------
// Z3 encoding (feature-gated)
// -------------------------------------------------------------------

/// Feature-gated entry point used by `verifier_z3::translate_bool`.
/// Returns `Some(formula)` for the bounded-range form, `None`
/// otherwise (the caller falls back to runtime / leaves the verdict
/// `Unknown`).
#[cfg(feature = "z3")]
pub(crate) fn z3_encode<'c>(
    ctx: &'c z3::Context,
    kind: QuantifierKind,
    var: &str,
    range: &QuantRange,
    body: &Node,
    bindings: &std::collections::HashMap<String, i64>,
) -> Option<z3::ast::Bool<'c>> {
    use z3::ast::{Ast, Bool, Int};
    let (lo, hi) = match range {
        QuantRange::Range { lo, hi } => (lo.as_ref(), hi.as_ref()),
        QuantRange::Iterable(_) => return None,
    };
    let lo_z = crate::verifier_z3::translate_int_pub(ctx, lo, bindings)?;
    let hi_z = crate::verifier_z3::translate_int_pub(ctx, hi, bindings)?;

    // Rebind `var` to a fresh symbolic Int before encoding the body.
    // The `bindings` map carries *known constants*; a quantified
    // variable is intentionally absent so `translate_int` falls back
    // to `Int::new_const(name)` — that gives Z3 the universal/existential
    // constant it needs.
    let var_const = Int::new_const(ctx, var);
    let body_z = crate::verifier_z3::translate_bool_pub(ctx, body, bindings)?;

    // `lo <= var < hi`
    let in_range = Bool::and(ctx, &[&var_const.ge(&lo_z), &var_const.lt(&hi_z)]);

    let bound: Vec<&dyn Ast<'c>> = vec![&var_const];
    Some(match kind {
        QuantifierKind::Forall => {
            let imp = in_range.implies(&body_z);
            z3::ast::forall_const(ctx, &bound, &[], &imp)
        }
        QuantifierKind::Exists => {
            let conj = Bool::and(ctx, &[&in_range, &body_z]);
            z3::ast::exists_const(ctx, &bound, &[], &conj)
        }
    })
}

// -------------------------------------------------------------------
// Span helper
// -------------------------------------------------------------------

/// Read the span carried on a `Node::Quantifier`. Used by sites that
/// need source positions for diagnostics.
#[allow(dead_code)]
pub(crate) fn quantifier_span(node: &Node) -> Option<span::Span> {
    if let Node::Quantifier { span, .. } = node {
        Some(*span)
    } else {
        None
    }
}

// -------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_program(src: &str) -> Node {
        let lexer = crate::Lexer::new(src.to_string());
        let mut p = crate::Parser::new(lexer);
        p.parse_program()
    }

    fn first_assert_expr(program: &Node) -> Node {
        let stmts = match program {
            Node::Program(stmts) => stmts,
            _ => panic!("expected Program"),
        };
        for s in stmts {
            if let Node::Assert { condition, .. } = &s.node {
                return (**condition).clone();
            }
        }
        panic!("no assert in program");
    }

    fn expect_bool(v: Option<Value>, what: &str) -> bool {
        match v {
            Some(Value::Bool(b)) => b,
            other => panic!("{}: expected Bool, got {:?}", what, other),
        }
    }

    #[test]
    fn parses_forall_range_form() {
        let prog = parse_program("assert(forall i in 0..3: i >= 0);");
        let cond = first_assert_expr(&prog);
        match cond {
            Node::Quantifier {
                kind, var, range, ..
            } => {
                assert_eq!(kind, QuantifierKind::Forall);
                assert_eq!(var, "i");
                assert!(matches!(range, QuantRange::Range { .. }));
            }
            other => panic!("expected Quantifier, got {:?}", other),
        }
    }

    #[test]
    fn parses_exists_iterable_form() {
        let prog = parse_program("assert(exists x in [1, 2, 3]: x > 1);");
        let cond = first_assert_expr(&prog);
        match cond {
            Node::Quantifier {
                kind, var, range, ..
            } => {
                assert_eq!(kind, QuantifierKind::Exists);
                assert_eq!(var, "x");
                assert!(matches!(range, QuantRange::Iterable(_)));
            }
            other => panic!("expected Quantifier, got {:?}", other),
        }
    }

    #[test]
    fn forall_short_circuits_on_first_false() {
        let prog = parse_program("let r = forall i in 0..5: i < 3;\nprintln(r);");
        let mut interp = crate::Interpreter::new();
        let _ = interp.eval(&prog);
        let r = expect_bool(interp.env.get("r"), "r");
        assert!(!r, "forall over [0,5) of `i < 3` should be false");
    }

    #[test]
    fn forall_true_when_all_witnesses_satisfy() {
        let prog = parse_program("let r = forall i in 0..5: i >= 0;\nprintln(r);");
        let mut interp = crate::Interpreter::new();
        let _ = interp.eval(&prog);
        assert!(expect_bool(interp.env.get("r"), "r"));
    }

    #[test]
    fn exists_short_circuits_on_first_true() {
        let prog = parse_program("let r = exists i in 0..5: i == 2;\nprintln(r);");
        let mut interp = crate::Interpreter::new();
        let _ = interp.eval(&prog);
        assert!(expect_bool(interp.env.get("r"), "r"));
    }

    #[test]
    fn exists_false_when_no_witness() {
        let prog = parse_program("let r = exists i in 0..3: i > 100;\nprintln(r);");
        let mut interp = crate::Interpreter::new();
        let _ = interp.eval(&prog);
        assert!(!expect_bool(interp.env.get("r"), "r"));
    }

    #[test]
    fn exists_over_array_iterable() {
        let prog = parse_program("let r = exists x in [1, 5, 9]: x > 4;\nprintln(r);");
        let mut interp = crate::Interpreter::new();
        let _ = interp.eval(&prog);
        assert!(expect_bool(interp.env.get("r"), "r"));
    }

    #[test]
    fn quantified_variable_does_not_escape() {
        let prog = parse_program("let r = forall i in 0..3: i >= 0;\nprintln(r);");
        let mut interp = crate::Interpreter::new();
        let _ = interp.eval(&prog);
        assert!(
            interp.env.get("i").is_none(),
            "quantifier variable `i` leaked out of its scope"
        );
    }

    #[test]
    fn empty_range_is_vacuous_truth_for_forall() {
        let prog = parse_program("let r = forall i in 5..5: false;\nprintln(r);");
        let mut interp = crate::Interpreter::new();
        let _ = interp.eval(&prog);
        assert!(
            expect_bool(interp.env.get("r"), "r"),
            "forall over an empty range is vacuously true"
        );
    }

    #[test]
    fn empty_range_is_false_for_exists() {
        let prog = parse_program("let r = exists i in 5..5: true;\nprintln(r);");
        let mut interp = crate::Interpreter::new();
        let _ = interp.eval(&prog);
        assert!(!expect_bool(interp.env.get("r"), "r"));
    }

    #[test]
    fn typecheck_accepts_forall_range_with_bool_body() {
        let prog = parse_program("assert(forall i in 0..3: i >= 0);");
        let mut tc = TypeChecker::new();
        assert!(tc.check_program(&prog).is_ok());
    }

    #[test]
    fn typecheck_rejects_non_bool_body() {
        let prog = parse_program("assert(forall i in 0..3: i + 1);");
        let mut tc = TypeChecker::new();
        let res = tc.check_program(&prog);
        assert!(res.is_err());
        assert!(res.unwrap_err().contains("body must evaluate to Bool"));
    }
}
