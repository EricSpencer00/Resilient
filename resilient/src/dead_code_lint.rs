//! RES-2589: compile-time dead-code warnings over the source AST.
//!
//! Three checks, all warning-only (never fatal):
//!
//! 1. **Unused top-level functions** — a function declared at top level
//!    (or inside a module) with zero call sites anywhere in the program.
//!    Exempt: `main`, names starting with `_`, and `$`-mangled impl methods.
//!
//! 2. **Unreachable code after early exit** — statements in a `Block`
//!    that follow a `return`, `break`, or `continue` are dead.
//!
//! 3. **Unused let bindings** — a local `let name = …` where `name`
//!    never appears as an `Identifier` reference anywhere in the
//!    enclosing function body. Names starting with `_` are exempt.
//!
//! All diagnostics go to stderr in the form
//! `warning: <source_path>:<line>:<col>: <message>`.
//!
//! The pass is called from the `<EXTENSION_PASSES>` block in
//! `typechecker.rs` after the main type-check completes.

use crate::Node;
use crate::span::{Span, Spanned};
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run all dead-code lint checks on `program` and print warnings to stderr.
pub(crate) fn check(program: &Node, source_path: &str) {
    for msg in collect_warnings(program, source_path) {
        eprintln!("{msg}");
    }
}

/// Collect all dead-code warnings as strings (for testing).
pub(crate) fn collect_warnings(program: &Node, source_path: &str) -> Vec<String> {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return vec![],
    };

    let mut warnings = Vec::new();
    collect_unused_fn_warnings(stmts, source_path, &mut warnings);
    collect_unreachable_warnings_stmts(stmts, source_path, &mut warnings);
    collect_unused_var_warnings_stmts(stmts, source_path, &mut warnings);
    warnings
}

// ---------------------------------------------------------------------------
// 1. Unused top-level functions
// ---------------------------------------------------------------------------

fn collect_unused_fn_warnings(stmts: &[Spanned<Node>], source_path: &str, out: &mut Vec<String>) {
    let mut declared: HashMap<String, Span> = HashMap::new();
    let mut called: HashSet<String> = HashSet::new();

    collect_fn_declarations(stmts, &mut declared);
    for s in stmts {
        collect_called_names(&s.node, &mut called);
    }

    let mut names: Vec<&String> = declared.keys().collect();
    names.sort();
    for name in names {
        if called.contains(name) {
            continue;
        }
        if name == "main" || name.starts_with('_') || name.contains('$') {
            continue;
        }
        let span = declared[name];
        let loc = fmt_loc(source_path, span);
        out.push(format!("warning: {loc}: function `{name}` is never called"));
    }
}

fn collect_fn_declarations(stmts: &[Spanned<Node>], declared: &mut HashMap<String, Span>) {
    for s in stmts {
        match &s.node {
            Node::Function { name, span, .. } => {
                declared.entry(name.clone()).or_insert(*span);
            }
            Node::ModuleDecl { body, .. } => {
                for child in body {
                    if let Node::Function { name, span, .. } = child {
                        declared.entry(name.clone()).or_insert(*span);
                    }
                }
            }
            _ => {}
        }
    }
}

