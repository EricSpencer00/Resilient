//! RES-296 (RES-128 follow-up): named type synonyms — `type Meters = float;`.
//!
//! Type aliases let authors name domain types (meters, volts, seconds)
//! without the runtime cost of a full newtype. They are *transparent*
//! to the type checker: `Meters` and `float` unify everywhere they meet.
//!
//! ## What this module owns
//!
//! - [`check`]: the program-level pass invoked from `<EXTENSION_PASSES>`
//!   in `typechecker.rs`. Walks every top-level `Node::TypeAlias`,
//!   builds the `name -> target` table, and rejects any *circular*
//!   alias definition (`type A = B; type B = A;`) eagerly with a
//!   `type alias cycle: A -> B -> A` diagnostic.
//!
//! ## What lives elsewhere (and why)
//!
//! - **Token + keyword + AST node**: `main.rs` (`Token::Type`,
//!   `"type" => Token::Type`, `Node::TypeAlias { name, target, span }`).
//!   Predates the feature-isolation refactor — kept in place to avoid
//!   churning every consumer (`free_vars`, `compiler`, `formatter`,
//!   `lsp_server`, `jit_backend`).
//! - **Logos token**: `lexer_logos.rs` (`#[token("type")] Type`).
//! - **Parser**: `Parser::parse_type_alias` in `main.rs`.
//! - **Lazy alias expansion**: `TypeChecker::parse_type_name` in
//!   `typechecker.rs` — the actual structural-equivalence step that
//!   makes `Meters` interchangeable with `float`.
//!
//! The eager-cycle pass in this module complements the lazy one: it
//! catches dead aliases (`type A = B; type B = A;` with neither used)
//! that would otherwise slip past the type checker because cycle
//! detection only fires when an alias is *referenced*. The ticket
//! lists "circular aliases are a type error" as an unconditional
//! acceptance criterion, so we surface it at module load time.
//!
//! ## Cycle detection algorithm
//!
//! Standard DFS over the alias graph. Each node is in one of three
//! colors: `Unvisited`, `OnStack`, or `Done`. A back-edge to a stack
//! node is a cycle; we reconstruct the chain by truncating the visit
//! path back to the offending node and rendering `A -> B -> A`.
//! Self-loops (`type A = A;`) fall out as the degenerate single-node
//! case.
//!
//! Aliases whose target is *not* another alias (e.g. `type M = int;`)
//! act as terminals — DFS bottoms out without ever visiting them.

use crate::Node;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Color {
    Unvisited,
    OnStack,
    Done,
}

/// RES-296: top-level pass — register every `type X = T;` declaration
/// and reject circular alias definitions.
///
/// Returns `Err` on the first cycle found; the message embeds
/// `<source_path>:<line>:<col>` for the offending alias declaration.
/// Non-cycle aliases are silently accepted; the typechecker's
/// `parse_type_name` does the actual transparent expansion at use
/// sites.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let Node::Program(statements) = program else {
        return Ok(());
    };

    // RES-1230 / RES-2312: the typechecker's `<EXTENSION_PASSES>`
    // dispatch gates this call behind `markers.has_type_alias`, so
    // the program is guaranteed to contain at least one
    // `Node::TypeAlias`. The previous internal `stmts.iter().any(...)`
    // pre-scan walked the full top-level statement list a second
    // time for the same signal Markers already computed during the
    // shared whole-AST walk. Mirrors RES-2292 through RES-2310.

    // RES-1537: borrow alias/target names from the AST throughout the
    // cycle detector. The previous shape cloned each alias name into
    // `aliases` keys, `decl_order` entries, and a fresh `color`
    // HashMap key per entry — plus `node.to_string()` per DFS visit
    // and `target.clone()` to close a detected cycle. None of the
    // owned strings were needed: lookups, equality checks, and the
    // final `chain.join(" -> ")` all work on `&str`. Mirror of
    // RES-1514 (SCC DFS) and RES-1517 (full_modules DFS) applied to
    // type-alias cycles.
    // RES-1790: pre-size both to statements.len() — at most one
    // TypeAlias per top-level statement, so this is an upper bound.
    let mut aliases: std::collections::HashMap<&str, (&str, crate::span::Span)> =
        std::collections::HashMap::with_capacity(statements.len());
    let mut decl_order: Vec<&str> = Vec::with_capacity(statements.len());

    for spanned in statements {
        if let Node::TypeAlias { name, target, span } = &spanned.node {
            if name.is_empty() {
                // Parser recovery already emitted a diagnostic; skip.
                continue;
            }
            let k = name.as_str();
            if !aliases.contains_key(k) {
                decl_order.push(k);
            }
            aliases.insert(k, (target.as_str(), *span));
        }
    }

    let mut color: std::collections::HashMap<&str, Color> =
        aliases.keys().map(|k| (*k, Color::Unvisited)).collect();

    for &start in &decl_order {
        if color[start] != Color::Unvisited {
            continue;
        }
        // RES-1790: pre-size to aliases.len() — DFS chain peaks at
        // total alias count.
        let mut path: Vec<&str> = Vec::with_capacity(aliases.len());
        if let Some(chain) = dfs(start, &aliases, &mut color, &mut path) {
            // Anchor the diagnostic to the first alias in the cycle —
            // its span points users at a real declaration in source.
            let (_, span) = &aliases[chain[0]];
            return Err(format!(
                "{}:{}:{}: type alias cycle: {}",
                source_path,
                span.start.line,
                span.start.column,
                chain.join(" -> ")
            ));
        }
    }

    Ok(())
}

