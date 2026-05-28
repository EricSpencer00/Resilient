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

/// RES-2202: `kind` is now `&'static str`. Every call site in
/// `generate_in` passes a string literal ("arithmetic", "boundary",
/// "relational", "logical", "bitwise", "shift", "unary"), so there
/// was never a runtime-computed kind value — the previous `String`
/// shape forced a `kind.into()` allocation per Mutation push for
/// data that lives at static lifetime. Downstream `summarize`
/// kept the kind in a `Vec<String>`; switching to `Vec<&'static str>`
/// drops both the per-push alloc AND the per-summarize-push
/// `m.kind.clone()` (now a `Copy` of a `&str`).
/// RES-2288: `fn_name` is `&'a str` borrowed from the AST (top-level
/// Function name slot, or the mangled `Struct$method` ident inside an
/// ImplBlock). The previous `String` shape paid a per-mutation
/// `to_string()` allocation in `push` — for a typical function with
/// dozens of mutation sites, that's dozens of wasted heap allocations
/// per fn during the `generate_in` walk. Same shape as RES-2220
/// (labeled_break::DeepBreakWarning) / RES-2204 (coverage_warnings).
/// `kind` keeps its `&'static str` form (RES-2202); `description`
/// stays `String` because most call sites produce computed text via
/// `format!(...)`.
#[derive(Debug, Clone)]
pub struct Mutation<'a> {
    pub fn_name: &'a str,
    pub kind: &'static str,
    pub description: String,
}