fn collect_called_names(node: &Node, called: &mut HashSet<String>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                called.insert(name.clone());
            }
            collect_called_names(function, called);
            for arg in arguments {
                collect_called_names(arg, called);
            }
        }
        Node::Program(stmts) => {
            for s in stmts {
                collect_called_names(&s.node, called);
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            ..
        } => {
            collect_called_names(body, called);
            for r in requires {
                collect_called_names(r, called);
            }
            for e in ensures {
                collect_called_names(e, called);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_called_names(s, called);
            }
        }
        Node::ExpressionStatement { expr, .. } => collect_called_names(expr, called),
        Node::LetStatement { value, .. } => collect_called_names(value, called),
        Node::StaticLet { value, .. } => collect_called_names(value, called),
        Node::Const { value, .. } => collect_called_names(value, called),
        Node::Assignment { value, .. } => collect_called_names(value, called),
        Node::ReturnStatement { value: Some(v), .. } => collect_called_names(v, called),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_called_names(condition, called);
            collect_called_names(consequence, called);
            if let Some(e) = alternative {
                collect_called_names(e, called);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_called_names(condition, called);
            collect_called_names(body, called);
        }
        Node::ForInStatement { iterable, body, .. } => {
            collect_called_names(iterable, called);
            collect_called_names(body, called);
        }
        Node::InfixExpression { left, right, .. } => {
            collect_called_names(left, called);
            collect_called_names(right, called);
        }
        Node::PrefixExpression { right, .. } => collect_called_names(right, called),
        Node::IndexExpression { target, index, .. } => {
            collect_called_names(target, called);
            collect_called_names(index, called);
        }
        Node::FieldAccess { target, .. } => collect_called_names(target, called),
        Node::FieldAssignment { target, value, .. } => {
            collect_called_names(target, called);
            collect_called_names(value, called);
        }
        Node::ArrayLiteral { items, .. } => {
            for e in items {
                collect_called_names(e, called);
            }
        }
        Node::TupleLiteral { items, .. } => {
            for e in items {
                collect_called_names(e, called);
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            for (_, v) in fields {
                collect_called_names(v, called);
            }
            if let Some(b) = base {
                collect_called_names(b, called);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            collect_called_names(scrutinee, called);
            for (_, guard, body) in arms {
                if let Some(g) = guard {
                    collect_called_names(g, called);
                }
                collect_called_names(body, called);
            }
        }
        Node::FunctionLiteral { body, .. } => collect_called_names(body, called),
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                collect_called_names(m, called);
            }
        }
        Node::ModuleDecl { body, .. } => {
            for child in body {
                collect_called_names(child, called);
            }
        }
        Node::TryExpression { expr, .. } => collect_called_names(expr, called),
        Node::OptionalChain { object, .. } => collect_called_names(object, called),
        Node::InterpolatedString { parts, .. } => {
            for part in parts {
                if let crate::string_interp::StringPart::Expr(e) = part {
                    collect_called_names(e, called);
                }
            }
        }
        Node::LiveBlock { body, .. } => collect_called_names(body, called),
        Node::Slice { target, lo, hi, .. } => {
            collect_called_names(target, called);
            if let Some(l) = lo {
                collect_called_names(l, called);
            }
            if let Some(h) = hi {
                collect_called_names(h, called);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// 2. Unreachable code after early exit
// ---------------------------------------------------------------------------

fn collect_unreachable_warnings_stmts(
    stmts: &[Spanned<Node>],
    source_path: &str,
    out: &mut Vec<String>,
) {
    for s in stmts {
        collect_unreachable_in_node(&s.node, source_path, out);
    }
}

fn collect_unreachable_in_node(node: &Node, source_path: &str, out: &mut Vec<String>) {
    match node {
        Node::Function { body, .. } => collect_unreachable_in_block_node(body, source_path, out),
        Node::Block { stmts, .. } => {
            check_block_for_unreachable(stmts, source_path, out);
            for s in stmts {
                collect_unreachable_in_node(s, source_path, out);
            }
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            collect_unreachable_in_block_node(consequence, source_path, out);
            if let Some(e) = alternative {
                collect_unreachable_in_block_node(e, source_path, out);
            }
        }
        Node::WhileStatement { body, .. } => {
            collect_unreachable_in_block_node(body, source_path, out);
        }
        Node::ForInStatement { body, .. } => {
            collect_unreachable_in_block_node(body, source_path, out);
        }
        Node::LiveBlock { body, .. } => {
            collect_unreachable_in_block_node(body, source_path, out);
        }
        Node::Match { arms, .. } => {
            for (_, _, body) in arms {
                collect_unreachable_in_block_node(body, source_path, out);
            }
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                collect_unreachable_in_node(m, source_path, out);
            }
        }
        Node::ModuleDecl { body, .. } => {
            for child in body {
                collect_unreachable_in_node(child, source_path, out);
            }
        }
        _ => {}
    }
}

fn collect_unreachable_in_block_node(node: &Node, source_path: &str, out: &mut Vec<String>) {
    if let Node::Block { stmts, .. } = node {
        check_block_for_unreachable(stmts, source_path, out);
        for s in stmts {
            collect_unreachable_in_node(s, source_path, out);
        }
    } else {
        collect_unreachable_in_node(node, source_path, out);
    }
}

fn check_block_for_unreachable(stmts: &[Node], source_path: &str, out: &mut Vec<String>) {
    let mut found_terminator = false;
    for stmt in stmts {
        if found_terminator {
            let span = node_span(stmt);
            if span.start.line > 0 {
                let loc = fmt_loc(source_path, span);
                out.push(format!("warning: {loc}: unreachable code after early exit"));
            }
        }
        if is_terminator(stmt) {
            found_terminator = true;
        }
    }
}

fn is_terminator(node: &Node) -> bool {
    matches!(
        node,
        Node::ReturnStatement { .. }
            | Node::Break { .. }
            | Node::Continue { .. }
            | Node::BreakLabel { .. }
            | Node::ContinueLabel { .. }
    )
}

// ---------------------------------------------------------------------------
// 3. Unused let bindings
// ---------------------------------------------------------------------------

fn collect_unused_var_warnings_stmts(
    stmts: &[Spanned<Node>],
    source_path: &str,
    out: &mut Vec<String>,
) {
    for s in stmts {
        collect_unused_var_in_node(&s.node, source_path, out);
    }
}

fn collect_unused_var_in_node(node: &Node, source_path: &str, out: &mut Vec<String>) {
    match node {
        Node::Function { body, .. } => {
            check_fn_body_for_unused_vars(body, source_path, out);
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                collect_unused_var_in_node(m, source_path, out);
            }
        }
        Node::ModuleDecl { body, .. } => {
            for child in body {
                collect_unused_var_in_node(child, source_path, out);
            }
        }
        _ => {}
    }
}

fn check_fn_body_for_unused_vars(body: &Node, source_path: &str, out: &mut Vec<String>) {
    let stmts = match body {
        Node::Block { stmts, .. } => stmts,
        _ => return,
    };

    let mut bindings: Vec<(String, Span)> = Vec::new();
    collect_let_bindings(stmts, &mut bindings);

    if bindings.is_empty() {
        return;
    }

    let mut reads: HashSet<String> = HashSet::new();
    collect_identifier_reads(body, &mut reads);

    for (name, span) in &bindings {
        if name.starts_with('_') || name.is_empty() {
            continue;
        }
        if reads.contains(name.as_str()) {
            continue;
        }
        if span.start.line > 0 {
            let loc = fmt_loc(source_path, *span);
            out.push(format!(
                "warning: {loc}: variable `{name}` is assigned but never read"
            ));
        }
    }
}

fn collect_let_bindings(stmts: &[Node], out: &mut Vec<(String, Span)>) {
    for stmt in stmts {
        match stmt {
            Node::LetStatement { name, span, .. } => {
                out.push((name.clone(), *span));
            }
            Node::Block { stmts, .. } => collect_let_bindings(stmts, out),
            Node::IfStatement {
                consequence,
                alternative,
                ..
            } => {
                collect_let_bindings_in_node(consequence, out);
                if let Some(e) = alternative {
                    collect_let_bindings_in_node(e, out);
                }
            }
            Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
                collect_let_bindings_in_node(body, out);
            }
            _ => {}
        }
    }
}

