//! RES-324: `mod name { ... }` inline namespace blocks.
//!
//! A `mod` block groups declarations under a namespace prefix. Every
//! `fn` declared inside `mod math { ... }` is registered in the
//! environment as `"math::fn_name"`. Call sites write `math::add(1, 2)`,
//! which the parser already collapses into a flat
//! `Node::Identifier { name: "math::add" }` via the `::` token path —
//! so no extra runtime lookup machinery is needed.

// RES-324: `modules::check` validates duplicate module names and
// unresolved `mod::item` references; wired into EXTENSION_PASSES.

use crate::{Environment, Interpreter, Node, RResult, Value};

/// Evaluate a `mod name { ... }` block.
///
/// Each `fn` in `body` is renamed to `"mod_name::fn_name"` before being
/// registered in the outer environment, making the binding visible to
/// subsequent call sites that use the `name::item` syntax.
///
/// Struct declarations inside the block are similarly prefixed.
/// Other statements (helper `let` bindings, bare expressions) are
/// evaluated in a temporary enclosed scope and do not pollute the outer
/// environment.
pub(crate) fn eval_module(
    mod_name: &str,
    body: &[Node],
    interp: &mut Interpreter,
) -> RResult<Value> {
    for node in body {
        match node {
            Node::Function { name, .. } => {
                let mut renamed = node.clone();
                if let Node::Function {
                    name: ref mut n, ..
                } = renamed
                {
                    *n = format!("{}::{}", mod_name, name);
                }
                interp.eval(&renamed)?;
            }
            Node::StructDecl { name, .. } => {
                let mut renamed = node.clone();
                if let Node::StructDecl {
                    name: ref mut n, ..
                } = renamed
                {
                    *n = format!("{}::{}", mod_name, name);
                }
                interp.eval(&renamed)?;
            }
            Node::ImplBlock { .. } => {
                // impl blocks inside modules are evaluated directly; their
                // methods are already parser-mangled with the struct name
                // and do not receive an extra namespace prefix here.
                interp.eval(node)?;
            }
            _ => {
                // For other statements evaluate them in a temporary child
                // scope so they cannot clobber outer bindings.
                let saved = interp.env.clone();
                interp.env = Environment::new_enclosed(saved.clone());
                let result = interp.eval(node);
                interp.env = saved;
                result?;
            }
        }
    }
    Ok(Value::Void)
}

