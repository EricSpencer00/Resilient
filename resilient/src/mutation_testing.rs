//! Feature 43/50 — Mutation Testing.
//!
//! `rz mutate <file>` walks the AST, generates structured mutations
//! (operator swaps, constant changes, branch flips), and reports
//! the program-level mutation site count. A higher count = more spots
//! tests need to cover to distinguish live from killed mutants.
//!
//! Built-in mutators:
//! * **Arithmetic**: `+`↔`-`, `*`↔`/`, `%`→`*`
//! * **Boundary**: `<`↔`<=`, `>`↔`>=`
//! * **Relational**: `==`↔`!=`
//! * **Logical**: `&&`↔`||`
//! * **Bitwise**: `&`↔`|`, `^`→`|`
//! * **Literal**: `0`→`1`, non-zero `n`→`0`, non-zero `n`→`n±1`,
//!   `true`↔`false`, non-empty string→`""`
//! * **Condition negation**: `if c {}` → `if !c {}`
//! * **Return void**: `return <expr>` → omit value
//!
//! The runner does not re-execute tests — it reports the count of
//! generated mutation sites. The test-run orchestrator is a follow-up.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Mutation {
    pub fn_name: String,
    pub kind: String,
    pub description: String,
}

pub fn generate(program: &Node) -> Vec<Mutation> {
    let mut out = Vec::new();
    let Node::Program(stmts) = program else {
        return out;
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            generate_in(body, name, &mut out);
        }
    }
    out
}

fn push(out: &mut Vec<Mutation>, fn_name: &str, kind: &str, description: impl Into<String>) {
    out.push(Mutation {
        fn_name: fn_name.to_string(),
        kind: kind.into(),
        description: description.into(),
    });
}

fn generate_in(node: &Node, fn_name: &str, out: &mut Vec<Mutation>) {
    match node {
        Node::InfixExpression {
            operator,
            left,
            right,
            ..
        } => {
            match operator.as_str() {
                // arithmetic
                "+" => push(out, fn_name, "arithmetic", "swap `+` -> `-`"),
                "-" => push(out, fn_name, "arithmetic", "swap `-` -> `+`"),
                "*" => push(out, fn_name, "arithmetic", "swap `*` -> `/`"),
                "/" => push(out, fn_name, "arithmetic", "swap `/` -> `*`"),
                "%" => push(out, fn_name, "arithmetic", "swap `%` -> `*`"),
                // boundary
                "<"  => push(out, fn_name, "boundary", "swap `<` -> `<=`"),
                "<=" => push(out, fn_name, "boundary", "swap `<=` -> `<`"),
                ">"  => push(out, fn_name, "boundary", "swap `>` -> `>=`"),
                ">=" => push(out, fn_name, "boundary", "swap `>=` -> `>`"),
                // relational
                "==" => push(out, fn_name, "relational", "swap `==` -> `!=`"),
                "!=" => push(out, fn_name, "relational", "swap `!=` -> `==`"),
                // logical
                "&&" => push(out, fn_name, "logical", "swap `&&` -> `||`"),
                "||" => push(out, fn_name, "logical", "swap `||` -> `&&`"),
                // bitwise
                "&" => push(out, fn_name, "bitwise", "swap `&` -> `|`"),
                "|" => push(out, fn_name, "bitwise", "swap `|` -> `&`"),
                "^" => push(out, fn_name, "bitwise", "swap `^` -> `|`"),
                // shift
                "<<" => push(out, fn_name, "shift", "swap `<<` -> `>>`"),
                ">>" => push(out, fn_name, "shift", "swap `>>` -> `<<`"),
                _ => {}
            }
            generate_in(left, fn_name, out);
            generate_in(right, fn_name, out);
        }
        Node::PrefixExpression { operator, right, .. } => {
            if operator == "-" {
                push(out, fn_name, "unary", "swap unary `-` -> `+` (remove negation)");
            }
            generate_in(right, fn_name, out);
        }
        Node::IntegerLiteral { value, .. } => {
            if *value == 0 {
                push(out, fn_name, "literal", "swap `0` -> `1`");
            } else {
                push(out, fn_name, "literal", format!("swap `{value}` -> `0`"));
                push(out, fn_name, "literal", format!("off-by-one `{value}` -> `{}`", value - 1));
                push(out, fn_name, "literal", format!("off-by-one `{value}` -> `{}`", value + 1));
            }
        }
        Node::BooleanLiteral { value, .. } => {
            push(out, fn_name, "literal", format!("flip `{}` -> `{}`", value, !value));
        }
        Node::StringLiteral { value, .. } if !value.is_empty() => {
            push(out, fn_name, "literal", format!("swap string `\"{value}\"` -> `\"\"`"));
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            // Condition negation: `if c` → `if !c`
            push(out, fn_name, "condition", "negate if-condition");
            generate_in(condition, fn_name, out);
            generate_in(consequence, fn_name, out);
            if let Some(e) = alternative {
                generate_in(e, fn_name, out);
            }
        }
        Node::ReturnStatement { value: Some(e), .. } => {
            // Return-void mutation: drop the return value
            push(out, fn_name, "return", "drop return value (return void)");
            generate_in(e, fn_name, out);
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                generate_in(s, fn_name, out);
            }
        }
        Node::LetStatement { value, .. } | Node::Assignment { value, .. } => {
            generate_in(value, fn_name, out);
        }
        Node::ExpressionStatement { expr, .. } => generate_in(expr, fn_name, out),
        Node::WhileStatement { condition, body, .. } => {
            push(out, fn_name, "condition", "negate while-condition");
            generate_in(condition, fn_name, out);
            generate_in(body, fn_name, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            generate_in(iterable, fn_name, out);
            generate_in(body, fn_name, out);
        }
        Node::CallExpression { arguments, .. } => {
            for a in arguments {
                generate_in(a, fn_name, out);
            }
        }
        _ => {}
    }
}

