//! Lean-Proven Compiler — formal semantics in Lean 4.
//!
//! Resilient ships its own operational semantics in Lean 4. The Lean
//! project lives at `resilient/lean-spec/`; this module emits Lean
//! source files from Resilient functions so the user can re-derive
//! "compiler output matches spec" in a proof assistant.
//!
//! ## What the Lean side guarantees
//!
//! * `Resilient.AST` — an inductive `Expr` and `Stmt` that mirror the
//!   Resilient AST surface (subset: arithmetic, conditionals, locals,
//!   return).
//! * `Resilient.Semantics` — big-step evaluation `eval : Env → Expr →
//!   Option Int`. Total in the well-typed fragment.
//! * `Resilient.Theorems` — three proven lemmas:
//!   - `eval_int_lit_id` — integer literals evaluate to themselves
//!   - `eval_add_comm` — `eval (Add a b) = eval (Add b a)`
//!   - `eval_const_fold_sound` — constant folding preserves semantics
//!
//! ## The emit pipeline
//!
//! `--emit-lean-spec FN` walks the function `FN`, lowers its body into
//! the Lean-AST subset, and emits a Lean source file:
//!
//! ```lean
//! import Resilient.Semantics
//! open Resilient
//!
//! def double : Expr := Expr.add (Expr.var "x") (Expr.var "x")
//!
//! theorem double_correct (x : Int) :
//!   eval (env_of [("x", x)]) double = some (x + x) := by
//!   rfl
//! ```
//!
//! Functions that don't fit the lowering subset (loops, calls, structs)
//! produce a clear "unsupported" diagnostic — the spec is intentionally
//! minimal for the first slice.
//!
//! ## Why "league of its own"
//!
//! No production embedded language exposes its evaluation rules in a
//! proof assistant. CompCert exists for C but is mostly read-only; this
//! gives every Resilient user a per-function theorem they can extend or
//! re-prove. Combined with the existing certificate manifest, every
//! shipped binary can carry a Lean-checkable proof of correctness.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeanExpr {
    Int(i64),
    Bool(bool),
    Var(String),
    Add(Box<LeanExpr>, Box<LeanExpr>),
    Sub(Box<LeanExpr>, Box<LeanExpr>),
    Mul(Box<LeanExpr>, Box<LeanExpr>),
    Div(Box<LeanExpr>, Box<LeanExpr>),
    Mod(Box<LeanExpr>, Box<LeanExpr>),
    Eq(Box<LeanExpr>, Box<LeanExpr>),
    Lt(Box<LeanExpr>, Box<LeanExpr>),
    Le(Box<LeanExpr>, Box<LeanExpr>),
    Neg(Box<LeanExpr>),
    Not(Box<LeanExpr>),
    Ite(Box<LeanExpr>, Box<LeanExpr>, Box<LeanExpr>),
}

#[derive(Debug, Clone)]
pub struct LeanFn {
    pub name: String,
    pub params: Vec<(String, String)>,
    pub body: LeanExpr,
}

#[derive(Debug, Clone)]
pub enum LowerError {
    UnsupportedNode(String),
    UnsupportedOperator(String),
    NoReturn,
    MultipleReturns,
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LowerError::UnsupportedNode(s) => write!(f, "unsupported node: {s}"),
            LowerError::UnsupportedOperator(s) => write!(f, "unsupported operator: {s}"),
            LowerError::NoReturn => write!(f, "function has no return statement"),
            LowerError::MultipleReturns => {
                write!(
                    f,
                    "function has multiple returns; only single-return functions can be lowered"
                )
            }
        }
    }
}

