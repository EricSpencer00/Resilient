//! Module imports for Resilient.
//!
//! Supports three forms of import:
//!
//! 1. **File imports**: `use "path/to/other.rz";` — imports `pub` declarations
//!    from the referenced file. Without `pub`, declarations are private.
//!    Legacy behaviour: if no declarations are marked `pub`, all are imported
//!    (backward compatibility with pre-visibility code).
//!
//! 2. **Namespaced file imports**: `use "path" as name;` — like above but
//!    declarations are scoped under `name::`.
//!
//! 3. **Standard library imports**: `use std::http;` / `use std::json as j;`
//!    — imports a built-in standard library module.
//!
//! Cycles are detected via an in-flight stack: before expanding a file,
//! we check if it's already being expanded higher up the call chain.
//! If so, a clean diagnostic shows the full cycle path.
//! Files already fully loaded are skipped on re-import (dedup by canonical path).
//!
//! Visibility:
//! - `pub fn name(...)` marks a function as exported.
//! - `pub struct Name { ... }` marks a struct as exported.
//! - Declarations without `pub` are private to their file.
//! - If NO declarations have `pub` in a file, ALL are exported (legacy mode).

use crate::span::Spanned;
use crate::{Node, parse};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Tracks pending standard library imports discovered during expansion.
/// These are collected and returned so the caller can inject bindings
/// into the interpreter environment after parsing.
#[derive(Debug, Clone)]
pub struct StdImport {
    pub module: String,
    pub alias: Option<String>,
}

/// Expand every `Node::Use` in `program`'s top level.
///
/// File-based imports are resolved, parsed, and spliced in.
/// Standard library imports (`use std::X;`) are collected into `std_imports`
/// for the caller to inject at runtime.
///
/// `base_dir` is the directory file paths are resolved against.
/// `loaded` tracks files already pulled in (dedup + cycle detection).
pub fn expand_uses(
    program: &mut Node,
    base_dir: &Path,
    loaded: &mut HashSet<PathBuf>,
) -> Result<(), String> {
    expand_uses_with_std(program, base_dir, loaded, &mut Vec::new())
}

/// Like `expand_uses` but also collects `use std::X;` imports.
pub fn expand_uses_with_std(
    program: &mut Node,
    base_dir: &Path,
    loaded: &mut HashSet<PathBuf>,
    std_imports: &mut Vec<StdImport>,
) -> Result<(), String> {
    let mut in_flight: Vec<PathBuf> = Vec::new();
    expand_recursive(program, base_dir, loaded, std_imports, &mut in_flight)
}

fn expand_recursive(
    program: &mut Node,
    base_dir: &Path,
    loaded: &mut HashSet<PathBuf>,
    std_imports: &mut Vec<StdImport>,
    in_flight: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return Ok(()),
    };

    if !stmts.iter().any(|s| matches!(&s.node, Node::Use { .. })) {
        return Ok(());
    }

    let mut expanded: Vec<Spanned<Node>> = Vec::with_capacity(stmts.len());
    for stmt in stmts.drain(..) {
        if let Node::Use { path, alias, .. } = &stmt.node {
            let alias = alias.clone();

            // Check for standard library import: `use std::module;`
            if let Some(module_name) = path.strip_prefix("std::") {
                std_imports.push(StdImport {
                    module: module_name.to_string(),
                    alias,
                });
                continue;
            }

            // Check for package dependency import: `use dep::module;`
            // Falls through to file-path resolution if not a known dep.
            if let Some((dep_name, module)) = path.split_once("::")
                && let Ok(Some(dep_path)) =
                    crate::pkg_deps::resolve_dep_module(base_dir, dep_name, module)
            {
                let canon = canonicalize_or_self(&dep_path);

                // RES-2540: cycle detection — check in-flight stack.
                check_cycle(&canon, in_flight)?;

                if alias.is_none() && !loaded.contains(&canon) {
                    loaded.insert(canon.clone());
                } else if alias.is_none() {
                    continue;
                }
                let imported_program = load_and_parse(&dep_path)?;
                let imported_dir = dep_path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from("."));
                let mut imported_program = imported_program;
                in_flight.push(canon);
                expand_recursive(
                    &mut imported_program,
                    &imported_dir,
                    loaded,
                    std_imports,
                    in_flight,
                )?;
                in_flight.pop();
                if let Node::Program(imported_stmts) = imported_program {
                    let has_any_pub = imported_stmts.iter().any(|s| is_pub_decl(&s.node));
                    let ns = alias.clone().unwrap_or_else(|| dep_name.to_string());
                    for s in imported_stmts {
                        if matches!(s.node, Node::Use { .. }) {
                            continue;
                        }
                        if has_any_pub && is_exportable_decl(&s.node) && !is_pub_decl(&s.node) {
                            continue;
                        }
                        let renamed = rename_decl(s, &ns);
                        expanded.push(renamed);
                    }
                }
                continue;
            }

            let target = resolve_use_path(base_dir, path)?;
            let canon = canonicalize_or_self(&target);

            // RES-2540: cycle detection — check in-flight stack.
            check_cycle(&canon, in_flight)?;

            if alias.is_none() {
                if loaded.contains(&canon) {
                    continue;
                }
                loaded.insert(canon.clone());
            }

            let imported_program = load_and_parse(&target)?;
            let imported_dir = target
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));

            let mut imported_program = imported_program;
            in_flight.push(canon);
            expand_recursive(
                &mut imported_program,
                &imported_dir,
                loaded,
                std_imports,
                in_flight,
            )?;
            in_flight.pop();

            if let Node::Program(imported_stmts) = imported_program {
                let has_any_pub = imported_stmts.iter().any(|s| is_pub_decl(&s.node));

                for s in imported_stmts {
                    if matches!(s.node, Node::Use { .. }) {
                        continue;
                    }

                    if has_any_pub && is_exportable_decl(&s.node) && !is_pub_decl(&s.node) {
                        continue;
                    }

                    if let Some(ref ns) = alias {
                        let renamed = rename_decl(s, ns);
                        expanded.push(renamed);
                    } else {
                        expanded.push(s);
                    }
                }
            }
        } else {
            expanded.push(stmt);
        }
    }
    *stmts = expanded;
    Ok(())
}