/// Lightweight static pass — no-op for the MVP. Future extensions can
/// Validate module declarations:
///
/// 1. **Duplicate module names** — two `mod foo { }` blocks in the same
///    program are an error; the second silently shadows the first at
///    runtime, which is almost always a bug.
/// 2. **Unresolved `name::item` references** — identifiers of the form
///    `mod::item` where `mod` matches a declared `mod name { }` block
///    but `item` is not declared inside it are flagged with a more
///    specific message than the generic "undefined variable" the
///    typechecker would produce.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let Node::Program(stmts) = program else {
        return Ok(());
    };

    // First pass: collect module name → exported item names.
    let mut module_items: std::collections::HashMap<&str, std::collections::HashSet<&str>> =
        std::collections::HashMap::new();

    for s in stmts {
        if let Node::ModuleDecl { name, body, .. } = &s.node {
            let n = name.as_str();
            if module_items.contains_key(n) {
                return Err(format!(
                    "{}: error: duplicate module declaration `{}`; \
                     each module name must be declared at most once",
                    source_path, n
                ));
            }
            let mut exports: std::collections::HashSet<&str> = std::collections::HashSet::new();
            for item in body {
                match item {
                    Node::Function { name: fn_name, .. } => {
                        exports.insert(fn_name.as_str());
                    }
                    Node::StructDecl { name: st_name, .. } => {
                        exports.insert(st_name.as_str());
                    }
                    Node::EnumDecl { name: en_name, .. } => {
                        exports.insert(en_name.as_str());
                    }
                    _ => {}
                }
            }
            module_items.insert(n, exports);
        }
    }

    if module_items.is_empty() {
        return Ok(());
    }

    // Second pass: walk identifier nodes for `mod::item` references.
    // The typechecker's "undefined variable" check catches unresolved
    // identifiers; we only want to surface a *better* message for the
    // case where the module IS declared but the specific item IS NOT.
    //
    // RES-2352: use `any_node` instead of `visit` so the walk
    // terminates on the first unresolved reference. The previous shape
    // toggled an `Option<String>` then checked it at the top of the
    // closure, but `visit` still recurses through every remaining node
    // — every post-match call paid the dispatch in `walk_children` for
    // no reason. `any_node` (RES-1238) propagates `true` upward and
    // short-circuits siblings/aunts/the rest of the tree. Same shape
    // as RES-2340 (ghost_types) and RES-2342 (audit_log_required).
    let mut unresolved: Option<String> = None;
    crate::uniqueness_walk::any_node(program, |n| {
        if let Node::Identifier { name, .. } = n
            && let Some((mod_name, item_name)) = name.split_once("::")
            && let Some(exports) = module_items.get(mod_name)
            && !exports.contains(item_name)
        {
            unresolved = Some(format!(
                "{}: error: module `{}` does not export `{}`",
                source_path, mod_name, item_name
            ));
            true
        } else {
            false
        }
    });

    if let Some(e) = unresolved {
        return Err(e);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::parse;

    #[test]
    fn check_always_returns_ok_no_modules() {
        let (prog, _) = parse("fn f(int x) -> int { return x; }");
        assert!(super::check(&prog, "test.rz").is_ok());
    }

    #[test]
    fn check_ok_on_empty_program() {
        let (prog, _) = parse("");
        assert!(super::check(&prog, "test.rz").is_ok());
    }

    #[test]
    fn check_ok_single_module() {
        let src = r#"
mod math {
    fn add(int x, int y) -> int { return x + y; }
}
fn main(int _d) -> int { return math::add(1, 2); }
"#;
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        assert!(super::check(&prog, "test.rz").is_ok());
    }

    #[test]
    fn check_errors_on_duplicate_module() {
        // Two `mod math` blocks should be rejected.
        let src = r#"
mod math { fn add(int x, int y) -> int { return x + y; } }
mod math { fn sub(int x, int y) -> int { return x - y; } }
"#;
        let (prog, _) = parse(src);
        let result = super::check(&prog, "test.rz");
        assert!(
            result.is_err(),
            "expected error for duplicate module declaration"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("duplicate module declaration"),
            "error message must mention 'duplicate module declaration': {msg}"
        );
        assert!(
            msg.contains("math"),
            "error message must name the module: {msg}"
        );
    }

    #[test]
    fn check_errors_on_unresolved_mod_item() {
        // `math::mul` is not declared in `mod math`.
        let src = r#"
mod math { fn add(int x, int y) -> int { return x + y; } }
fn main(int _d) -> int { return math::mul(2, 3); }
"#;
        let (prog, _) = parse(src);
        let result = super::check(&prog, "test.rz");
        assert!(result.is_err(), "expected error for unresolved module item");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("does not export"),
            "error message must mention 'does not export': {msg}"
        );
    }

    #[test]
    fn check_ok_when_mod_item_exists() {
        let src = r#"
mod utils { fn greet(int _x) -> int { return 1; } }
fn main(int _d) -> int { return utils::greet(0); }
"#;
        let (prog, _) = parse(src);
        // Both parse and modules check should succeed.
        assert!(super::check(&prog, "test.rz").is_ok());
    }

    #[test]
    fn module_fn_is_registered_under_qualified_name() {
        // A fn declared inside `mod math { fn add ... }` should be
        // callable as `math::add` from the outer scope.
        let src = r#"
            mod math {
                fn add(int x, int y) -> int { return x + y; }
            }
            fn main(int _d) -> int { return math::add(1, 2); }
        "#;
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = crate::Interpreter::new();
        let result = interp.eval(&prog);
        assert!(result.is_ok(), "eval failed: {:?}", result);
    }

    #[test]
    fn eval_module_registers_struct_with_prefix() {
        // A struct declared inside a mod block should be accessible as
        // `modname::StructName` in the outer scope.
        let src = r#"
            mod geo {
                struct Point { int x, int y }
            }
            fn main(int _d) -> int { return 0; }
        "#;
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = crate::Interpreter::new();
        // If struct registration fails the eval would error.
        assert!(interp.eval(&prog).is_ok());
    }

    #[test]
    fn eval_module_returns_void() {
        // eval_module should return Ok(Value::Void) for an empty body.
        let mut interp = crate::Interpreter::new();
        let result = super::eval_module("empty", &[], &mut interp);
        assert!(result.is_ok(), "eval_module failed: {:?}", result);
        assert!(
            matches!(result.unwrap(), crate::Value::Void),
            "expected Void return from empty module"
        );
    }
}