/// DFS over the alias graph. Returns `Some(chain)` on the first
/// back-edge into the current stack; `chain` is the cycle in source
/// order with the re-entered node repeated at the end (`A -> B -> A`).
fn dfs<'a>(
    node: &'a str,
    aliases: &std::collections::HashMap<&'a str, (&'a str, crate::span::Span)>,
    color: &mut std::collections::HashMap<&'a str, Color>,
    path: &mut Vec<&'a str>,
) -> Option<Vec<&'a str>> {
    color.insert(node, Color::OnStack);
    path.push(node);

    let target = aliases[node].0;
    if let Some(c) = color.get(target).copied() {
        match c {
            Color::OnStack => {
                // Cycle. Reconstruct the chain from the first
                // occurrence of `target` in `path` to the end, then
                // append `target` again to close the loop visually.
                let cut = path.iter().position(|n| *n == target).unwrap_or(0);
                let mut chain: Vec<&'a str> = path[cut..].to_vec();
                chain.push(target);
                return Some(chain);
            }
            Color::Unvisited => {
                if let Some(chain) = dfs(target, aliases, color, path) {
                    return Some(chain);
                }
            }
            Color::Done => {
                // Already fully explored — no cycle from here.
            }
        }
    }
    // Else: target is not an alias — it's a terminal type name. No edge.

    path.pop();
    color.insert(node, Color::Done);
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn run_check(src: &str) -> Result<(), String> {
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        check(&program, "<test>")
    }

    #[test]
    fn simple_alias_is_accepted() {
        let src = "type M = int;\n";
        run_check(src).expect("non-circular alias must pass");
    }

    #[test]
    fn three_parallel_aliases_to_same_terminal_are_accepted() {
        let src = "\
            type Meters = float;\n\
            type Volts = float;\n\
            type Seconds = float;\n\
        ";
        run_check(src).expect("parallel aliases share no edges");
    }

    #[test]
    fn alias_chain_to_terminal_is_accepted() {
        // A -> B -> C -> int. No back-edge.
        let src = "\
            type A = B;\n\
            type B = C;\n\
            type C = int;\n\
        ";
        run_check(src).expect("acyclic chain must pass");
    }

    #[test]
    fn two_node_cycle_is_rejected() {
        let src = "\
            type A = B;\n\
            type B = A;\n\
        ";
        let err = run_check(src).expect_err("A <-> B is a cycle");
        assert!(
            err.contains("type alias cycle"),
            "expected cycle diagnostic, got: {}",
            err
        );
        assert!(
            err.contains("A") && err.contains("B"),
            "chain must mention both nodes, got: {}",
            err
        );
    }

    #[test]
    fn three_node_cycle_is_rejected() {
        let src = "\
            type A = B;\n\
            type B = C;\n\
            type C = A;\n\
        ";
        let err = run_check(src).expect_err("A -> B -> C -> A is a cycle");
        assert!(err.contains("type alias cycle"), "got: {}", err);
        assert!(
            err.contains("A") && err.contains("B") && err.contains("C"),
            "chain must mention all three, got: {}",
            err
        );
    }

    #[test]
    fn self_loop_is_rejected() {
        let src = "type A = A;\n";
        let err = run_check(src).expect_err("self-loop is a cycle");
        assert!(err.contains("type alias cycle"), "got: {}", err);
    }

    #[test]
    fn cycle_diagnostic_carries_source_position() {
        let src = "\
            type A = B;\n\
            type B = A;\n\
        ";
        let err = run_check(src).expect_err("cycle");
        // Format: <path>:<line>:<col>: type alias cycle: ...
        assert!(
            err.starts_with("<test>:"),
            "expected source position prefix, got: {}",
            err
        );
    }

    #[test]
    fn end_to_end_alias_in_fn_signature_typechecks() {
        // Alias used as both parameter and return type. The
        // typechecker's lazy expansion makes this work; the eager
        // pass here just doesn't reject it.
        let src = "\
            type Meters = float;\n\
            type Seconds = float;\n\
            fn travel(Meters d, Seconds t) -> Meters { return d; }\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        check(&program, "<test>").expect("no cycle");
        let mut tc = crate::typechecker::TypeChecker::new();
        tc.check_program_with_source(&program, "<test>")
            .expect("alias-typed signature must check");
    }

    #[test]
    fn end_to_end_alias_in_let_binding_typechecks() {
        let src = "\
            type Meters = float;\n\
            let m: Meters = 100.0;\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        check(&program, "<test>").expect("no cycle");
        let mut tc = crate::typechecker::TypeChecker::new();
        tc.check_program_with_source(&program, "<test>")
            .expect("alias-typed let must check");
    }
}
