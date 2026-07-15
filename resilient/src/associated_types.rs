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
//! Second increment (RES-3933 A-E3 follow-up, #4067):
//!
//! 3. **`T::AssocName` projections in generic fn signatures.** A
//!    projection whose base is a generic type parameter is *opaque* —
//!    its concrete identity only exists at monomorphization time — so
//!    the call-site machinery substitutes it to `Type::Any`
//!    (`type_relations::is_type_param_projection`), which stops
//!    parameter-position projections falsely rejecting every call
//!    (`Type::Struct("T::Item")` could never structurally match a
//!    real argument). What *can* be checked statically, and is (this
//!    module, `check_generic_fn_projections`): well-formedness — a
//!    projection `T::Assoc` where every trait reachable from `T`'s
//!    declared bounds (including `extends` super-traits) is known and
//!    none declares `Assoc` is a provable violation and is rejected.
//!    Any unknown trait in the chain, an unbounded `T` (a `where`
//!    clause may bound it), or a projection nested inside a larger
//!    type expression is permissively skipped — zero false positives
//!    by construction.
//! 4. **`let`-binding annotations inside impl methods** — `let x:
//!    Self::Width = ...;` resolves against the impl's concrete
//!    binding for free, because `parse_type_name` is the single
//!    shared resolver `Node::LetBinding` also goes through. Now
//!    explicitly tested (see `self_assoc_let_binding_*` tests and
//!    `examples/trait_associated_type_let_binding.rz`) rather than an
//!    undocumented side effect.
//!
//! Still out of scope — tracked in
//! [issue #4067](https://github.com/EricSpencer00/Resilient/issues/4067):
//!
//! - Resolving `T::AssocName` to the *concrete* bound type at a
//!   monomorphized call site (today it is opaque `Any` — permissive,
//!   never wrong, just not maximally precise).
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
    // least one associated type, or some generic fn signature carries
    // a `T::Assoc` projection (#4067 second increment).
    let has_impl_work = stmts.iter().any(|s| {
        matches!(
            &s.node,
            Node::ImplBlock { associated_type_impls, .. } if !associated_type_impls.is_empty()
        )
    });
    let has_generic_fn_work = stmts.iter().any(|s| {
        matches!(
            &s.node,
            Node::Function { type_params, .. } if !type_params.is_empty()
        )
    });
    if !has_impl_work && !has_generic_fn_work {
        return Ok(());
    }

    // trait_name -> set of associated-type names it declares.
    let mut trait_assoc_names: HashMap<&str, HashSet<&str>> = HashMap::new();
    // trait_name -> super-traits it `extends` (for transitive
    // associated-type lookup through inheritance).
    let mut trait_supers: HashMap<&str, &[String]> = HashMap::new();
    for stmt in stmts {
        if let Node::TraitDecl {
            name,
            associated_types,
            supers,
            ..
        } = &stmt.node
        {
            let set = trait_assoc_names.entry(name.as_str()).or_default();
            for at in associated_types {
                set.insert(at.name.as_str());
            }
            trait_supers.insert(name.as_str(), supers.as_slice());
        }
    }

    if has_generic_fn_work {
        for stmt in stmts {
            if let Node::Function { .. } = &stmt.node {
                check_generic_fn_projections(
                    &stmt.node,
                    &trait_assoc_names,
                    &trait_supers,
                    source_path,
                )?;
            }
        }
    }

    if !has_impl_work {
        return Ok(());
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

/// #4067: well-formedness of `T::Assoc` projections in a generic fn
/// signature (return and parameter positions). Rejects ONLY provable
/// violations: the base must be one of the fn's type parameters, that
/// parameter must have at least one declared bound, every trait
/// reachable from those bounds (through `extends` super-traits) must
/// be known, and none of them may declare the projected name. An
/// unbounded parameter, any unknown trait in the chain, or a
/// projection nested inside a larger type expression is skipped —
/// permissive by construction, so every previously-compiling program
/// keeps compiling unless it names a provably-nonexistent associated
/// type.
fn check_generic_fn_projections(
    func: &Node,
    trait_assoc_names: &HashMap<&str, HashSet<&str>>,
    trait_supers: &HashMap<&str, &[String]>,
    source_path: &str,
) -> Result<(), String> {
    let Node::Function {
        name: fn_name,
        parameters,
        return_type,
        type_params,
        type_param_bounds,
        span,
        ..
    } = func
    else {
        return Ok(());
    };
    if type_params.is_empty() {
        return Ok(());
    }

    let annotations = return_type
        .iter()
        .map(|rt| (rt.as_str(), "return type"))
        .chain(
            parameters
                .iter()
                .map(|(ty, _name)| (ty.as_str(), "parameter type")),
        );

    for (raw, position) in annotations {
        let ty = raw.strip_prefix("linear ").unwrap_or(raw);
        let Some((base, assoc)) = ty.split_once("::") else {
            continue;
        };
        // Nested projections (`Array<T::Item>`) don't split at the
        // top level like this; a chained `A::B::C` is already
        // unparseable. Only exact `Base::Assoc` reaches here.
        let Some(idx) = type_params.iter().position(|p| p == base) else {
            continue; // `Self::X` (impl methods) or a non-generic base.
        };
        let bounds = match type_param_bounds.get(idx) {
            Some(b) if !b.is_empty() => b,
            // Unbounded parameter — a `where` clause may bound it;
            // stay permissive.
            _ => continue,
        };
        let Some(declared) = transitive_assoc_names(bounds, trait_assoc_names, trait_supers) else {
            continue; // Unknown trait somewhere in the chain.
        };
        if !declared.contains(assoc) {
            return Err(format_err(
                source_path,
                *span,
                &format!(
                    "fn `{}` {} projects `{}::{}`, but no trait bound of `{}` ({}) declares associated type `{}`",
                    fn_name,
                    position,
                    base,
                    assoc,
                    base,
                    bounds.join(" + "),
                    assoc
                ),
            ));
        }
    }
    Ok(())
}

/// Union of associated-type names declared by `bounds` and,
/// transitively, their `extends` super-traits. Returns `None` when
/// any trait in the chain is unknown — the caller must then stay
/// permissive rather than risk a false positive.
fn transitive_assoc_names<'a>(
    bounds: &[String],
    trait_assoc_names: &HashMap<&str, HashSet<&'a str>>,
    trait_supers: &HashMap<&str, &'a [String]>,
) -> Option<HashSet<&'a str>> {
    let mut names: HashSet<&str> = HashSet::new();
    let mut visited: HashSet<&str> = HashSet::new();
    let mut work: Vec<&str> = bounds.iter().map(String::as_str).collect();
    while let Some(t) = work.pop() {
        if !visited.insert(t) {
            continue;
        }
        // `effect` is a bound-position keyword (`E: effect`), not a
        // trait; it declares no associated types and is always known.
        if t == "effect" {
            continue;
        }
        let declared = trait_assoc_names.get(t)?;
        names.extend(declared.iter().copied());
        if let Some(supers) = trait_supers.get(t) {
            work.extend(supers.iter().map(String::as_str));
        }
    }
    Some(names)
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

    // --- #4067: `T::Assoc` projections in generic fn signatures ---

    const CONTAINER_PRELUDE: &str = "trait Container { type Item; fn first(self) -> Self::Item; }\n\
         struct IntBox { int v }\n\
         impl Container for IntBox {\n\
             type Item = int;\n\
             fn first(self) -> Self::Item { return self.v; }\n\
         }\n";

    #[test]
    fn generic_return_projection_of_declared_assoc_accepted() {
        let src = format!(
            "{CONTAINER_PRELUDE}\
             fn get_first<T: Container>(T c) -> T::Item {{ return c.first(); }}\n\
             fn main() {{}} main();"
        );
        typecheck(&src).unwrap_or_else(|e| panic!("unexpected error: {e}"));
    }

    #[test]
    fn generic_param_position_projection_call_site_accepted() {
        // The regression this increment fixes: before #4067 the call
        // was rejected with "expected T::Item, got int" because the
        // unresolved projection survived call-site substitution.
        let src = format!(
            "{CONTAINER_PRELUDE}\
             fn use_item<T: Container>(T c, T::Item seed) -> int {{ return 1; }}\n\
             fn main() {{\n\
                 let b = new IntBox {{ v: 42 }};\n\
                 println(use_item(b, 5));\n\
             }} main();"
        );
        typecheck(&src).unwrap_or_else(|e| panic!("unexpected error: {e}"));
    }

    #[test]
    fn generic_projection_of_undeclared_assoc_rejected() {
        let src = format!(
            "{CONTAINER_PRELUDE}\
             fn get_bogus<T: Container>(T c) -> T::Bogus {{ return c.first(); }}\n\
             fn main() {{}} main();"
        );
        let e = typecheck(&src).expect_err("expected undeclared-projection error");
        assert!(e.contains("T::Bogus"), "got: {e}");
        assert!(e.contains("Container"), "got: {e}");
    }

    #[test]
    fn generic_param_position_projection_of_undeclared_assoc_rejected() {
        let src = format!(
            "{CONTAINER_PRELUDE}\
             fn use_bogus<T: Container>(T c, T::Bogus seed) -> int {{ return 1; }}\n\
             fn main() {{}} main();"
        );
        let e = typecheck(&src).expect_err("expected undeclared-projection error");
        assert!(e.contains("T::Bogus"), "got: {e}");
        assert!(e.contains("parameter type"), "got: {e}");
    }

    #[test]
    fn generic_projection_through_supertrait_accepted() {
        // `Iter extends Container` — projecting `T::Item` through a
        // bound on the *sub*-trait must find the super-trait's
        // declaration.
        let src = "trait Container { type Item; }\n\
             trait Iter extends Container { fn advance(self) -> int; }\n\
             fn peek<T: Iter>(T it) -> T::Item { return 1; }\n\
             fn main() {} main();";
        // The body `return 1` vs opaque `T::Item` is a separate
        // (pre-existing) mismatch question; run only this module's
        // well-formedness pass, which must accept the projection.
        let prog = parse_program(src);
        check(&prog, "test.rz").unwrap_or_else(|e| panic!("unexpected error: {e}"));
    }

    #[test]
    fn generic_projection_on_unbounded_param_is_permissively_skipped() {
        // No bounds on `T` — a `where` clause may bound it, so the
        // pass must not reject (zero false positives).
        let src = "fn f<T>(T x) -> T::Item { return x; }\n\
             fn main() {} main();";
        let prog = parse_program(src);
        check(&prog, "test.rz").unwrap_or_else(|e| panic!("unexpected error: {e}"));
    }

    #[test]
    fn generic_projection_with_unknown_bound_trait_is_permissively_skipped() {
        // `Mystery` is not declared in this program — `traits::check`
        // owns that diagnostic; this pass must stay permissive.
        let src = "fn f<T: Mystery>(T x) -> T::Item { return x; }\n\
             fn main() {} main();";
        let prog = parse_program(src);
        check(&prog, "test.rz").unwrap_or_else(|e| panic!("unexpected error: {e}"));
    }

    // --- #4067 item 4: `Self::Assoc` in let-binding annotations ---

    #[test]
    fn self_assoc_let_binding_matching_binding_typechecks() {
        let src = "trait Sized2 { type Width; fn width(self) -> Self::Width; }\n\
             struct Fixed { int w }\n\
             impl Sized2 for Fixed {\n\
                 type Width = int;\n\
                 fn width(self) -> Self::Width {\n\
                     let x: Self::Width = self.w;\n\
                     return x;\n\
                 }\n\
             }\n\
             fn main() {} main();";
        typecheck(src).unwrap_or_else(|e| panic!("unexpected error: {e}"));
    }

    #[test]
    fn self_assoc_let_binding_mismatched_binding_is_rejected() {
        let src = "trait Sized2 { type Width; fn width(self) -> Self::Width; }\n\
             struct Fixed { int w }\n\
             impl Sized2 for Fixed {\n\
                 type Width = int;\n\
                 fn width(self) -> Self::Width {\n\
                     let x: Self::Width = \"nope\";\n\
                     return 1;\n\
                 }\n\
             }\n\
             fn main() {} main();";
        let e = typecheck(src).expect_err("expected let-binding mismatch error");
        assert!(e.contains("int"), "got: {e}");
        assert!(e.contains("string"), "got: {e}");
    }
}
