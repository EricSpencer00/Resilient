//! RES-2552: blanket trait implementations.
//!
//! Validates `impl<T: Bound> Trait for T { ... }` declarations parsed into
//! `Node::BlanketImpl`. The pass runs after `traits::check` so it can assume
//! that all `TraitDecl` nodes are well-formed.
//!
//! ## Rules
//!
//! 1. **Trait must be declared** — `trait_name` must refer to a `TraitDecl`
//!    in scope.
//! 2. **Every bound must be a declared trait** — each entry in `bounds` must
//!    name a `TraitDecl`.
//! 3. **Full method coverage** — the blanket impl must implement every method
//!    declared by the trait (matching by name; arity is checked).
//! 4. **Conflict detection** — a blanket impl may coexist with a specific
//!    `impl Trait for ConcreteType` (the specific impl wins at call sites);
//!    only *duplicate* blanket impls (same `trait_name + bounds`) are an error.
//!
//! ## Monomorphization (RES-2685)
//!
//! `lower_program` synthesizes concrete `ImplBlock` nodes from each `BlanketImpl`.
//! For every struct that satisfies all bounds but lacks a specific impl of the
//! target trait, it clones the blanket methods, rewrites `T$method → Concrete$method`
//! in the function name and self-parameter type, and injects the resulting
//! `ImplBlock` immediately after the originating `BlanketImpl` in the AST.

#![allow(unused_imports)]

use crate::span::Span;
use crate::traits::TraitMethodSig;
use crate::{Node, Token};
use std::collections::{HashMap, HashSet};

/// Top-level entry point, called from the `<EXTENSION_PASSES>` block in
/// `typechecker.rs` when `markers.has_blanket_impl` is true.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let stmts = match program {
        Node::Program(s) => s,
        _ => return Ok(()),
    };

    // Pass 1: collect all declared traits.
    let mut traits: HashMap<String, Vec<TraitMethodSig>> = HashMap::new();
    for s in stmts {
        if let Node::TraitDecl { name, methods, .. } = &s.node {
            traits.insert(name.clone(), methods.clone());
        }
    }

    // Pass 2: collect specific impls (trait_name, struct_name) so we can
    // report when a blanket impl overlaps (informational — not an error).
    let mut specific_impls: HashSet<(String, String)> = HashSet::new();
    for s in stmts {
        if let Node::ImplBlock {
            trait_name: Some(t),
            struct_name,
            ..
        } = &s.node
        {
            specific_impls.insert((t.clone(), struct_name.clone()));
        }
    }

    // Pass 3: validate every BlanketImpl.
    // Track seen (trait_name, type_param, bounds) triples for duplicate detection.
    let mut seen_blankets: HashSet<(String, String, Vec<String>)> = HashSet::new();

    for s in stmts {
        if let Node::BlanketImpl {
            type_param,
            bounds,
            trait_name,
            methods,
            span,
        } = &s.node
        {
            // Rule 1: trait must be declared.
            let trait_sigs = match traits.get(trait_name) {
                Some(sigs) => sigs,
                None => {
                    return Err(format_err(
                        source_path,
                        *span,
                        &format!("blanket impl references unknown trait `{}`", trait_name),
                    ));
                }
            };

            // Rule 2: every bound must name a declared trait.
            for bound in bounds {
                if !traits.contains_key(bound) {
                    return Err(format_err(
                        source_path,
                        *span,
                        &format!(
                            "blanket impl `impl<{}: ...> {} for {}` uses unknown trait bound `{}`",
                            type_param, trait_name, type_param, bound
                        ),
                    ));
                }
            }

            // Rule 3: method coverage. Build a set of methods provided.
            // Methods are mangled `<type_param>$<plain_name>` by the parser.
            let prefix = format!("{}$", type_param);
            let mut provided: HashMap<String, usize> = HashMap::new();
            for m in methods {
                if let Node::Function {
                    name, parameters, ..
                } = m
                {
                    let plain = name.strip_prefix(&prefix).unwrap_or(name.as_str());
                    provided.insert(plain.to_string(), parameters.len());
                }
            }

            for sig in trait_sigs {
                match provided.get(&sig.name) {
                    None => {
                        return Err(format_err(
                            source_path,
                            *span,
                            &format!(
                                "blanket impl `impl<{}> {} for {}` is missing method `{}` declared by trait `{}`",
                                type_param, trait_name, type_param, sig.name, trait_name
                            ),
                        ));
                    }
                    Some(&arity) if arity != sig.param_arity => {
                        return Err(format_err(
                            source_path,
                            *span,
                            &format!(
                                "blanket impl `impl<{}> {} for {}` method `{}` has {} parameter(s); trait `{}` declares {}",
                                type_param,
                                trait_name,
                                type_param,
                                sig.name,
                                arity,
                                trait_name,
                                sig.param_arity
                            ),
                        ));
                    }
                    Some(_) => {}
                }
            }

            // Rule 4: no duplicate blanket impls (same trait + same bounds).
            let mut sorted_bounds = bounds.clone();
            sorted_bounds.sort();
            let key = (trait_name.clone(), type_param.clone(), sorted_bounds);
            if !seen_blankets.insert(key) {
                return Err(format_err(
                    source_path,
                    *span,
                    &format!(
                        "duplicate blanket impl `impl<{}> {} for {}`",
                        type_param, trait_name, type_param
                    ),
                ));
            }
        }
    }

    Ok(())
}