/// Per-function mutation summary: function name → (total sites, kinds).
pub fn summarize(mutations: &[Mutation]) -> HashMap<String, (usize, Vec<String>)> {
    let mut map: HashMap<String, (usize, Vec<String>)> = HashMap::new();
    for m in mutations {
        let e = map.entry(m.fn_name.clone()).or_default();
        e.0 += 1;
        if !e.1.contains(&m.kind) {
            e.1.push(m.kind.clone());
        }
    }
    map
}

/// Report mutation sites and contract coverage.
///
/// Beyond just counting sites, this pass cross-references the mutation
/// inventory against the function's contract declarations. Functions that
/// have mutation sites but zero `requires`/`ensures` contracts are flagged:
/// the Z3 verifier has nothing to kill their mutants with, so any behavioral
/// regression in those functions would pass undetected.
///
/// The "unconstrained mutation ratio" is a useful CI metric: 0% means every
/// function with mutations is contractually verified; 100% means vibe-coded
/// all the way down.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    // Fast-reject: skip when there are no infix expressions or literals to mutate.
    let has_mutatable = crate::uniqueness_walk::any_node(program, |n| {
        matches!(
            n,
            Node::InfixExpression { .. }
                | Node::BooleanLiteral { .. }
                | Node::IntegerLiteral { .. }
        )
    });
    if !has_mutatable {
        return Ok(());
    }
    let mutations = generate(program);
    if mutations.is_empty() {
        return Ok(());
    }
    let summary = summarize(&mutations);
    let total: usize = summary.values().map(|(c, _)| c).sum();
    eprintln!(
        "mutation: {} total mutation site(s) across {} function(s)",
        total,
        summary.len()
    );
    let mut fns: Vec<_> = summary.iter().collect();
    fns.sort_by_key(|(n, _)| n.as_str());
    for (fn_name, (count, kinds)) in &fns {
        let mut kinds_sorted = kinds.clone();
        kinds_sorted.sort();
        eprintln!(
            "mutation:   `{fn_name}`: {count} site(s) [{}]",
            kinds_sorted.join(", ")
        );
    }

    // Cross-reference with contract declarations to identify unconstrained sites.
    let contracted_fns = contract_coverage(program);
    let unconstrained: Vec<(&str, usize)> = fns
        .iter()
        .filter_map(|(name, (count, _))| {
            if !contracted_fns.contains(name.as_str()) {
                Some((name.as_str(), *count))
            } else {
                None
            }
        })
        .collect();
    if !unconstrained.is_empty() {
        let unconstrained_total: usize = unconstrained.iter().map(|(_, n)| n).sum();
        let pct = unconstrained_total * 100 / total.max(1);
        eprintln!(
            "{source_path}:0:0: warning[mutation]: \
             {unconstrained_total}/{total} mutation site(s) ({pct}%) are in \
             functions with no contracts — the Z3 verifier cannot kill them"
        );
        for (name, count) in &unconstrained {
            eprintln!(
                "{source_path}:0:0: warning[mutation]: \
                 `{name}`: {count} unconstrained mutation site(s) — \
                 add `requires`/`ensures` contracts"
            );
        }
    }
    Ok(())
}

