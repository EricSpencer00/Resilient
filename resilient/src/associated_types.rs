//! A-E3 (RES-3933): associated-type projection resolution — first
//! increment.
//!
//! `traits.rs` already parses `type Name;` trait declarations and
//! `type Name = ConcreteType;` impl bindings, and already enforces
//! binding *completeness* (every trait-declared associated type must
//! be bound by every impl of that trait — see `traits::check`'s
//! Pass 2, and its `impl_missing_associated_type_errors` test).
//! What was entirely unimplemented before this change: a
//! `Self::AssocName` projection written in Resilient source was not
//! even *parseable* as a type (`lib.rs::parse_type_annotation` had no
//! `::` handling — see the RES-3933 A-E3 change there), and nothing
//! resolved such a projection against a concrete binding or let it
//! participate in type checking.
//!
//! This increment ships two pieces:
//!
//! 1. **Real type-checking of `Self::AssocName`.** This is done in
//!    `typechecker.rs`, not here: `TypeChecker::current_self_assoc_types`
//!    is populated from an impl block's `associated_type_impls` on
//!    entry to `Node::ImplBlock`, and `parse_type_name_inner` resolves
//!    a `Self::AssocName` type annotation against it. That one change
//!    means a method declared `-> Self::Width` is checked by the
//!    *same* full return-type machinery every other function goes
//!    through (`Node::Function`'s `effective_rt` check,
//!    `Node::ReturnStatement`'s early-return check, and — as a
//!    side effect of `parse_type_name` being the single shared
//!    resolver — parameter types too) instead of a bespoke, weaker
//!    parallel checker. Deliberately *not* duplicated in this module.
//! 2. **Unknown / duplicate associated-type *binding* detection**
//!    (this module). `traits::check` validates every trait-declared
//!    associated type is bound; it does not validate the converse —
//!    that every binding an impl provides actually names an
//!    associated type the trait declares. This pass closes that gap,
//!    and rejects binding the same associated type twice within one
//!    impl block.
//!
//!    This module also gives a **dedicated, clearer diagnostic** when
//!    an impl method's return type is a projection naming an
//!    associated type the trait never declared at all (e.g.
//!    `-> Self::Bogus` on a trait with no `type Bogus;`). The deep
//!    typechecker's fallback for an unresolvable `Self::X` is the
//!    generic `Type::Struct("Self::X")`, which usually — but not
//!    always (a body that infers as `Type::Any`, e.g. via an
//!    unannotated call, is permissively compatible with anything) —
//!    surfaces as a generic "return type mismatch" error rather than
//!    naming the real problem: the projection itself is malformed.
//!
//! Deliberately out of scope for this increment — tracked in
//! [issue #4067](https://github.com/EricSpencer00/Resilient/issues/4067):
//!
//! - `T::AssocName` projections for a generic type parameter `T`
//!   (as opposed to `Self`) at a *use* site — RES-2695 already
//!   resolves these at `where T::Assoc: Bound` call sites; resolving
//!   them at arbitrary use sites needs monomorphization-time context.
//! - `Self::AssocName` in trait *default* method bodies (shared
//!   across impls — no single binding to resolve against until
//!   per-impl dispatch).
//! - Generic associated types and associated *constants*.
//!
//! Trait objects / `dyn Trait` (vtable dispatch) are a separate,
//! larger design question — descoped for v1 and tracked in
//! [issue #4068](https://github.com/EricSpencer00/Resilient/issues/4068).
//! See the `dyn` arm in `lib.rs::parse_type_annotation` for the
//! dedicated parse-time diagnostic that ships alongside this module.