fn collect_let_bindings_in_node(node: &Node, out: &mut Vec<(String, Span)>) {
    if let Node::Block { stmts, .. } = node {
        collect_let_bindings(stmts, out);
    }
}

/// Collect all `Identifier` names appearing in *read* positions.
/// Excludes the LHS `name` of `Assignment` and the binding `name` of `LetStatement`.
fn collect_identifier_reads(node: &Node, reads: &mut HashSet<String>) {
    match node {
        Node::Identifier { name, .. } => {
            reads.insert(name.clone());
        }
        Node::Assignment { name: _, value, .. } => {
            collect_identifier_reads(value, reads);
        }
        Node::LetStatement { name: _, value, .. } => {
            collect_identifier_reads(value, reads);
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_identifier_reads(s, reads);
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            ..
        } => {
            collect_identifier_reads(body, reads);
            for r in requires {
                collect_identifier_reads(r, reads);
            }
            for e in ensures {
                collect_identifier_reads(e, reads);
            }
        }
        Node::ExpressionStatement { expr, .. } => collect_identifier_reads(expr, reads),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            collect_identifier_reads(function, reads);
            for a in arguments {
                collect_identifier_reads(a, reads);
            }
        }
        Node::InfixExpression { left, right, .. } => {
            collect_identifier_reads(left, reads);
            collect_identifier_reads(right, reads);
        }
        Node::PrefixExpression { right, .. } => collect_identifier_reads(right, reads),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_identifier_reads(condition, reads);
            collect_identifier_reads(consequence, reads);
            if let Some(e) = alternative {
                collect_identifier_reads(e, reads);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_identifier_reads(condition, reads);
            collect_identifier_reads(body, reads);
        }
        Node::ForInStatement { iterable, body, .. } => {
            collect_identifier_reads(iterable, reads);
            collect_identifier_reads(body, reads);
        }
        Node::ReturnStatement { value: Some(v), .. } => collect_identifier_reads(v, reads),
        Node::FieldAccess { target, .. } => collect_identifier_reads(target, reads),
        Node::FieldAssignment { target, value, .. } => {
            collect_identifier_reads(target, reads);
            collect_identifier_reads(value, reads);
        }
        Node::IndexExpression { target, index, .. } => {
            collect_identifier_reads(target, reads);
            collect_identifier_reads(index, reads);
        }
        Node::Slice { target, lo, hi, .. } => {
            collect_identifier_reads(target, reads);
            if let Some(l) = lo {
                collect_identifier_reads(l, reads);
            }
            if let Some(h) = hi {
                collect_identifier_reads(h, reads);
            }
        }
        Node::ArrayLiteral { items, .. } => {
            for e in items {
                collect_identifier_reads(e, reads);
            }
        }
        Node::TupleLiteral { items, .. } => {
            for e in items {
                collect_identifier_reads(e, reads);
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            for (_, v) in fields {
                collect_identifier_reads(v, reads);
            }
            if let Some(b) = base {
                collect_identifier_reads(b, reads);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            collect_identifier_reads(scrutinee, reads);
            for (_, guard, body) in arms {
                if let Some(g) = guard {
                    collect_identifier_reads(g, reads);
                }
                collect_identifier_reads(body, reads);
            }
        }
        Node::FunctionLiteral { body, .. } => collect_identifier_reads(body, reads),
        Node::TryExpression { expr, .. } => collect_identifier_reads(expr, reads),
        Node::OptionalChain { object, .. } => collect_identifier_reads(object, reads),
        Node::StaticLet { value, .. } => collect_identifier_reads(value, reads),
        Node::Const { value, .. } => collect_identifier_reads(value, reads),
        Node::LiveBlock { body, .. } => collect_identifier_reads(body, reads),
        Node::InterpolatedString { parts, .. } => {
            for part in parts {
                if let crate::string_interp::StringPart::Expr(e) = part {
                    collect_identifier_reads(e, reads);
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fmt_loc(source_path: &str, span: Span) -> String {
    if span.start.line == 0 {
        source_path.to_string()
    } else {
        format!("{}:{}:{}", source_path, span.start.line, span.start.column)
    }
}

fn node_span(node: &Node) -> Span {
    match node {
        Node::LetStatement { span, .. }
        | Node::StaticLet { span, .. }
        | Node::Const { span, .. }
        | Node::Assignment { span, .. }
        | Node::ReturnStatement { span, .. }
        | Node::Break { span }
        | Node::Continue { span }
        | Node::BreakLabel { span, .. }
        | Node::ContinueLabel { span, .. }
        | Node::IfStatement { span, .. }
        | Node::WhileStatement { span, .. }
        | Node::ForInStatement { span, .. }
        | Node::Block { span, .. }
        | Node::Function { span, .. }
        | Node::Identifier { span, .. }
        | Node::CallExpression { span, .. }
        | Node::InfixExpression { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::FieldAccess { span, .. }
        | Node::FieldAssignment { span, .. }
        | Node::IndexExpression { span, .. }
        | Node::Match { span, .. }
        | Node::StructLiteral { span, .. }
        | Node::ArrayLiteral { span, .. }
        | Node::TupleLiteral { span, .. }
        | Node::ExpressionStatement { span, .. }
        | Node::InterpolatedString { span, .. } => *span,
        Node::IntegerLiteral { span, .. }
        | Node::FloatLiteral { span, .. }
        | Node::StringLiteral { span, .. } => *span,
        _ => Span::default(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::{Lexer, Parser};

    fn parse_src(src: &str) -> crate::Node {
        let lexer = Lexer::new(src);
        let mut parser = Parser::new(lexer);
        parser.parse_program()
    }

    fn warnings(src: &str) -> Vec<String> {
        let prog = parse_src(src);
        super::collect_warnings(&prog, "test.rz")
    }

    // ---- unused functions ----

    #[test]
    fn unused_function_warns() {
        let src = "fn unused_fn() -> int { 42 } fn main() { println(\"hello\"); }";
        let w = warnings(src);
        assert!(
            w.iter().any(|m| m.contains("unused_fn")),
            "expected warning about unused_fn, got: {w:?}"
        );
    }

    #[test]
    fn used_function_no_warn() {
        let src =
            "fn helper() -> int { 42 } fn main() { let x = helper(); println(to_string(x)); }";
        let w = warnings(src);
        assert!(
            !w.iter().any(|m| m.contains("function `helper`")),
            "should not warn about used helper, got: {w:?}"
        );
    }

    #[test]
    fn main_function_never_warned() {
        let src = "fn main() { println(\"hi\"); }";
        let w = warnings(src);
        assert!(
            !w.iter().any(|m| m.contains("function `main`")),
            "should not warn about main, got: {w:?}"
        );
    }

    #[test]
    fn underscore_prefix_suppresses_fn_warn() {
        let src = "fn _unused() -> int { 0 } fn main() { println(\"ok\"); }";
        let w = warnings(src);
        assert!(
            !w.iter().any(|m| m.contains("_unused")),
            "should not warn about _-prefixed fn, got: {w:?}"
        );
    }

    #[test]
    fn recursive_function_not_warned() {
        // fib calls itself, and is called from main.
        let src = "fn fib(int n) -> int { if n <= 1 { n } else { fib(n-1) + fib(n-2) } } fn main() { fib(5); }";
        let w = warnings(src);
        assert!(
            !w.iter().any(|m| m.contains("function `fib`")),
            "recursive fn should not warn, got: {w:?}"
        );
    }

    // ---- unreachable code ----

    #[test]
    fn unreachable_after_return_warns() {
        let src = "fn foo() -> int { return 1; let x = 2; x } fn main() { foo(); }";
        let w = warnings(src);
        assert!(
            w.iter().any(|m| m.contains("unreachable")),
            "expected unreachable warning, got: {w:?}"
        );
    }

    #[test]
    fn no_unreachable_without_terminator() {
        let src = "fn foo() -> int { let x = 1; x + 1 } fn main() { foo(); }";
        let w = warnings(src);
        assert!(
            !w.iter().any(|m| m.contains("unreachable")),
            "should not warn about reachable code, got: {w:?}"
        );
    }

    // ---- unused variables ----

    #[test]
    fn unused_variable_warns() {
        let src = "fn main() { let unused_var = 42; println(\"done\"); }";
        let w = warnings(src);
        assert!(
            w.iter().any(|m| m.contains("unused_var")),
            "expected warning about unused_var, got: {w:?}"
        );
    }

    #[test]
    fn used_variable_no_warn() {
        let src = "fn main() { let x = 42; println(to_string(x)); }";
        let w = warnings(src);
        assert!(
            !w.iter().any(|m| m.contains("variable `x`")),
            "should not warn about used variable, got: {w:?}"
        );
    }

    #[test]
    fn underscore_variable_suppresses_warn() {
        let src = "fn main() { let _ignored = 42; println(\"ok\"); }";
        let w = warnings(src);
        assert!(
            !w.iter().any(|m| m.contains("_ignored")),
            "should not warn about _-prefixed variable, got: {w:?}"
        );
    }

    #[test]
    fn variable_used_in_call_no_warn() {
        let src = "fn main() { let msg = \"hello\"; println(msg); }";
        let w = warnings(src);
        assert!(
            !w.iter().any(|m| m.contains("variable `msg`")),
            "should not warn about variable used in call, got: {w:?}"
        );
    }
}