/// Returns the set of function names that have at least one
/// `requires` or `ensures` contract clause.
fn contract_coverage(program: &Node) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    if let Node::Program(stmts) = program {
        for s in stmts {
            if let Node::Function {
                name,
                requires,
                ensures,
                ..
            } = &s.node
            {
                if !requires.is_empty() || !ensures.is_empty() {
                    out.insert(name.clone());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn arithmetic_op_generates_mutation() {
        let src = r#"fn add(int a, int b) { return a + b; }"#;
        let (prog, _) = parse(src);
        let mutations = generate(&prog);
        assert!(!mutations.is_empty());
        assert!(mutations.iter().any(|m| m.kind == "arithmetic"));
    }

    #[test]
    fn boundary_op_generates_mutation() {
        let src = r#"fn lt(int a, int b) -> bool { return a < b; }"#;
        let (prog, _) = parse(src);
        let mutations = generate(&prog);
        assert!(mutations.iter().any(|m| m.kind == "boundary"));
    }

    #[test]
    fn multiply_and_divide_generate_arithmetic_mutations() {
        let src = r#"fn calc(int a, int b) -> int { return a * b / 2; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        let descs: Vec<_> = m.iter().map(|x| x.description.as_str()).collect();
        assert!(descs.iter().any(|d| d.contains("*` -> `/`")));
        assert!(descs.iter().any(|d| d.contains("/` -> `*`")));
    }

    #[test]
    fn relational_ops_generate_mutations() {
        let src = r#"fn eq(int a, int b) -> bool { return a == b; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(m.iter().any(|x| x.kind == "relational"));
    }

    #[test]
    fn neq_generates_relational_mutation() {
        let src = r#"fn neq(int a, int b) -> bool { return a != b; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(m.iter().any(|x| x.description.contains("!=` -> `==`")));
    }

    #[test]
    fn boundary_gt_gte_generate_mutations() {
        let src = r#"fn cmp(int a, int b) -> bool { return a > b; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(m.iter().any(|x| x.description.contains(">` -> `>=`")));
    }

    #[test]
    fn logical_ops_generate_mutations() {
        let src = r#"fn both(bool a, bool b) -> bool { return a && b; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(m.iter().any(|x| x.kind == "logical"));
    }

    #[test]
    fn bitwise_ops_generate_mutations() {
        let src = r#"fn bit(int a, int b) -> int { return a & b; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(m.iter().any(|x| x.kind == "bitwise"));
    }

    #[test]
    fn non_zero_literal_gets_three_mutations() {
        let src = r#"fn f(int x) -> int { return x + 5; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        let lit: Vec<_> = m.iter().filter(|x| x.kind == "literal").collect();
        // 5 → 0, 5 → 4, 5 → 6
        assert!(lit.iter().any(|x| x.description.contains("-> `0`")));
        assert!(lit.iter().any(|x| x.description.contains("-> `4`")));
        assert!(lit.iter().any(|x| x.description.contains("-> `6`")));
    }

    #[test]
    fn zero_literal_gets_one_mutation() {
        let src = r#"fn f(int x) -> int { return x + 0; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        let lit: Vec<_> = m.iter().filter(|x| x.kind == "literal").collect();
        assert!(lit.iter().any(|x| x.description.contains("`0` -> `1`")));
    }

    #[test]
    fn bool_literal_flips() {
        let src = r#"fn f() -> bool { return true; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(m.iter().any(|x| x.kind == "literal" && x.description.contains("true")));
    }

    #[test]
    fn condition_negation_generated_for_if() {
        let src = r#"fn f(int x) { if x > 0 { return x; } }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(m.iter().any(|x| x.kind == "condition" && x.description.contains("negate if")));
    }

    #[test]
    fn return_void_mutation_generated() {
        let src = r#"fn f(int x) -> int { return x + 1; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(m.iter().any(|x| x.kind == "return"));
    }

    #[test]
    fn forin_body_traversed() {
        let src = r#"fn f(IntArr xs) -> int { let sum = 0; for x in xs { sum = sum + x; } return sum; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(m.iter().any(|x| x.kind == "arithmetic"), "for-in body must be traversed");
    }

    #[test]
    fn summarize_groups_by_function() {
        let src = r#"
            fn add(int a, int b) -> int { return a + b; }
            fn sub(int a, int b) -> int { return a - b; }
        "#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        let s = summarize(&m);
        assert!(s.contains_key("add"));
        assert!(s.contains_key("sub"));
    }

    #[test]
    fn check_ok_on_empty_program() {
        let (prog, _) = parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn check_ok_on_program_with_mutations() {
        let src = r#"fn f(int x) -> int { return x + 1; }"#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    // ── contract_coverage / check() cross-reference ──────────────────────────

    #[test]
    fn contract_coverage_finds_contracted_fn() {
        let src = "fn f(int x) requires x > 0 { return x; }";
        let (prog, _) = parse(src);
        let cov = contract_coverage(&prog);
        assert!(cov.contains("f"));
    }

    #[test]
    fn contract_coverage_empty_for_uncontracted_fn() {
        let src = "fn f(int x) { return x; }";
        let (prog, _) = parse(src);
        let cov = contract_coverage(&prog);
        assert!(!cov.contains("f"));
    }

    #[test]
    fn check_ok_on_contracted_fn_with_mutations() {
        // Contracted function — no unconstrained-mutation warning, still Ok.
        let src = "fn add(int a, int b) -> int requires a >= 0 ensures result >= 0 { return a + b; }";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn check_ok_on_uncontracted_fn_with_mutations() {
        // Uncontracted function — warns about unconstrained sites but is NOT an error.
        let src = "fn add(int a, int b) -> int { return a + b; }";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }
}