use crate::Node;
use crate::span::Span;
use std::collections::{HashMap, HashSet};

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return Ok(()),
    };

    // Fast-reject: nothing to do unless some impl block binds at
    // least one associated type.
    let has_work = stmts.iter().any(|s| {
        matches!(
            &s.node,
            Node::ImplBlock { associated_type_impls, .. } if !associated_type_impls.is_empty()
        )
    });
    if !has_work {
        return Ok(());
    }

    // trait_name -> set of associated-type names it declares.
    let mut trait_assoc_names: HashMap<&str, HashSet<&str>> = HashMap::new();
    for stmt in stmts {
        if let Node::TraitDecl {
            name,
            associated_types,
            ..
        } = &stmt.node
        {
            let set = trait_assoc_names.entry(name.as_str()).or_default();
            for at in associated_types {
                set.insert(at.name.as_str());
            }
        }
    }

    for stmt in stmts {
        let Node::ImplBlock {
            trait_name: Some(t),
            struct_name,
            methods,
            associated_type_impls,
            span,
        } = &stmt.node
        else {
            continue;
        };

        let declared = match trait_assoc_names.get(t.as_str()) {
            Some(d) => d,
            // Unknown trait — `traits::check` already reports this.
            None => continue,
        };

        // Unknown / duplicate bindings.
        let mut seen: HashSet<&str> = HashSet::with_capacity(associated_type_impls.len());
        for (name, _type_expr) in associated_type_impls {
            if !declared.contains(name.as_str()) {
                return Err(format_err(
                    source_path,
                    *span,
                    &format!(
                        "impl `{}` for `{}` binds unknown associated type `{}` — trait `{}` does not declare it",
                        t, struct_name, name, t
                    ),
                ));
            }
            if !seen.insert(name.as_str()) {
                return Err(format_err(
                    source_path,
                    *span,
                    &format!(
                        "impl `{}` for `{}` binds associated type `{}` more than once",
                        t, struct_name, name
                    ),
                ));
            }
        }

        // A `Self::AssocName` return-type projection that names an
        // associated type the trait never declared. The bound-value
        // resolution and the actual return-type check both happen in
        // `typechecker.rs` (see module doc) — this is purely the
        // "is the projection even well-formed" check, which gives a
        // clearer diagnostic than that generic path's fallback.
        for method in methods {
            let Node::Function {
                name: method_name,
                return_type: Some(rt),
                span: fn_span,
                ..
            } = method
            else {
                continue;
            };
            let Some(assoc_name) = rt.strip_prefix("Self::") else {
                continue;
            };
            if !declared.contains(assoc_name) {
                return Err(format_err(
                    source_path,
                    *fn_span,
                    &format!(
                        "method `{}` in impl `{}` for `{}` returns `Self::{}`, but trait `{}` does not declare associated type `{}`",
                        method_name, t, struct_name, assoc_name, t, assoc_name
                    ),
                ));
            }
        }
    }

    Ok(())
}