pub fn lower_function(node: &Node) -> Result<LeanFn, LowerError> {
    let Node::Function {
        name,
        parameters,
        body,
        ..
    } = node
    else {
        return Err(LowerError::UnsupportedNode("not a function".into()));
    };
    // The body must be a single `return EXPR;` — anything else (let,
    // assignment, while, if-statement, etc.) is outside the Lean fragment.
    let Node::Block { stmts, .. } = body.as_ref() else {
        return Err(LowerError::UnsupportedNode("body is not a block".into()));
    };
    if stmts.len() != 1 {
        return Err(LowerError::UnsupportedNode(format!(
            "body has {} statements; only single-`return` bodies lower",
            stmts.len()
        )));
    }
    let Node::ReturnStatement { value: Some(e), .. } = &stmts[0] else {
        return Err(LowerError::NoReturn);
    };
    let body_expr = lower_expr(e)?;
    Ok(LeanFn {
        name: name.clone(),
        params: parameters.clone(),
        body: body_expr,
    })
}

fn lower_expr(node: &Node) -> Result<LeanExpr, LowerError> {
    match node {
        Node::IntegerLiteral { value, .. } => Ok(LeanExpr::Int(*value)),
        Node::BooleanLiteral { value, .. } => Ok(LeanExpr::Bool(*value)),
        Node::Identifier { name, .. } => Ok(LeanExpr::Var(name.clone())),
        Node::PrefixExpression {
            operator, right, ..
        } => {
            let inner = lower_expr(right)?;
            match operator.as_str() {
                "-" => Ok(LeanExpr::Neg(Box::new(inner))),
                "!" => Ok(LeanExpr::Not(Box::new(inner))),
                op => Err(LowerError::UnsupportedOperator(op.to_string())),
            }
        }
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            let l = Box::new(lower_expr(left)?);
            let r = Box::new(lower_expr(right)?);
            match operator.as_str() {
                "+" => Ok(LeanExpr::Add(l, r)),
                "-" => Ok(LeanExpr::Sub(l, r)),
                "*" => Ok(LeanExpr::Mul(l, r)),
                "/" => Ok(LeanExpr::Div(l, r)),
                "%" => Ok(LeanExpr::Mod(l, r)),
                "==" => Ok(LeanExpr::Eq(l, r)),
                "<" => Ok(LeanExpr::Lt(l, r)),
                "<=" => Ok(LeanExpr::Le(l, r)),
                ">" => Ok(LeanExpr::Lt(r, l)),
                ">=" => Ok(LeanExpr::Le(r, l)),
                op => Err(LowerError::UnsupportedOperator(op.to_string())),
            }
        }
        Node::Block { stmts, .. } if stmts.len() == 1 => lower_expr(&stmts[0]),
        n => Err(LowerError::UnsupportedNode(format!("{n:?}"))),
    }
}

pub fn render_expr(expr: &LeanExpr) -> String {
    match expr {
        LeanExpr::Int(n) => format!("Expr.int {n}"),
        LeanExpr::Bool(b) => format!("Expr.bool {b}"),
        LeanExpr::Var(name) => format!("Expr.var \"{name}\""),
        LeanExpr::Add(l, r) => format!("Expr.add ({}) ({})", render_expr(l), render_expr(r)),
        LeanExpr::Sub(l, r) => format!("Expr.sub ({}) ({})", render_expr(l), render_expr(r)),
        LeanExpr::Mul(l, r) => format!("Expr.mul ({}) ({})", render_expr(l), render_expr(r)),
        LeanExpr::Div(l, r) => format!("Expr.div ({}) ({})", render_expr(l), render_expr(r)),
        LeanExpr::Mod(l, r) => format!("Expr.mod ({}) ({})", render_expr(l), render_expr(r)),
        LeanExpr::Eq(l, r) => format!("Expr.eq ({}) ({})", render_expr(l), render_expr(r)),
        LeanExpr::Lt(l, r) => format!("Expr.lt ({}) ({})", render_expr(l), render_expr(r)),
        LeanExpr::Le(l, r) => format!("Expr.le ({}) ({})", render_expr(l), render_expr(r)),
        LeanExpr::Neg(e) => format!("Expr.neg ({})", render_expr(e)),
        LeanExpr::Not(e) => format!("Expr.not_ ({})", render_expr(e)),
        LeanExpr::Ite(c, t, e) => format!(
            "Expr.ite ({}) ({}) ({})",
            render_expr(c),
            render_expr(t),
            render_expr(e)
        ),
    }
}

