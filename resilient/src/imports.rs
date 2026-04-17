//! RES-073: minimum-viable module imports for Resilient.
//!
//! `use "path/to/other.res";` at the top level of a file imports every
//! top-level `fn` declaration of the referenced file into the current
//! scope. Resolution is path-based and relative to the file containing
//! the `use`. This module performs that expansion BEFORE the program
//! ever reaches the typechecker or interpreter, so by the time eval
//! starts there are no `Node::Use` nodes left and the imported
//! functions are simply prepended to the program's top-level statement
//! list.
//!
//! Cycles are detected via an in-flight set and produce a clean
//! diagnostic. Files already loaded once are skipped on re-import
//! (dedup by canonicalized path).
//!
//! NOT in scope here:
//! - Qualified names (`module::fn`)
//! - Visibility modifiers (`pub`)
//! - Submodules / packages
//! - Re-exports
//!
//! Those are intentional follow-ups; this is the foundation.

use crate::span::Spanned;
use crate::{parse, Node};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Expand every `Node::Use` in `program`'s top level by loading the
/// referenced file, recursively expanding ITS uses, and prepending
/// the resulting top-level `Function` (and `Struct` decl) nodes.
///
/// `base_dir` is the directory paths in `use` clauses are resolved
/// against — typically the parent of the file currently being parsed.
///
/// `loaded` is the set of canonicalized paths already pulled in (used
/// for both dedup and cycle detection).
pub fn expand_uses(
    program: &mut Node,
    base_dir: &Path,
    loaded: &mut HashSet<PathBuf>,
) -> Result<(), String> {
    // RES-077: top-level statements are now Spanned<Node>. Destructure
    // each `Spanned` to inspect / route the inner node, but preserve
    // the span on whatever we keep (so once RES-078..080 add spans to
    // sub-expressions the diagnostic chain stays intact end-to-end).
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return Ok(()),
    };

    let mut expanded: Vec<Spanned<Node>> = Vec::with_capacity(stmts.len());
    for stmt in stmts.drain(..) {
        if let Node::Use { path, .. } = &stmt.node {
            let target = resolve_use_path(base_dir, path)?;

            // Cycle / already-loaded check: canonicalize so that
            // `./helpers.res` and `helpers.res` collapse to one entry.
            let canon = canonicalize_or_self(&target);
            if loaded.contains(&canon) {
                // Already loaded once. Re-importing is a no-op — same
                // semantics as Rust's `use` after a `mod` was already
                // brought in.
                continue;
            }
            loaded.insert(canon.clone());

            let imported_program = load_and_parse(&target)?;
            let imported_dir = target
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));

            // Recursively expand imports of the imported file FIRST,
            // so by the time we splice we have only top-level decls.
            let mut imported_program = imported_program;
            expand_uses(&mut imported_program, &imported_dir, loaded)?;

            // Splice in the resulting top-level decls (everything
            // except residual Node::Use, which expand_uses already
            // drained).
            if let Node::Program(imported_stmts) = imported_program {
                for s in imported_stmts {
                    if !matches!(s.node, Node::Use { .. }) {
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
        let (mut program, _) = crate::parse("use \"nope-does-not-exist.res\";");
        let mut loaded = HashSet::new();
        let err = expand_uses(&mut program, Path::new("."), &mut loaded)
            .expect_err("missing file must error");
        assert!(err.contains("could not be resolved"), "got: {}", err);
    }
}