fn format_err(source_path: &str, span: Span, msg: &str) -> String {
    if span.start.line == 0 {
        msg.to_string()
    } else {
        format!(
            "{}:{}:{}: {}",
            source_path, span.start.line, span.start.column, msg
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Lexer;
    use crate::Parser;

    fn parse_program(src: &str) -> Node {
        let lexer = Lexer::new(src);
        let mut parser = Parser::new(lexer);
        parser.parse_program()
    }

    /// Full-pipeline typecheck, exercising the `typechecker.rs`
    /// `current_self_assoc_types` resolution end to end (not just
    /// this module's own `check`).
    fn typecheck(src: &str) -> Result<(), String> {
        let (program, errors) = crate::parse(src);
        assert!(errors.is_empty(), "parse errors: {:?}", errors);
        crate::typechecker::TypeChecker::new()
            .check_program(&program)
            .map(|_| ())
    }

    #[test]
    fn self_assoc_projection_parses_as_return_type() {
        let prog = parse_program(
            "trait Sized2 { type Width; fn width(self) -> Self::Width; }\n\
             struct Fixed { int w }\n\
             impl Sized2 for Fixed {\n\
                 type Width = int;\n\
                 fn width(self) -> Self::Width { return self.w; }\n\
             }\n\
             fn main(int dummy) {} main();",
        );
        let stmts = match &prog {
            Node::Program(s) => s,
            _ => panic!("expected program"),
        };
        let found = stmts.iter().any(|s| {
            matches!(&s.node, Node::ImplBlock { methods, .. } if methods.iter().any(|m| matches!(
                m,
                Node::Function { return_type: Some(rt), .. } if rt == "Self::Width"
            )))
        });
        assert!(
            found,
            "expected an impl method with return_type `Self::Width`"
        );
    }

    // --- Real type-checking of `Self::AssocName` (typechecker.rs) ---

    #[test]
    fn self_assoc_return_matching_binding_typechecks() {
        let src = "trait Sized2 { type Width; fn width(self) -> Self::Width; }\n\
             struct Fixed { int w }\n\
             impl Sized2 for Fixed {\n\
                 type Width = int;\n\
                 fn width(self) -> Self::Width { return 4; }\n\
             }\n\
             fn main() {} main();";
        typecheck(src).unwrap_or_else(|e| panic!("unexpected error: {e}"));
    }

    #[test]
    fn self_assoc_return_mismatched_binding_is_rejected() {
        let src = "trait Sized2 { type Width; fn width(self) -> Self::Width; }\n\
             struct Fixed { int w }\n\
             impl Sized2 for Fixed {\n\
                 type Width = int;\n\
                 fn width(self) -> Self::Width { return \"nope\"; }\n\
             }\n\
             fn main(int dummy) {} main();";
        let e = typecheck(src).expect_err("expected return-type mismatch error");
        assert!(e.contains("return type mismatch"), "got: {e}");
        assert!(e.contains("string"), "got: {e}");
        assert!(e.contains("int"), "got: {e}");
    }

    #[test]
    fn self_assoc_return_struct_binding_matches_struct_literal() {
        let src = "trait Container { type Item; fn make(self) -> Self::Item; }\n\
             struct Payload { int x }\n\
             struct Box2 { int dummy }\n\
             impl Container for Box2 {\n\
                 type Item = Payload;\n\
                 fn make(self) -> Self::Item { return new Payload { x: 1 }; }\n\
             }\n\
             fn main() {} main();";
        typecheck(src).unwrap_or_else(|e| panic!("unexpected error: {e}"));
    }

    #[test]
    fn self_assoc_return_struct_binding_mismatch_is_rejected() {
        let src = "trait Container { type Item; fn make(self) -> Self::Item; }\n\
             struct Payload { int x }\n\
             struct Other { int y }\n\
             struct Box2 { int dummy }\n\
             impl Container for Box2 {\n\
                 type Item = Payload;\n\
                 fn make(self) -> Self::Item { return new Other { y: 1 }; }\n\
             }\n\
             fn main(int dummy) {} main();";
        let e = typecheck(src).expect_err("expected struct-literal mismatch error");
        assert!(e.contains("Payload"), "got: {e}");
    }

    #[test]
    fn self_assoc_field_access_return_typechecks() {
        // `self.w` is a FieldAccess resolved via full inference by the
        // real typechecker (not a shallow literal-shape heuristic).
        let src = "trait Sized2 { type Width; fn width(self) -> Self::Width; }\n\
             struct Fixed { int w }\n\
             impl Sized2 for Fixed {\n\
                 type Width = int;\n\
                 fn width(self) -> Self::Width { return self.w; }\n\
             }\n\
             fn main() {} main();";
        typecheck(src).unwrap_or_else(|e| panic!("unexpected error: {e}"));
    }

    // --- Unknown / duplicate binding detection (this module) ---

    #[test]
    fn unknown_assoc_projection_in_return_type_errors() {
        let src = "trait Sized2 { type Width; fn width(self) -> Self::Width; }\n\
             struct Fixed { int w }\n\
             impl Sized2 for Fixed {\n\
                 type Width = int;\n\
                 fn bogus(self) -> Self::NotDeclared { return 1; }\n\
                 fn width(self) -> Self::Width { return self.w; }\n\
             }\n\
             fn main(int dummy) {} main();";
        let prog = parse_program(src);
        let e = check(&prog, "test.rz").expect_err("expected unknown-projection error");
        assert!(e.contains("NotDeclared"), "got: {e}");
        assert!(e.contains("does not declare"), "got: {e}");
    }

    #[test]
    fn unknown_associated_type_binding_errors() {
        let src = "trait Sized2 { type Width; }\n\
             struct Fixed { int w }\n\
             impl Sized2 for Fixed {\n\
                 type Width = int;\n\
                 type Bogus = int;\n\
             }\n\
             fn main(int dummy) {} main();";
        let prog = parse_program(src);
        let e = check(&prog, "test.rz").expect_err("expected unknown-binding error");
        assert!(e.contains("Bogus"), "got: {e}");
        assert!(e.contains("does not declare"), "got: {e}");
    }

    #[test]
    fn duplicate_associated_type_binding_errors() {
        let src = "trait Sized2 { type Width; }\n\
             struct Fixed { int w }\n\
             impl Sized2 for Fixed {\n\
                 type Width = int;\n\
                 type Width = float;\n\
             }\n\
             fn main(int dummy) {} main();";
        let prog = parse_program(src);
        let e = check(&prog, "test.rz").expect_err("expected duplicate-binding error");
        assert!(e.contains("more than once"), "got: {e}");
    }

    #[test]
    fn plain_impl_without_associated_types_is_unaffected() {
        let src = "trait Printable { fn to_string(self) -> string; }\n\
             struct Point { int x }\n\
             impl Printable for Point { fn to_string(self) -> string { return \"p\"; } }\n\
             fn main(int dummy) {} main();";
        let prog = parse_program(src);
        check(&prog, "test.rz").unwrap_or_else(|e| panic!("unexpected error: {e}"));
    }

    #[test]
    fn empty_program_passes() {
        let prog = Node::Program(Vec::new());
        check(&prog, "test.rz").expect("empty program should pass");
    }
}
