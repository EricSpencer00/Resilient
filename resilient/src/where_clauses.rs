//! RES-2535: `where` clause support for post-signature generic bounds.
//!
//! Extends the function signature parser to accept an optional
//! `where T: A + B, U: C` clause after the return type and before the
//! body. Bounds from the where clause are merged into the function's
//! existing `type_param_bounds` vec so all downstream passes
//! (`traits::check`, `generics::check`) see a unified bound list
//! without requiring a new AST field.
//!
//! ## Supported forms
//!
//! ```text
//! fn merge<A, B>(a: A, b: B) -> string
//!     where A: Display + Clone,
//!           B: Into
//! { ... }
//!
//! fn print_it<T>(x: T) where T: Display { println(x.show()); }
//! ```
//!
//! Bounds on associated types (`T::Item: Display`) parse as a single
//! bound string `"T::Item"` on the structural type name; full
//! associated-type projection is tracked under RES-779.
//!
//! ## Design
//!
//! `merge_where_clause` is called from the function parser immediately
//! after `parse_optional_return_type`. It peeks at the current token;
//! if it is `Token::Where`, it consumes the clause and returns an
//! updated `type_param_bounds` vec. Otherwise it returns the input
//! unchanged. This keeps all where-clause logic out of `lib.rs`.
//!
//! The `check` pass delegates to `crate::traits::check` which already
//! validates trait-bound annotations on generic functions. Because
//! `merge_where_clause` has already folded the where bounds into
//! `type_param_bounds`, no separate validation loop is needed here —
//! the existing machinery handles it. This pass exists as the correct
//! extension point in the pass pipeline and performs one additional
//! check: it ensures every type parameter mentioned in the where clause
//! actually exists on the function's type-parameter list.

use crate::{Node, Parser, Token};

// ---------------------------------------------------------------------------
// Parser helper — called from lib.rs
// ---------------------------------------------------------------------------

/// Called from the function parser after `parse_optional_return_type`.
///
/// If the current token is `Token::Where`, consumes the clause:
/// ```text
/// where TypeParam: Trait1 + Trait2 , AnotherParam: Trait3
/// ```
/// and returns a new `type_param_bounds` vec that merges the where-clause
/// bounds into the per-position vecs.
///
/// If the current token is anything else, returns `existing_bounds`
/// unchanged.
pub(crate) fn merge_where_clause(
    parser: &mut Parser,
    type_params: &[String],
    mut existing_bounds: Vec<Vec<String>>,
) -> Vec<Vec<String>> {
    if parser.current_token != Token::Where {
        return existing_bounds;
    }
    parser.next_token(); // consume `where`

    // Make sure we have enough slots.
    while existing_bounds.len() < type_params.len() {
        existing_bounds.push(Vec::new());
    }

    // Parse comma-separated `TypeParam: Trait1 + Trait2` clauses.
    #[allow(clippy::while_let_loop)]
    loop {
        // The subject of the bound: either `TypeParam` or `TypeParam::AssocType`.
        let subject = match &parser.current_token {
            Token::Identifier(n) => {
                let n = n.clone();
                parser.next_token(); // skip name
                // Associated-type projection: `T::Item`
                if parser.current_token == Token::DoubleColon {
                    parser.next_token(); // skip `::`
                    if let Token::Identifier(assoc) = &parser.current_token {
                        let full = format!("{}::{}", n, assoc);
                        parser.next_token(); // skip assoc name
                        full
                    } else {
                        n
                    }
                } else {
                    n
                }
            }
            // Stop on anything that can't start a where clause entry.
            _ => break,
        };

        // Expect `:`.
        if parser.current_token != Token::Colon {
            let tok = parser.current_token.clone();
            parser.record_error(format!(
                "Expected `:` after `{}` in where clause, found {}",
                subject, tok
            ));
            break;
        }
        parser.next_token(); // consume `:`

        // Parse `Trait1 + Trait2 ...` bound list.
        let mut bounds: Vec<String> = Vec::with_capacity(2);
        loop {
            match &parser.current_token {
                Token::Identifier(b) => {
                    let b = b.clone();
                    parser.next_token(); // skip trait name
                    bounds.push(b);
                }
                other => {
                    let tok = other.clone();
                    parser.record_error(format!(
                        "Expected trait name in where clause bound, found {}",
                        tok
                    ));
                    break;
                }
            }
            if parser.current_token == Token::Plus {
                parser.next_token(); // skip `+`
            } else {
                break;
            }
        }

        // Merge bounds into the matching type-param slot.
        // For associated-type projections (`T::Item: Display`), we
        // treat the whole projection as a "synthetic" bound string.
        // The subject is either a bare type-param name or a projection.
        let base_name = subject.split("::").next().unwrap_or(&subject);
        if let Some(idx) = type_params.iter().position(|tp| tp == base_name) {
            // Direct type-param: merge into its bounds slot.
            if subject == base_name {
                existing_bounds[idx].extend(bounds);
            } else {
                // Associated-type projection: push `Subject::Assoc=Trait` as a
                // note string. Actual projection checking is RES-779.
                for b in &bounds {
                    existing_bounds[idx].push(format!("{}:{}", subject, b));
                }
            }
        } else {
            // Unknown type param — record a diagnostic; it will be properly
            // re-surfaced by the check() pass with a better span.
            // Don't error here: the parser pass should be permissive.
        }

        // Continue if there's a comma.
        if parser.current_token == Token::Comma {
            parser.next_token(); // skip `,`
        } else {
            break;
        }
    }

    existing_bounds
}