/// Check if a file is already being expanded (cycle detection).
/// If so, produce a diagnostic showing the full cycle path.
fn check_cycle(canon: &Path, in_flight: &[PathBuf]) -> Result<(), String> {
    if let Some(idx) = in_flight.iter().position(|p| p == canon) {
        let mut chain: Vec<String> = in_flight[idx..]
            .iter()
            .map(|p| {
                p.file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.display().to_string())
            })
            .collect();
        chain.push(
            canon
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| canon.display().to_string()),
        );
        return Err(format!(
            "error: circular import detected: {}",
            chain.join(" -> ")
        ));
    }
    Ok(())
}

/// Check if a node is marked with `pub` visibility.
fn is_pub_decl(node: &Node) -> bool {
    match node {
        Node::Function { is_pub, .. } => *is_pub,
        Node::StructDecl { is_pub, .. } => *is_pub,
        _ => false,
    }
}

/// Check if a node is a declaration that could be exported.
fn is_exportable_decl(node: &Node) -> bool {
    matches!(node, Node::Function { .. } | Node::StructDecl { .. })
}

/// Rename an imported declaration by prepending `ns::` to its name.
fn rename_decl(mut s: Spanned<Node>, ns: &str) -> Spanned<Node> {
    match &mut s.node {
        Node::Function { name, .. } => {
            *name = format!("{}::{}", ns, name);
        }
        Node::StructDecl { name, .. } => {
            *name = format!("{}::{}", ns, name);
        }
        _ => {}
    }
    s
}

fn resolve_use_path(base_dir: &Path, path: &str) -> Result<PathBuf, String> {
    let candidate = base_dir.join(path);
    if !candidate.exists() {
        return Err(format!(
            "use \"{}\" could not be resolved (looked in {})",
            path,
            base_dir.display()
        ));
    }
    Ok(candidate)
}