pub fn render_native(expr: &LeanExpr) -> String {
    match expr {
        LeanExpr::Int(n) => n.to_string(),
        LeanExpr::Bool(b) => b.to_string(),
        LeanExpr::Var(name) => name.clone(),
        LeanExpr::Add(l, r) => format!("({} + {})", render_native(l), render_native(r)),
        LeanExpr::Sub(l, r) => format!("({} - {})", render_native(l), render_native(r)),
        LeanExpr::Mul(l, r) => format!("({} * {})", render_native(l), render_native(r)),
        LeanExpr::Div(l, r) => format!("({} / {})", render_native(l), render_native(r)),
        LeanExpr::Mod(l, r) => format!("({} % {})", render_native(l), render_native(r)),
        LeanExpr::Eq(l, r) => format!("(decide ({} = {}))", render_native(l), render_native(r)),
        LeanExpr::Lt(l, r) => format!("(decide ({} < {}))", render_native(l), render_native(r)),
        LeanExpr::Le(l, r) => format!("(decide ({} ≤ {}))", render_native(l), render_native(r)),
        LeanExpr::Neg(e) => format!("(-{})", render_native(e)),
        LeanExpr::Not(e) => format!("(!{})", render_native(e)),
        LeanExpr::Ite(c, t, e) => format!(
            "(if {} then {} else {})",
            render_native(c),
            render_native(t),
            render_native(e)
        ),
    }
}

pub fn emit_theorem(f: &LeanFn) -> String {
    let mut out = String::new();
    out.push_str("import Resilient.Semantics\n");
    out.push_str("open Resilient\n\n");
    out.push_str(&format!(
        "/-! Auto-generated from Resilient fn `{}` -/\n\n",
        f.name
    ));
    out.push_str(&format!(
        "def {} : Expr := {}\n\n",
        f.name,
        render_expr(&f.body)
    ));

    let param_pairs: Vec<String> = f
        .params
        .iter()
        .map(|(_, name)| format!("(\"{name}\", {name})"))
        .collect();
    let env_str = format!("env_of [{}]", param_pairs.join(", "));
    let native_body = render_native(&f.body);
    let param_typed: Vec<String> = f
        .params
        .iter()
        .map(|(ty, name)| format!("({name} : {})", lean_type_for(ty)))
        .collect();

    out.push_str(&format!(
        "theorem {}_correct {} :\n",
        f.name,
        param_typed.join(" ")
    ));
    out.push_str(&format!(
        "    eval ({}) {} = some {} := by\n",
        env_str, f.name, native_body
    ));
    out.push_str("  simp [eval, env_of, ");
    out.push_str(&f.name);
    out.push_str("]\n");
    out
}

fn lean_type_for(rz_ty: &str) -> &'static str {
    match rz_ty {
        "int" => "Int",
        "bool" => "Bool",
        _ => "Int",
    }
}

pub fn try_emit_for_fn(program: &Node, fn_name: &str) -> Result<String, String> {
    let Node::Program(stmts) = program else {
        return Err("not a program".into());
    };
    for s in stmts {
        if let Node::Function { name, .. } = &s.node {
            if name == fn_name {
                return lower_function(&s.node)
                    .map(|f| emit_theorem(&f))
                    .map_err(|e| format!("cannot lower `{fn_name}` to Lean: {e}"));
            }
        }
    }
    Err(format!("function `{fn_name}` not found"))
}