/// RES-2685: Monomorphize blanket impls into concrete `ImplBlock` nodes.
///
/// For each `BlanketImpl { type_param, bounds, trait_name, methods }`, finds
/// every struct that implements all bounds but does NOT already have a specific
/// `impl trait_name for Struct`, and synthesizes a concrete `ImplBlock` by
/// substituting `type_param → StructName` in method names and self-param types.
///
/// Injected nodes appear immediately after the originating `BlanketImpl` so
/// the interpreter registers them before any downstream function calls.
pub fn lower_program(program: &mut Node) {
    let stmts = match program {
        Node::Program(s) => s,
        _ => return,
    };

    // Fast-path: nothing to do if there are no blanket impls.
    if !stmts
        .iter()
        .any(|s| matches!(s.node, Node::BlanketImpl { .. }))
    {
        return;
    }

    // Build struct_trait_impls: for each struct name, which traits it explicitly
    // implements via concrete `ImplBlock` nodes.
    let mut struct_trait_impls: HashMap<String, HashSet<String>> = HashMap::new();
    for s in stmts.iter() {
        if let Node::ImplBlock {
            trait_name: Some(t),
            struct_name,
            ..
        } = &s.node
        {
            struct_trait_impls
                .entry(struct_name.clone())
                .or_default()
                .insert(t.clone());
        }
    }

    // Collect all declared struct names.
    let mut struct_names: Vec<String> = Vec::new();
    for s in stmts.iter() {
        if let Node::StructDecl { name, .. } = &s.node {
            struct_names.push(name.clone());
        }
    }

    // Walk the statement list, inserting synthetic ImplBlocks after each
    // BlanketImpl. Use an index-based loop because we mutate `stmts`.
    let mut i = 0;
    while i < stmts.len() {
        let (type_param, bounds, trait_name, methods, blanket_span) = if let Node::BlanketImpl {
            type_param,
            bounds,
            trait_name,
            methods,
            span,
        } = &stmts[i].node
        {
            (
                type_param.clone(),
                bounds.clone(),
                trait_name.clone(),
                methods.clone(),
                *span,
            )
        } else {
            i += 1;
            continue;
        };

        let mut to_insert: Vec<crate::span::Spanned<Node>> = Vec::new();

        for concrete in &struct_names {
            // Skip if there's already a specific impl for this (struct, trait) pair.
            if struct_trait_impls
                .get(concrete)
                .map(|ts| ts.contains(&trait_name))
                .unwrap_or(false)
            {
                continue;
            }

            // Check the struct implements every required bound.
            let satisfies = bounds.iter().all(|bound| {
                struct_trait_impls
                    .get(concrete)
                    .map(|ts| ts.contains(bound))
                    .unwrap_or(false)
            });
            if !satisfies {
                continue;
            }

            // Synthesize concrete method nodes: T$method → Concrete$method,
            // self-parameter type T → Concrete.
            let prefix = format!("{}$", type_param);
            let synthesized: Vec<Node> = methods
                .iter()
                .map(|method| {
                    if let Node::Function {
                        name,
                        parameters,
                        defaults,
                        body,
                        requires,
                        ensures,
                        return_type,
                        span,
                        pure,
                        effects,
                        type_params,
                        type_param_bounds,
                        fails,
                        recovers_to,
                        is_pub,
                    } = method.clone()
                    {
                        let new_name = name
                            .strip_prefix(&prefix)
                            .map(|rest| format!("{}${}", concrete, rest))
                            .unwrap_or(name);
                        let new_params: Vec<(String, String)> = parameters
                            .into_iter()
                            .map(|(ty, nm)| {
                                (
                                    if ty == type_param {
                                        concrete.clone()
                                    } else {
                                        ty
                                    },
                                    nm,
                                )
                            })
                            .collect();
                        Node::Function {
                            name: new_name,
                            parameters: new_params,
                            defaults,
                            body,
                            requires,
                            ensures,
                            return_type,
                            span,
                            pure,
                            effects,
                            type_params,
                            type_param_bounds,
                            fails,
                            recovers_to,
                            is_pub,
                        }
                    } else {
                        method.clone()
                    }
                })
                .collect();

            let synthetic_impl = Node::ImplBlock {
                trait_name: Some(trait_name.clone()),
                struct_name: concrete.clone(),
                methods: synthesized,
                associated_type_impls: Vec::new(),
                span: blanket_span,
            };
            to_insert.push(crate::span::Spanned::new(synthetic_impl, blanket_span));
            // Record the new impl so later blankets see it.
            struct_trait_impls
                .entry(concrete.clone())
                .or_default()
                .insert(trait_name.clone());
        }

        let insert_count = to_insert.len();
        stmts.splice((i + 1)..(i + 1), to_insert);
        i += 1 + insert_count;
    }
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
    use crate::Lexer;
    use crate::Parser;

    fn parse_and_check(src: &str) -> Result<(), String> {
        let lexer = Lexer::new(src);
        let mut parser = Parser::new(lexer);
        let prog = parser.parse_program();
        super::check(&prog, "<test>")
    }

    #[test]
    fn blanket_impl_valid() {
        let src = r#"
trait Display { fn show(self) -> string; }
trait Loud { fn shout(self) -> string; }
impl<T: Display> Loud for T {
    fn shout(self) -> string { return "loud"; }
}
fn main() {}
main();
"#;
        assert!(
            parse_and_check(src).is_ok(),
            "expected valid blanket impl to pass"
        );
    }

    #[test]
    fn blanket_impl_unknown_trait() {
        let src = r#"
trait Loud { fn shout(self) -> string; }
impl<T: Display> Loud for T {
    fn shout(self) -> string { return "loud"; }
}
fn main() {}
main();
"#;
        let err = parse_and_check(src).unwrap_err();
        assert!(
            err.contains("unknown trait bound") || err.contains("unknown trait"),
            "got: {}",
            err
        );
    }

    #[test]
    fn blanket_impl_missing_method() {
        let src = r#"
trait Display { fn show(self) -> string; }
trait Loud { fn shout(self) -> string; fn whisper(self) -> string; }
impl<T: Display> Loud for T {
    fn shout(self) -> string { return "loud"; }
}
fn main() {}
main();
"#;
        let err = parse_and_check(src).unwrap_err();
        assert!(err.contains("missing method"), "got: {}", err);
    }

    #[test]
    fn blanket_impl_unknown_implemented_trait() {
        let src = r#"
trait Display { fn show(self) -> string; }
impl<T: Display> UnknownTrait for T {
    fn foo(self) -> string { return "x"; }
}
fn main() {}
main();
"#;
        let err = parse_and_check(src).unwrap_err();
        assert!(err.contains("unknown trait"), "got: {}", err);
    }

    #[test]
    fn duplicate_blanket_impl_errors() {
        let src = r#"
trait Display { fn show(self) -> string; }
trait Loud { fn shout(self) -> string; }
impl<T: Display> Loud for T {
    fn shout(self) -> string { return "loud"; }
}
impl<T: Display> Loud for T {
    fn shout(self) -> string { return "LOUD"; }
}
fn main() {}
main();
"#;
        let err = parse_and_check(src).unwrap_err();
        assert!(err.contains("duplicate blanket impl"), "got: {}", err);
    }

    #[test]
    fn blanket_impl_coexists_with_specific_impl() {
        // Specific impl for `int` and a blanket impl for all `Display` types
        // should both be valid (specific wins at call sites).
        let src = r#"
trait Display { fn show(self) -> string; }
trait Loud { fn shout(self) -> string; }
impl Loud for int { fn shout(self) -> string { return "42!"; } }
impl<T: Display> Loud for T {
    fn shout(self) -> string { return "loud"; }
}
fn main() {}
main();
"#;
        assert!(
            parse_and_check(src).is_ok(),
            "blanket impl coexisting with specific impl should be valid"
        );
    }

    /// RES-2685: lower_program synthesizes a concrete ImplBlock so the method
    /// can be called without a redundant specific impl.
    #[test]
    fn lower_program_synthesizes_concrete_impl() {
        let r = crate::run_program(
            r#"
struct Score { int value, }
trait Display { fn show(self) -> string; }
trait Loud    { fn shout(self) -> string; }
impl Display for Score {
    fn show(self) -> string { return to_string(self.value); }
}
impl<T: Display> Loud for T {
    fn shout(self) -> string { return self.show() + "!"; }
}
fn main() {
    let s = new Score { value: 7 };
    println(s.show());
    println(s.shout());
}
main();
"#,
        );
        assert!(r.ok, "eval errors: {:?}", r.errors);
        assert!(
            r.stdout.contains("7"),
            "expected '7' in output, got: {}",
            r.stdout
        );
        assert!(
            r.stdout.contains("7!"),
            "expected '7!' in output, got: {}",
            r.stdout
        );
    }

    /// RES-2685: specific impl wins over blanket — no duplicate-method error.
    #[test]
    fn lower_program_specific_impl_wins() {
        let r = crate::run_program(
            r#"
struct Score { int value, }
trait Display { fn show(self) -> string; }
trait Loud    { fn shout(self) -> string; }
impl Display for Score {
    fn show(self) -> string { return to_string(self.value); }
}
impl Loud for Score {
    fn shout(self) -> string { return "SPECIFIC"; }
}
impl<T: Display> Loud for T {
    fn shout(self) -> string { return "BLANKET"; }
}
fn main() {
    let s = new Score { value: 1 };
    println(s.shout());
}
main();
"#,
        );
        assert!(r.ok, "eval errors: {:?}", r.errors);
        assert!(
            r.stdout.contains("SPECIFIC"),
            "specific impl should win; got: {}",
            r.stdout
        );
    }
}