fn canonicalize_or_self(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn load_and_parse(path: &Path) -> Result<Node, String> {
    let src = fs::read_to_string(path)
        .map_err(|e| format!("failed to read import \"{}\": {}", path.display(), e))?;
    let (program, errors) = parse(&src);
    if !errors.is_empty() {
        return Err(format!(
            "import \"{}\" contained {} parser error(s)",
            path.display(),
            errors.len()
        ));
    }
    Ok(program)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rz_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn cleanup_temp_dir(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn expand_is_a_noop_when_there_are_no_uses() {
        let (mut program, errs) = crate::parse("fn main() { return 1; }");
        assert!(errs.is_empty());
        let before_len = match &program {
            Node::Program(s) => s.len(),
            _ => unreachable!(),
        };
        let mut loaded = HashSet::new();
        expand_uses(&mut program, Path::new("."), &mut loaded).unwrap();
        let after_len = match &program {
            Node::Program(s) => s.len(),
            _ => unreachable!(),
        };
        assert_eq!(before_len, after_len);
        assert!(loaded.is_empty());
    }

    #[test]
    fn missing_import_is_a_clean_error_not_a_panic() {
        let (mut program, _) = crate::parse("use \"nope-does-not-exist.rz\";");
        let mut loaded = HashSet::new();
        let err = expand_uses(&mut program, Path::new("."), &mut loaded)
            .expect_err("missing file must error");
        assert!(err.contains("could not be resolved"), "got: {}", err);
    }

    #[test]
    fn std_imports_are_collected() {
        let (mut program, _) = crate::parse("use std::http;\nuse std::json as j;");
        let mut loaded = HashSet::new();
        let mut std_imports = Vec::new();
        expand_uses_with_std(&mut program, Path::new("."), &mut loaded, &mut std_imports).unwrap();
        assert_eq!(std_imports.len(), 2);
        assert_eq!(std_imports[0].module, "http");
        assert_eq!(std_imports[0].alias, None);
        assert_eq!(std_imports[1].module, "json");
        assert_eq!(std_imports[1].alias, Some("j".to_string()));
    }

    #[test]
    fn pub_visibility_filters_private_decls() {
        let (program, errs) =
            crate::parse("pub fn exported() { return 1; }\nfn private() { return 2; }");
        assert!(errs.is_empty());
        if let Node::Program(stmts) = &program {
            assert!(is_pub_decl(&stmts[0].node));
            assert!(!is_pub_decl(&stmts[1].node));
        } else {
            panic!("expected Program");
        }
    }

    #[test]
    fn circular_import_detected() {
        let dir = make_temp_dir().join("cycle2");
        let _ = fs::create_dir_all(&dir);
        fs::write(dir.join("a.rz"), "use \"b.rz\" as b;\nlet x = 1\n").unwrap();
        fs::write(dir.join("b.rz"), "use \"a.rz\" as a;\nlet y = 2\n").unwrap();

        let src = fs::read_to_string(dir.join("a.rz")).unwrap();
        let (mut program, _) = crate::parse(&src);
        let mut loaded = HashSet::new();
        let result = expand_uses(&mut program, &dir, &mut loaded);
        cleanup_temp_dir(&dir);
        assert!(result.is_err(), "circular import should error");
        let err = result.unwrap_err();
        assert!(
            err.contains("circular import"),
            "expected cycle error, got: {}",
            err
        );
    }

    #[test]
    fn three_file_cycle_detected() {
        let dir = make_temp_dir().join("cycle3");
        let _ = fs::create_dir_all(&dir);
        fs::write(dir.join("x.rz"), "use \"y.rz\" as y;\nlet a = 1\n").unwrap();
        fs::write(dir.join("y.rz"), "use \"z.rz\" as z;\nlet b = 2\n").unwrap();
        fs::write(dir.join("z.rz"), "use \"x.rz\" as x;\nlet c = 3\n").unwrap();

        let src = fs::read_to_string(dir.join("x.rz")).unwrap();
        let (mut program, _) = crate::parse(&src);
        let mut loaded = HashSet::new();
        let result = expand_uses(&mut program, &dir, &mut loaded);
        cleanup_temp_dir(&dir);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("circular import"), "got: {}", err);
    }

    #[test]
    fn non_aliased_reimport_deduplicates_no_error() {
        let dir = make_temp_dir().join("dedup");
        let _ = fs::create_dir_all(&dir);
        fs::write(dir.join("lib.rz"), "let shared = 42\n").unwrap();

        let src = "use \"lib.rz\";\nuse \"lib.rz\";\nlet x = shared\n";
        let (mut program, _) = crate::parse(src);
        let mut loaded = HashSet::new();
        let result = expand_uses(&mut program, &dir, &mut loaded);
        cleanup_temp_dir(&dir);
        assert!(result.is_ok(), "dedup should not error: {:?}", result);
    }

    #[test]
    fn check_cycle_finds_cycle() {
        let a = PathBuf::from("/a.rz");
        let b = PathBuf::from("/b.rz");
        let in_flight = vec![a.clone(), b.clone()];
        let result = check_cycle(&a, &in_flight);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("circular import"));
    }

    #[test]
    fn check_cycle_passes_for_new_file() {
        let a = PathBuf::from("/a.rz");
        let b = PathBuf::from("/b.rz");
        let c = PathBuf::from("/c.rz");
        let in_flight = vec![a, b];
        assert!(check_cycle(&c, &in_flight).is_ok());
    }
}