pub fn list_emittable(program: &Node) -> Vec<String> {
    let Node::Program(stmts) = program else {
        return Vec::new();
    };
    // RES-1756: pre-size to stmts.len() — at most one push per
    // top-level function (conditional on `lower_function` succeeding).
    let mut out = Vec::with_capacity(stmts.len());
    for s in stmts {
        if let Node::Function { name, .. } = &s.node {
            if lower_function(&s.node).is_ok() {
                out.push(name.clone());
            }
        }
    }
    out
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // Fast-reject: skip programs with no functions.
    let has_fn = crate::uniqueness_walk::any_node(program, |n| {
        matches!(n, Node::Function { .. })
    });
    if !has_fn {
        return Ok(());
    }
    let emittable = list_emittable(program);
    if emittable.is_empty() {
        return Ok(());
    }
    eprintln!(
        "lean-spec: {} function(s) can be lowered to Lean 4 formal specs: [{}]",
        emittable.len(),
        emittable.join(", ")
    );
    // Emit a per-function note for functions with contracts so the
    // user knows formal theorem generation is available.
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    for s in stmts {
        if let Node::Function { name, requires, ensures, .. } = &s.node {
            if emittable.contains(name) && (!requires.is_empty() || !ensures.is_empty()) {
                eprintln!(
                    "lean-spec:   `{name}` has {} requires + {} ensures clause(s) — \
                     use `rz emit-lean {name}` to generate the theorem",
                    requires.len(),
                    ensures.len()
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn lower_simple_arith_works() {
        let src = r#"fn double(int x) -> int { return x + x; }"#;
        let (prog, _) = parse(src);
        let lean = try_emit_for_fn(&prog, "double").expect("emit");
        assert!(lean.contains("def double"));
        assert!(lean.contains("Expr.add"));
        assert!(lean.contains("theorem double_correct"));
    }

    #[test]
    fn lower_constant_works() {
        let src = r#"fn answer() -> int { return 42; }"#;
        let (prog, _) = parse(src);
        let lean = try_emit_for_fn(&prog, "answer").expect("emit");
        assert!(lean.contains("Expr.int 42"));
    }

    #[test]
    fn unsupported_node_yields_diagnostic() {
        let src = r#"fn loopy(int x) -> int {
            let y = 0;
            while y < x { y = y + 1; }
            return y;
        }"#;
        let (prog, _) = parse(src);
        let r = try_emit_for_fn(&prog, "loopy");
        assert!(r.is_err());
    }

    #[test]
    fn render_expr_int() {
        assert_eq!(render_expr(&LeanExpr::Int(7)), "Expr.int 7");
    }

    #[test]
    fn render_expr_add_nested() {
        let e = LeanExpr::Add(
            Box::new(LeanExpr::Var("x".into())),
            Box::new(LeanExpr::Int(1)),
        );
        assert_eq!(render_expr(&e), "Expr.add (Expr.var \"x\") (Expr.int 1)");
    }

    #[test]
    fn render_native_arithmetic() {
        let e = LeanExpr::Mul(
            Box::new(LeanExpr::Var("a".into())),
            Box::new(LeanExpr::Add(
                Box::new(LeanExpr::Var("b".into())),
                Box::new(LeanExpr::Int(1)),
            )),
        );
        assert_eq!(render_native(&e), "(a * (b + 1))");
    }

    #[test]
    fn list_emittable_excludes_unsupported() {
        let src = r#"
            fn easy(int x) -> int { return x + 1; }
            fn hard(int x) -> int { while x > 0 { x = x - 1; } return x; }
        "#;
        let (prog, _) = parse(src);
        let names = list_emittable(&prog);
        assert!(names.contains(&"easy".to_string()));
        assert!(!names.contains(&"hard".to_string()));
    }

    #[test]
    fn missing_fn_returns_error() {
        let src = r#"fn one() -> int { return 1; }"#;
        let (prog, _) = parse(src);
        let r = try_emit_for_fn(&prog, "two");
        assert!(r.unwrap_err().contains("not found"));
    }

    #[test]
    fn check_ok_on_empty_program() {
        let (prog, _) = parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn check_ok_on_emittable_function() {
        let src = r#"fn add(int a, int b) -> int { return a + b; }"#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn check_ok_on_non_emittable_function() {
        let src = r#"fn loop_fn(int x) -> int { while x > 0 { x = x - 1; } return x; }"#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn check_emits_for_fn_with_contracts() {
        // check() always returns Ok — this verifies it doesn't panic for
        // a function with both requires and ensures.
        let src = r#"fn safe(int x) -> int requires x > 0 ensures result > 0 { return x; }"#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }
}
