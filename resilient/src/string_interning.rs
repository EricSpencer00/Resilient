//! RES-2612: Compile-time string interning.
//!
//! String interning deduplicates identical string literals to reduce
//! binary size and enable O(1) equality checks in compiled output.
//!
//! ## Interpreter behaviour
//!
//! In the interpreter a thread-local intern pool maps strings to their
//! canonical copy. `intern` adds a string to the pool (or returns the
//! existing copy); `intern_eq` checks structural equality (O(n) here —
//! a compiled backend would use pointer comparison for O(1)).
//!
//! ## API
//!
//!   intern(s)              → string  — intern `s`, return canonical copy
//!   intern_eq(s1, s2)      → bool    — true if both are identical strings
//!   intern_count()         → int     — number of distinct interned strings
//!
//! ## Compile-time analysis
//!
//! `analyze` walks the AST and reports duplicated string literals —
//! strings that appear more than once and would benefit from interning.
//! The pass is advisory (warning-only).

use crate::{Node, Value};
use std::cell::RefCell;
use std::collections::HashMap;

type RResult<T> = Result<T, String>;

// ---------------------------------------------------------------------------
// Thread-local intern pool
// ---------------------------------------------------------------------------

thread_local! {
    static INTERN_POOL: RefCell<HashMap<String, ()>> = RefCell::new(HashMap::new());
}

fn intern_string(s: &str) -> String {
    INTERN_POOL.with(|pool| {
        let mut p = pool.borrow_mut();
        if !p.contains_key(s) {
            p.insert(s.to_string(), ());
        }
        s.to_string()
    })
}

fn pool_count() -> usize {
    INTERN_POOL.with(|pool| pool.borrow().len())
}

// ---------------------------------------------------------------------------
// Builtins
// ---------------------------------------------------------------------------

/// `intern(s) → string` — add `s` to the intern pool and return it.
pub(crate) fn builtin_intern(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => Ok(Value::String(intern_string(s))),
        [other] => Err(format!("intern: expected string, got {other}")),
        _ => Err(format!("intern: expected 1 argument, got {}", args.len())),
    }
}

/// `intern_eq(s1, s2) → bool` — true when the strings are identical.
/// O(n) in the interpreter; compiled backends use pointer comparison.
pub(crate) fn builtin_intern_eq(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(a), Value::String(b)] => Ok(Value::Bool(a == b)),
        [_, _] => Err("intern_eq: expected (string, string)".to_string()),
        _ => Err(format!(
            "intern_eq: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

/// `intern_count() → int` — number of distinct interned strings.
pub(crate) fn builtin_intern_count(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "intern_count: expected 0 arguments, got {}",
            args.len()
        ));
    }
    Ok(Value::Int(pool_count() as i64))
}

// ---------------------------------------------------------------------------
// Compile-time analysis pass
// ---------------------------------------------------------------------------

/// A duplicated string literal found during analysis.
#[derive(Debug, Clone)]
pub struct DuplicateStringWarning {
    pub value: String,
    pub count: usize,
}

/// Walk the AST and return strings that appear more than once.
pub fn analyze(program: &Node) -> Vec<DuplicateStringWarning> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    collect_string_literals(program, &mut counts);
    let mut out: Vec<DuplicateStringWarning> = counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(value, count)| DuplicateStringWarning { value, count })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.value.cmp(&b.value)));
    out
}

fn collect_string_literals(node: &Node, counts: &mut HashMap<String, usize>) {
    match node {
        Node::StringLiteral { value, .. } => {
            *counts.entry(value.clone()).or_insert(0) += 1;
        }
        Node::Program(stmts) => {
            for s in stmts {
                collect_string_literals(&s.node, counts);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_string_literals(s, counts);
            }
        }
        Node::Function { body, .. } => collect_string_literals(body, counts),
        Node::LetStatement { value, .. } => collect_string_literals(value, counts),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_string_literals(condition, counts);
            collect_string_literals(consequence, counts);
            if let Some(alt) = alternative {
                collect_string_literals(alt, counts);
            }
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            collect_string_literals(function, counts);
            for a in arguments {
                collect_string_literals(a, counts);
            }
        }
        Node::InfixExpression { left, right, .. } => {
            collect_string_literals(left, counts);
            collect_string_literals(right, counts);
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            collect_string_literals(v, counts);
        }
        Node::ReturnStatement { value: None, .. } => {}
        _ => {}
    }
}

/// Advisory pass: print warnings for duplicated string literals.
pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let warnings = analyze(program);
    for w in &warnings {
        eprintln!(
            "note: string {:?} appears {} times — consider using intern()",
            w.value, w.count
        );
    }
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

    #[test]
    fn intern_returns_same_string() {
        let out = run(r#"
let a = intern("hello");
let b = intern("hello");
println(a);
println(to_string(intern_eq(a, b)));
"#);
        assert!(out.contains("hello"), "got: {out:?}");
        assert!(out.contains("true"), "got: {out:?}");
    }

    #[test]
    fn intern_eq_false_for_different() {
        let out = run(r#"
let a = intern("foo");
let b = intern("bar");
println(to_string(intern_eq(a, b)));
"#);
        assert!(out.contains("false"), "got: {out:?}");
    }

    #[test]
    fn intern_count_grows() {
        let out = run(r#"
let a = intern("unique_alpha_x");
let b = intern("unique_alpha_y");
let c = intern("unique_alpha_x");
println(to_string(intern_count()));
"#);
        // "unique_alpha_x" and "unique_alpha_y" → at least 2 interned.
        // Thread-local pool may accumulate across tests; just check >= 2.
        let n: i64 = out
            .trim()
            .lines()
            .next()
            .unwrap_or("0")
            .trim()
            .parse()
            .unwrap_or(0);
        assert!(n >= 2, "expected >= 2 interned strings, got: {out:?}");
    }
}
