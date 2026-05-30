//! RES-2572: Trait inheritance — `trait A extends B + C`.
//!
//! This pass validates:
//! 1. Every super-trait name in `extends` refers to a declared trait.
//! 2. For every `impl Trait for Type`, if Trait has supers, the impl
//!    set for Type also contains those supers (directly or transitively).
//! 3. Diamond inheritance is handled by deduplication: a super appearing
//!    through multiple paths is only required once.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::Node;

/// Build a map from trait name → its direct super-trait names.
fn collect_supers(program: &Node) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    if let Node::Program(statements) = program {
        for st in statements {
            if let Node::TraitDecl { name, supers, .. } = &st.node {
                map.entry(name.clone()).or_default().extend(supers.clone());
            }
        }
    }
    map
}

/// Compute the full set of required super-traits for `trait_name`
/// (transitive closure, deduped, not including `trait_name` itself).
fn all_supers(trait_name: &str, supers_map: &HashMap<String, Vec<String>>) -> HashSet<String> {
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    if let Some(direct) = supers_map.get(trait_name) {
        for s in direct {
            queue.push_back(s.clone());
        }
    }
    while let Some(cur) = queue.pop_front() {
        if visited.insert(cur.clone())
            && let Some(next) = supers_map.get(&cur)
        {
            for s in next {
                queue.push_back(s.clone());
            }
        }
    }
    visited
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let Node::Program(statements) = program else {
        return Ok(());
    };

    // Step 1: collect trait→supers and declared trait names.
    let supers_map = collect_supers(program);
    let declared_traits: HashSet<&str> = supers_map.keys().map(String::as_str).collect();

    // Step 2: validate that every super-trait name is declared.
    for st in statements {
        if let Node::TraitDecl { name, supers, .. } = &st.node {
            for sup in supers {
                if !declared_traits.contains(sup.as_str()) {
                    return Err(format!("trait `{}` extends unknown trait `{}`", name, sup));
                }
            }
        }
    }

    // Step 3: collect which traits each type implements.
    // type → set of trait names it implements.
    let mut type_impls: HashMap<String, HashSet<String>> = HashMap::new();
    for st in statements {
        if let Node::ImplBlock {
            struct_name,
            trait_name: Some(tname),
            ..
        } = &st.node
        {
            type_impls
                .entry(struct_name.clone())
                .or_default()
                .insert(tname.clone());
        }
    }

    // Step 4: for every impl, verify supers are satisfied.
    for st in statements {
        if let Node::ImplBlock {
            struct_name,
            trait_name: Some(tname),
            ..
        } = &st.node
        {
            let required = all_supers(tname.as_str(), &supers_map);
            let implemented = type_impls
                .get(struct_name.as_str())
                .cloned()
                .unwrap_or_default();
            for sup in &required {
                if !implemented.contains(sup) {
                    return Err(format!(
                        "type `{}` implements `{}` but is missing required super-trait `{}`",
                        struct_name, tname, sup
                    ));
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::check;
    use crate::{Lexer, Parser};

    fn parse_program(src: &str) -> crate::Node {
        let lexer = Lexer::new(src);
        let mut parser = Parser::new(lexer);
        parser.parse_program()
    }

    #[test]
    fn single_super_satisfied() {
        let prog = parse_program(
            "trait Eq { fn eq(self) -> bool; }
             trait Ord extends Eq { fn cmp(self) -> int; }
             struct Point { int x, }
             impl Eq for Point { fn eq(self) -> bool { return true; } }
             impl Ord for Point { fn cmp(self) -> int { return 0; } }",
        );
        check(&prog, "test.rz").expect("expected ok");
    }

    #[test]
    fn single_super_missing() {
        let prog = parse_program(
            "trait Eq { fn eq(self) -> bool; }
             trait Ord extends Eq { fn cmp(self) -> int; }
             struct Point { int x, }
             impl Ord for Point { fn cmp(self) -> int { return 0; } }",
        );
        let err = check(&prog, "test.rz").expect_err("expected error");
        assert!(
            err.contains("missing required super-trait"),
            "wrong error: {}",
            err
        );
    }

    #[test]
    fn multiple_supers_satisfied() {
        let prog = parse_program(
            "trait A { fn a(self) -> int; }
             trait B { fn b(self) -> int; }
             trait C extends A + B { fn c(self) -> int; }
             struct S { int x, }
             impl A for S { fn a(self) -> int { return 1; } }
             impl B for S { fn b(self) -> int { return 2; } }
             impl C for S { fn c(self) -> int { return 3; } }",
        );
        check(&prog, "test.rz").expect("expected ok");
    }

    #[test]
    fn multiple_supers_one_missing() {
        let prog = parse_program(
            "trait A { fn a(self) -> int; }
             trait B { fn b(self) -> int; }
             trait C extends A + B { fn c(self) -> int; }
             struct S { int x, }
             impl A for S { fn a(self) -> int { return 1; } }
             impl C for S { fn c(self) -> int { return 3; } }",
        );
        let err = check(&prog, "test.rz").expect_err("expected error");
        assert!(
            err.contains("missing required super-trait"),
            "wrong error: {}",
            err
        );
    }

    #[test]
    fn transitive_supers_satisfied() {
        let prog = parse_program(
            "trait A { fn a(self) -> int; }
             trait B extends A { fn b(self) -> int; }
             trait C extends B { fn c(self) -> int; }
             struct S { int x, }
             impl A for S { fn a(self) -> int { return 1; } }
             impl B for S { fn b(self) -> int { return 2; } }
             impl C for S { fn c(self) -> int { return 3; } }",
        );
        check(&prog, "test.rz").expect("expected ok");
    }

    #[test]
    fn transitive_super_missing() {
        let prog = parse_program(
            "trait A { fn a(self) -> int; }
             trait B extends A { fn b(self) -> int; }
             trait C extends B { fn c(self) -> int; }
             struct S { int x, }
             impl B for S { fn b(self) -> int { return 2; } }
             impl C for S { fn c(self) -> int { return 3; } }",
        );
        let err = check(&prog, "test.rz").expect_err("expected error");
        assert!(
            err.contains("missing required super-trait"),
            "wrong error: {}",
            err
        );
    }

    #[test]
    fn unknown_super_trait_is_error() {
        let prog = parse_program(
            "trait Foo extends NonExistent { fn foo(self) -> int; }
             struct S { int x, }",
        );
        let err = check(&prog, "test.rz").expect_err("expected error");
        assert!(
            err.contains("extends unknown trait"),
            "wrong error: {}",
            err
        );
    }

    #[test]
    fn no_supers_always_ok() {
        let prog = parse_program(
            "trait Standalone { fn thing(self) -> int; }
             struct S { int x, }
             impl Standalone for S { fn thing(self) -> int { return 42; } }",
        );
        check(&prog, "test.rz").expect("expected ok");
    }
}