pub fn generate(program: &Node) -> Vec<Mutation<'_>> {
    let mut out = Vec::new();
    let Node::Program(stmts) = program else {
        return out;
    };
    for s in stmts {
        match &s.node {
            Node::Function { name, body, .. } => {
                generate_in(body, name.as_str(), &mut out);
            }
            // RES-1918: impl-block methods are parsed as `Node::Function`
            // values with mangled names (`<StructName>$<method>`); without
            // this arm the entire method body is invisible to the mutation
            // walker.
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    if let Node::Function { name, body, .. } = method {
                        generate_in(body, name.as_str(), &mut out);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn push<'a>(
    out: &mut Vec<Mutation<'a>>,
    fn_name: &'a str,
    kind: &'static str,
    description: impl Into<String>,
) {
    out.push(Mutation {
        fn_name,
        kind,
        description: description.into(),
    });
}

fn generate_in<'a>(node: &'a Node, fn_name: &'a str, out: &mut Vec<Mutation<'a>>) {
    match node {
        Node::InfixExpression {
            operator,
            left,
            right,
            ..
        } => {
            match *operator {
                // arithmetic
                "+" => push(out, fn_name, "arithmetic", "swap `+` -> `-`"),
                "-" => push(out, fn_name, "arithmetic", "swap `-` -> `+`"),
                "*" => push(out, fn_name, "arithmetic", "swap `*` -> `/`"),
                "/" => push(out, fn_name, "arithmetic", "swap `/` -> `*`"),
                "%" => push(out, fn_name, "arithmetic", "swap `%` -> `*`"),
                // boundary
                "<" => push(out, fn_name, "boundary", "swap `<` -> `<=`"),
                "<=" => push(out, fn_name, "boundary", "swap `<=` -> `<`"),
                ">" => push(out, fn_name, "boundary", "swap `>` -> `>=`"),
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
        Node::PrefixExpression {
            operator, right, ..
        } => {
            if *operator == "-" {
                push(
                    out,
                    fn_name,
                    "unary",
                    "swap unary `-` -> `+` (remove negation)",
                );
            }
            generate_in(right, fn_name, out);
        }
        Node::IntegerLiteral { value, .. } => {
            if *value == 0 {
                push(out, fn_name, "literal", "swap `0` -> `1`");
            } else {
                push(out, fn_name, "literal", format!("swap `{value}` -> `0`"));
                push(
                    out,
                    fn_name,
                    "literal",
                    format!("off-by-one `{value}` -> `{}`", value - 1),
                );
                push(
                    out,
                    fn_name,
                    "literal",
                    format!("off-by-one `{value}` -> `{}`", value + 1),
                );
            }
        }
        Node::BooleanLiteral { value, .. } => {
            push(
                out,
                fn_name,
                "literal",
                format!("flip `{}` -> `{}`", value, !value),
            );
        }
        Node::StringLiteral { value, .. } if !value.is_empty() => {
            push(
                out,
                fn_name,
                "literal",
                format!("swap string `\"{value}\"` -> `\"\"`"),
            );
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
        Node::WhileStatement {
            condition, body, ..
        } => {
            push(out, fn_name, "condition", "negate while-condition");
            generate_in(condition, fn_name, out);
            generate_in(body, fn_name, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            generate_in(iterable, fn_name, out);
            generate_in(body, fn_name, out);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            // RES-1918: descend into the callee position too — chained
            // expressions like `foo(1).bar(2)` parse with a CallExpression
            // as the `function` of the outer call, so a literal `1` inside
            // is otherwise invisible.
            generate_in(function, fn_name, out);
            for a in arguments {
                generate_in(a, fn_name, out);
            }
        }
        // RES-1918: match scrutinee, guards, and arm bodies all carry
        // mutatable nodes. The arm-pattern Vec<Pattern> uses literal
        // ints/bools internally; mutating patterns would change semantic
        // coverage in subtle ways, so we deliberately leave patterns
        // alone and walk only the value-producing positions.
        Node::Match {
            scrutinee, arms, ..
        } => {
            generate_in(scrutinee, fn_name, out);
            for (_, guard, body) in arms {
                if let Some(g) = guard {
                    generate_in(g, fn_name, out);
                }
                generate_in(body, fn_name, out);
            }
        }
        // RES-1918: struct/collection literal element expressions.
        Node::StructLiteral { fields, base, .. } => {
            if let Some(b) = base {
                generate_in(b, fn_name, out);
            }
            for (_, v) in fields {
                generate_in(v, fn_name, out);
            }
        }
        Node::ArrayLiteral { items, .. } | Node::SetLiteral { items, .. } => {
            for i in items {
                generate_in(i, fn_name, out);
            }
        }
        Node::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                generate_in(k, fn_name, out);
                generate_in(v, fn_name, out);
            }
        }
        Node::TupleLiteral { items, .. } => {
            for i in items {
                generate_in(i, fn_name, out);
            }
        }
        Node::TupleIndex { tuple, .. } => generate_in(tuple, fn_name, out),
        // RES-1918: indexing forms.
        Node::IndexExpression { target, index, .. } => {
            generate_in(target, fn_name, out);
            generate_in(index, fn_name, out);
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            generate_in(target, fn_name, out);
            generate_in(index, fn_name, out);
            generate_in(value, fn_name, out);
        }
        Node::Slice { target, lo, hi, .. } => {
            generate_in(target, fn_name, out);
            if let Some(e) = lo {
                generate_in(e, fn_name, out);
            }
            if let Some(e) = hi {
                generate_in(e, fn_name, out);
            }
        }
        // RES-1918: field-access chains.
        Node::FieldAccess { target, .. } => generate_in(target, fn_name, out),
        Node::FieldAssignment { target, value, .. } => {
            generate_in(target, fn_name, out);
            generate_in(value, fn_name, out);
        }
        // RES-1918: error-handling expression forms.
        Node::TryExpression { expr, .. } => generate_in(expr, fn_name, out),
        Node::TryCatch { body, handlers, .. } => {
            for s in body {
                generate_in(s, fn_name, out);
            }
            for (_caught_var, handler_body) in handlers {
                for s in handler_body {
                    generate_in(s, fn_name, out);
                }
            }
        }
        Node::OptionalChain { object, access, .. } => {
            generate_in(object, fn_name, out);
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args {
                    generate_in(a, fn_name, out);
                }
            }
        }
        // RES-1918: block-shaped statement forms.
        Node::LiveBlock {
            body, invariants, ..
        } => {
            generate_in(body, fn_name, out);
            for inv in invariants {
                generate_in(inv, fn_name, out);
            }
        }
        Node::UnsafeBlock { body, .. } => generate_in(body, fn_name, out),
        // RES-1918: interpolated-string sub-expressions.
        Node::InterpolatedString { parts, .. } => {
            for p in parts {
                if let crate::string_interp::StringPart::Expr(e) = p {
                    generate_in(e, fn_name, out);
                }
            }
        }
        // RES-1918: lambda body.
        Node::FunctionLiteral { body, .. } => generate_in(body, fn_name, out),
        // RES-1918: additional binding-introduction forms with
        // value-position expressions.
        Node::StaticLet { value, .. } | Node::Const { value, .. } => {
            generate_in(value, fn_name, out);
        }
        Node::LetDestructureStruct { value, .. } | Node::LetTupleDestructure { value, .. } => {
            generate_in(value, fn_name, out);
        }
        // RES-1918: assert/assume conditions carry mutatable infix /
        // literal sub-expressions; mutating these is exactly the kind
        // of regression mutation testing is meant to expose.
        Node::Assert { condition, .. } => generate_in(condition, fn_name, out),
        Node::Assume {
            condition, message, ..
        } => {
            generate_in(condition, fn_name, out);
            if let Some(m) = message {
                generate_in(m, fn_name, out);
            }
        }
        _ => {}
    }
}

/// Per-function mutation summary: function name → (total sites, kinds).
///
/// RES-2202: `kinds` is `Vec<&'static str>` because `Mutation::kind`
/// is `&'static str` (mutation categories are compile-time literals).
/// The previous `Vec<String>` shape paid a per-push `m.kind.clone()`
/// for data with static lifetime.
///
/// The fn-name lookup uses a get-or-insert dance instead of
/// `entry(m.fn_name.clone()).or_default()` so the `String` is only
/// allocated on the cold first-mutation-for-this-fn branch.
/// Subsequent mutations for the same fn pay zero allocations.
pub fn summarize(mutations: &[Mutation<'_>]) -> HashMap<String, (usize, Vec<&'static str>)> {
    let mut map: HashMap<String, (usize, Vec<&'static str>)> = HashMap::new();
    for m in mutations {
        if let Some(e) = map.get_mut(m.fn_name) {
            e.0 += 1;
            if !e.1.contains(&m.kind) {
                e.1.push(m.kind);
            }
        } else {
            map.insert(m.fn_name.to_string(), (1, vec![m.kind]));
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
    // RES-1918: removed the `any_node` fast-reject pre-scan. The
    // `uniqueness_walk::any_node` walker did not descend into the same
    // set of nodes that `generate_in` does (it skipped `ImplBlock`,
    // `StructLiteral`, `MapLiteral`, `SetLiteral`, and others), so any
    // program whose only mutatable nodes lived inside an impl method
    // body or a struct-literal field hit the fast-reject and produced
    // no mutation output. The `mutations.is_empty()` guard below still
    // short-circuits empty programs, and `generate` walks once
    // regardless — so the lost work is at most one early-terminating
    // walk per typecheck of a program with no mutatables (an empty
    // program or a declarations-only file). Same shape as RES-1916 /
    // RES-1917's removal of redundant any_node pre-scans.
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
        assert!(
            m.iter()
                .any(|x| x.kind == "literal" && x.description.contains("true"))
        );
    }

    #[test]
    fn condition_negation_generated_for_if() {
        let src = r#"fn f(int x) { if x > 0 { return x; } }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(
            m.iter()
                .any(|x| x.kind == "condition" && x.description.contains("negate if"))
        );
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
        let src =
            r#"fn f(IntArr xs) -> int { let sum = 0; for x in xs { sum = sum + x; } return sum; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(
            m.iter().any(|x| x.kind == "arithmetic"),
            "for-in body must be traversed"
        );
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
        let src =
            "fn add(int a, int b) -> int requires a >= 0 ensures result >= 0 { return a + b; }";
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

    // ── RES-1918: walker reaches impl methods, match arms, struct/collection
    //              literals, indexing, try/catch, interpolations, etc. ───────

    /// `impl` block methods are parsed as `Node::Function` values inside
    /// `Node::ImplBlock.methods`. The old walker iterated only top-level
    /// `Node::Function`, leaving every method body invisible. Attribution
    /// uses the parser-mangled `<StructName>$<method>` name.
    #[test]
    fn impl_block_methods_walked() {
        let src = r#"
            struct Point { int x, int y, }
            impl Point {
                fn distance(self) -> int { return self.x * self.x + self.y * self.y; }
            }
        "#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(
            m.iter()
                .any(|x| x.fn_name == "Point$distance" && x.kind == "arithmetic"),
            "method `Point$distance` body must yield arithmetic mutations (got {:?})",
            m
        );
    }

    /// `match` scrutinees and arm bodies were not in the old walker
    /// dispatch, so any mutation-bearing sub-expression inside them was
    /// silently dropped.
    #[test]
    fn match_arm_bodies_walked() {
        let src = r#"fn f(int x) -> int { return match x { 0 => 1, _ => x + 1, }; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(
            m.iter().any(|x| x.kind == "arithmetic"),
            "match arm `x + 1` must contribute an arithmetic mutation (got {:?})",
            m
        );
    }

    /// Match-arm guard expressions (`p if g => body`) must also be
    /// walked — they hold the same shape as any other condition.
    #[test]
    fn match_arm_guards_walked() {
        let src = r#"fn f(int x) -> int { return match x { y if y > 100 => 1, _ => 0, }; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(
            m.iter().any(|x| x.kind == "boundary"),
            "match guard `y > 100` must contribute a boundary mutation (got {:?})",
            m
        );
    }

    /// Struct-literal field values are expression positions; the integer
    /// literals `3` and `4` in `new Point { x: 3, y: 4 }` must each
    /// generate the standard literal-mutation triple.
    #[test]
    fn struct_literal_field_values_walked() {
        let src = r#"
            struct Point { int x, int y, }
            fn build() -> Point { return new Point { x: 3, y: 4 }; }
        "#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(
            m.iter().filter(|x| x.kind == "literal").count() >= 6,
            "two non-zero literals (3, 4) must each yield three mutations (got {:?})",
            m
        );
    }

    /// Array-literal element expressions must be walked.
    #[test]
    fn array_literal_items_walked() {
        let src = r#"fn f() -> IntArr { return [1, 2, 3]; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(
            m.iter().filter(|x| x.kind == "literal").count() >= 9,
            "three non-zero literals must each yield three mutations (got {:?})",
            m
        );
    }

    /// Map-literal keys and values are both expression positions.
    #[test]
    fn map_literal_entries_walked() {
        let src = r#"fn f() { let m = {"k" -> 1, "j" -> 2}; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(
            m.iter().filter(|x| x.kind == "literal").count() >= 4,
            "map literal entries must yield literal mutations for both keys and values (got {:?})",
            m
        );
    }

    /// Set-literal element expressions must be walked.
    #[test]
    fn set_literal_items_walked() {
        let src = r#"fn f() { let s = #{1, 2}; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(
            m.iter().filter(|x| x.kind == "literal").count() >= 6,
            "set literal items must yield literal mutations (got {:?})",
            m
        );
    }

    /// `arr[i]` indexing carries two expression positions (`arr` and
    /// `i`); both must be walked.
    #[test]
    fn index_expression_target_and_index_walked() {
        let src = r#"fn f(IntArr xs) -> int { return xs[1 + 1]; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(
            m.iter().any(|x| x.kind == "arithmetic"),
            "index expression `1 + 1` must produce an arithmetic mutation (got {:?})",
            m
        );
    }

    /// Field-assignment LHS and RHS are both expression positions.
    #[test]
    fn field_assignment_value_walked() {
        let src = r#"
            struct Holder { int x, }
            impl Holder { fn set(self) { self.x = 1 + 2; } }
        "#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(
            m.iter()
                .any(|x| x.fn_name == "Holder$set" && x.kind == "arithmetic"),
            "field assignment value `1 + 2` must produce arithmetic mutation (got {:?})",
            m
        );
    }

    /// `assert(<cond>)` carries the same expression shape as an `if`
    /// condition and must be walked.
    #[test]
    fn assert_condition_inside_fn_body_walked() {
        let src = r#"fn f(int x) { assert(x > 0); }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(
            m.iter().any(|x| x.kind == "boundary"),
            "assert condition `x > 0` must yield boundary mutation (got {:?})",
            m
        );
    }

    /// Lambda (`FunctionLiteral`) bodies carry the same expression
    /// shapes as named functions.
    #[test]
    fn lambda_body_walked() {
        let src = r#"fn f() { let inc = fn(int x) -> int { return x + 1; }; }"#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        assert!(
            m.iter().any(|x| x.kind == "arithmetic"),
            "lambda body `x + 1` must produce arithmetic mutation (got {:?})",
            m
        );
    }

    /// `try { ... } catch e { ... }` blocks must descend into both the
    /// try body and every catch handler body.
    #[test]
    fn try_catch_bodies_walked() {
        let src = r#"
            fn f() {
                try {
                    let a = 1 + 2;
                } catch e {
                    let b = 3 - 4;
                }
            }
        "#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        let arith = m.iter().filter(|x| x.kind == "arithmetic").count();
        assert!(
            arith >= 2,
            "both try body `1 + 2` and catch body `3 - 4` must yield arithmetic mutations (got {:?})",
            m
        );
    }

    /// Two top-level fns plus a method inside an impl block all
    /// contribute to the summary.
    #[test]
    fn summarize_groups_top_level_and_impl_methods() {
        let src = r#"
            fn add(int a, int b) -> int { return a + b; }
            struct S { int v, }
            impl S { fn bump(self) -> int { return self.v + 1; } }
        "#;
        let (prog, _) = parse(src);
        let m = generate(&prog);
        let s = summarize(&m);
        assert!(
            s.contains_key("add"),
            "top-level fn `add` must be in summary"
        );
        assert!(
            s.contains_key("S$bump"),
            "impl method `S$bump` must be in summary (got keys: {:?})",
            s.keys().collect::<Vec<_>>()
        );
    }
}