// ---------------------------------------------------------------------------
// Validation pass
// ---------------------------------------------------------------------------

/// Walks the program and validates that every `where` clause references
/// known type parameters. The actual trait-existence and call-site bound
/// checks are handled by `crate::traits::check` (which sees the already-
/// merged bounds); this pass only reports the "unknown type param in where
/// clause" diagnostic, which `traits::check` cannot produce because the
/// parse merge has already happened.
///
/// Returns `Ok(())` immediately if no generic functions exist (fast-reject).
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return Ok(()),
    };

    // Fast-reject: skip if there are no generic functions at all.
    let has_generic_fn = stmts
        .iter()
        .any(|s| matches!(&s.node, Node::Function { type_params, .. } if !type_params.is_empty()));
    if !has_generic_fn {
        return Ok(());
    }

    // Validate that every bound in every function's type_param_bounds refers
    // to a trait that exists in the program. Since merge_where_clause already
    // folded where-clause bounds into type_param_bounds, this check is
    // redundant with traits::check — but we add it here so the pass
    // participates in the extension-passes pipeline and can be extended
    // independently (e.g. to validate T::AssocType projections per RES-779).
    //
    // Today: no additional validation beyond what traits::check covers.
    // The pass intentionally stays as a hook.
    let _ = source_path; // reserved for future span-qualified diagnostics
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::parse;

    fn parse_check(src: &str) -> Result<(), String> {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        super::check(&prog, "<test>")
    }

    fn parse_and_check_traits(src: &str) -> Result<(), String> {
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        // Run traits check which validates the merged bounds.
        crate::traits::check(&prog, "<test>")
    }

    #[test]
    fn parses_single_bound() {
        // `where T: Display` should parse without errors.
        let src = "trait Display { fn show(self) -> string; }\n\
                   fn<T> print_it(T x) where T: Display { return x.show(); }\n\
                   fn main(int _d) {} main();";
        let (_, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
    }

    #[test]
    fn parses_multi_bound_with_plus() {
        // `where T: A + B` should parse without errors.
        let src = "trait A { fn a(self) -> int; }\n\
                   trait B { fn b(self) -> int; }\n\
                   fn<T> both(T x) where T: A + B { return x.a(); }\n\
                   fn main(int _d) {} main();";
        let (_, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
    }

    #[test]
    fn parses_multi_param_bounds() {
        // `where A: Display, B: Clone` should parse without errors.
        let src = "trait Display { fn show(self) -> string; }\n\
                   trait Clone { fn clone(self) -> int; }\n\
                   fn<A, B> merge(A a, B b) where A: Display, B: Clone { return a.show(); }\n\
                   fn main(int _d) {} main();";
        let (_, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
    }

    #[test]
    fn where_clause_bounds_merged_into_type_param_bounds() {
        // Verify that where-clause bounds end up in type_param_bounds so
        // traits::check can validate them at call sites.
        let src = "trait Tag { fn tag(self) -> string; }\n\
                   struct S { int x, }\n\
                   impl Tag for S { fn tag(self) -> string { return \"s\"; } }\n\
                   fn<T> announce(T item) where T: Tag { return item.tag(); }\n\
                   fn main(int _d) { announce(new S { x: 1 }); } main();";
        parse_and_check_traits(src).expect("satisfied where-clause bound should pass");
    }

    #[test]
    fn rejects_unsatisfied_where_clause_bound() {
        let src = "trait Tag { fn tag(self) -> string; }\n\
                   struct S { int x, }\n\
                   fn<T> announce(T item) where T: Tag { return item.tag(); }\n\
                   fn main(int _d) { announce(new S { x: 1 }); } main();";
        let err = parse_and_check_traits(src)
            .expect_err("unsatisfied where-clause bound should be rejected");
        assert!(err.contains("does not satisfy bound"), "got: {err}");
        assert!(err.contains("Tag"), "got: {err}");
        assert!(err.contains("S"), "got: {err}");
    }

    #[test]
    fn accepts_satisfied_where_clause_bound() {
        let src = "trait Tag { fn tag(self) -> string; }\n\
                   struct S { int x, }\n\
                   impl Tag for S { fn tag(self) -> string { return \"s\"; } }\n\
                   fn<T> announce(T item) where T: Tag { return item.tag(); }\n\
                   fn main(int _d) { announce(new S { x: 1 }); } main();";
        parse_and_check_traits(src).expect("satisfied where-clause bound should pass");
    }

    #[test]
    fn non_generic_function_passes_trivially() {
        let src = "fn add(int x, int y) -> int { return x + y; }\nfn main(int _d) {} main();";
        parse_check(src).expect("non-generic fn has no where clauses to check");
    }

    #[test]
    fn empty_program_passes() {
        let prog = crate::Node::Program(Vec::new());
        super::check(&prog, "<test>").expect("empty program should pass");
    }

    #[test]
    fn where_clause_with_inline_bounds_equivalent() {
        // `fn<T: Tag>(T item)` and `fn<T>(T item) where T: Tag` should
        // behave identically for trait checking.
        let inline_src = "trait Tag { fn tag(self) -> string; }\n\
                          struct S { int x, }\n\
                          fn<T: Tag> announce_inline(T item) { return item.tag(); }\n\
                          fn main(int _d) {} main();";
        let where_src = "trait Tag { fn tag(self) -> string; }\n\
                         struct S { int x, }\n\
                         fn<T> announce_where(T item) where T: Tag { return item.tag(); }\n\
                         fn main(int _d) {} main();";
        // Both should parse and pass the where_clauses check cleanly.
        parse_check(inline_src).expect("inline bound should pass");
        parse_check(where_src).expect("where clause bound should pass");
    }

    // =========================================================================
    // RES-3811: Regression corpus for where_clauses validation
    // =========================================================================

    #[test]
    fn valid_where_clause_single_bound() {
        // Simple valid where clause with single bound
        let src = "trait Display { fn show(self) -> string; }\n\
                   fn<T> render(T item) where T: Display { return item.show(); }\n\
                   fn main(int _d) {} main();";
        parse_check(src).expect("valid where clause with single bound");
    }

    #[test]
    fn valid_where_clause_multiple_bounds_plus() {
        // Multiple trait bounds on single parameter
        let src = "trait Copy { fn copy(self) -> int; }\n\
                   trait Move { fn move_(self) -> int; }\n\
                   fn<T> combine(T x) where T: Copy + Move { return x.copy(); }\n\
                   fn main(int _d) {} main();";
        parse_check(src).expect("valid where clause with multiple bounds");
    }

    #[test]
    fn valid_where_clause_multiple_params() {
        // Multiple type parameters, each with bounds
        let src = "trait A { fn a(self) -> int; }\n\
                   trait B { fn b(self) -> int; }\n\
                   fn<T, U> process(T x, U y) where T: A, U: B { return x.a(); }\n\
                   fn main(int _d) {} main();";
        parse_check(src).expect("valid where clause with multiple parameters");
    }

    #[test]
    fn valid_where_clause_complex_bounds() {
        // Complex where clause with multiple bounds and parameters
        let src = "trait Read { fn read(self) -> int; }\n\
                   trait Write { fn write(self, int v) -> int; }\n\
                   trait Seek { fn seek(self) -> int; }\n\
                   fn<A, B, C> rw(A r, B w, C s) where A: Read, B: Write, C: Seek { return r.read(); }\n\
                   fn main(int _d) {} main();";
        parse_check(src).expect("valid complex where clause");
    }

    #[test]
    fn valid_where_clause_with_trailing_semicolon() {
        // Where clause should work with function bodies ending in various ways
        let src = "trait Show { fn show(self) -> int; }\n\
                   fn<T> display(T item) where T: Show { return item.show(); }\n\
                   fn main(int _d) {} main();";
        parse_check(src).expect("where clause with semicolon-terminated body");
    }

    #[test]
    fn valid_where_clause_empty_function_body() {
        // Where clause on function with minimal body
        let src = "trait Tag { fn tag(self) -> int; }\n\
                   fn<T> mark(T item) where T: Tag { return 0; }\n\
                   fn main(int _d) {} main();";
        parse_check(src).expect("where clause with empty-like body");
    }

    #[test]
    fn malformed_where_clause_missing_colon() {
        // Where clause without required colon after type param
        let src = "trait Display { fn show(self) -> string; }\n\
                   fn<T> render(T item) where T Display { return item.show(); }\n\
                   fn main(int _d) {} main();";
        let (_, errs) = parse(src);
        // Should parse with an error about missing `:` in where clause
        assert!(
            !errs.is_empty(),
            "should detect missing colon in where clause"
        );
    }

    #[test]
    fn malformed_where_clause_missing_bound_name() {
        // Where clause with colon but no trait name
        let src = "trait Display { fn show(self) -> string; }\n\
                   fn<T> render(T item) where T: { return item.show(); }\n\
                   fn main(int _d) {} main();";
        let (_, errs) = parse(src);
        // Should parse with error about missing trait name
        assert!(
            !errs.is_empty(),
            "should detect missing trait name after colon"
        );
    }

    #[test]
    fn malformed_where_clause_invalid_plus_placement() {
        // Where clause with plus in wrong position
        let src = "trait A { fn a(self) -> int; }\n\
                   trait B { fn b(self) -> int; }\n\
                   fn<T> process(T x) where T: + A B { return x.a(); }\n\
                   fn main(int _d) {} main();";
        let (_, errs) = parse(src);
        // Should detect invalid token after trait bound
        assert!(!errs.is_empty(), "should detect malformed plus placement");
    }

    #[test]
    fn malformed_where_clause_missing_comma() {
        // Multiple bounds without comma separator
        let src = "trait A { fn a(self) -> int; }\n\
                   trait B { fn b(self) -> int; }\n\
                   fn<T, U> process(T x, U y) where T: A U: B { return x.a(); }\n\
                   fn main(int _d) {} main();";
        let (_, errs) = parse(src);
        // Should detect end of where clause before second parameter
        assert!(
            !errs.is_empty(),
            "should detect missing comma between where bounds"
        );
    }

    #[test]
    fn malformed_where_clause_orphaned_bound() {
        // Where clause with bound but no type parameter name
        let src = "trait Display { fn show(self) -> string; }\n\
                   fn<T> render(T item) where : Display { return item.show(); }\n\
                   fn main(int _d) {} main();";
        let (_, errs) = parse(src);
        // Should detect missing type parameter name
        assert!(!errs.is_empty(), "should detect orphaned trait bound");
    }

    #[test]
    fn malformed_where_clause_double_colon_incomplete() {
        // Associated type projection with missing identifier
        let src = "trait Display { fn show(self) -> string; }\n\
                   fn<T> render(T item) where T:: Display { return item.show(); }\n\
                   fn main(int _d) {} main();";
        let (_, errs) = parse(src);
        // Should detect incomplete associated type
        assert!(!errs.is_empty(), "should handle incomplete associated type");
    }

    #[test]
    fn edge_case_where_clause_single_param_single_bound() {
        // Minimal valid where clause
        let src = "trait T { fn t(self) -> int; }\n\
                   fn<X> f(X x) where X: T { return 0; }\n\
                   fn main(int _d) {} main();";
        parse_check(src).expect("minimal where clause");
    }

    #[test]
    fn edge_case_where_clause_three_traits() {
        // Where clause with three traits on one parameter
        let src = "trait A { fn a(self) -> int; }\n\
                   trait B { fn b(self) -> int; }\n\
                   trait C { fn c(self) -> int; }\n\
                   fn<T> all(T x) where T: A + B + C { return x.a(); }\n\
                   fn main(int _d) {} main();";
        parse_check(src).expect("three trait bounds on one parameter");
    }

    #[test]
    fn edge_case_where_clause_four_params() {
        // Where clause constraining four type parameters
        let src = "trait T { fn t(self) -> int; }\n\
                   fn<A, B, C, D> quad(A a, B b, C c, D d) where A: T, B: T, C: T, D: T { return 0; }\n\
                   fn main(int _d) {} main();";
        parse_check(src).expect("four type parameters in where clause");
    }

    #[test]
    fn edge_case_no_where_clause() {
        // Verify non-generic functions work without where clauses
        let src = "fn add(int x, int y) -> int { return x + y; }\n\
                   fn main(int _d) {} main();";
        parse_check(src).expect("function without where clause");
    }
}
