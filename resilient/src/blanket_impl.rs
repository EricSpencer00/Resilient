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
//! ## Scope
//!
//! This pass is purely a validation gate. Runtime dispatch is not changed:
//! the interpreter continues to call methods via the `<Type>$<method>` mangling.
//! Blanket impls do NOT currently inject synthetic `ImplBlock` nodes — they
//! serve as a checked annotation that the programmer intends to cover all types
//! satisfying the bound, and the interpreter's existing structural-satisfaction
//! logic (checking that a type has all the right methods) handles dispatch.

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
}
