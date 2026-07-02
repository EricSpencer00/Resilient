//! RES-3838: Package existence verification at import-resolution time.
//!
//! Validates that every `use pkg::module` import refers to a known
//! package name. File-based imports (`use "path/to/file.rz"`) and
//! standard library imports (`use std::*`) are exempt — those are
//! validated elsewhere.
//!
//! ## Why this lives in `imports.rs`, not a typechecker pass
//!
//! The obvious design — a `check_package_existence` pass wired into
//! `typechecker.rs`'s `<EXTENSION_PASSES>` block — cannot work in this
//! codebase: `imports::expand_uses_with_std` runs *before* typecheck and
//! unconditionally drains every top-level `Node::Use` from the program,
//! either splicing in the resolved content or returning early with an
//! "Import error". By the time a typechecker pass would run, there are
//! no `Node::Use` nodes left to inspect, and an unresolvable package
//! path has already aborted compilation with a generic
//! "could not be resolved (looked in ...)" file-not-found message.
//!
//! So the check must happen inline in `imports::expand_recursive`, at
//! the exact point where a `pkg::module` path fails the std-import and
//! dependency-module lookups and is about to fall through to file-path
//! resolution. `check_known_package` is called from there.
//!
//! ## Enforcement rules
//!
//! 1. `use std::*` is always allowed — handled entirely by the caller
//!    before this module is ever consulted.
//! 2. `use dep::module` where `dep` resolves via `pkg_deps::resolve_dep_module`
//!    is always allowed — also handled entirely by the caller.
//! 3. Otherwise, if the top-level segment `dep` is not one of the
//!    built-in package names below, and not the name of a dependency
//!    declared in the nearest `resilient.toml`'s `[dependencies]` table,
//!    this is treated as a hallucinated package name and rejected with
//!    an actionable diagnostic — this catches AI-generated code that
//!    invents plausible-sounding package names.
//!
//! ## Known built-in packages
//!
//! This list mirrors the standard library modules that the runtime
//! provides. It must stay in sync with the modules listed in
//! `resilient-runtime` and the stdlib dispatch in `stdlib.rs`.

use std::path::Path;

/// Built-in top-level package names that are always valid.
const BUILTIN_PACKAGES: &[&str] = &[
    "std",
    // stdlib modules that can appear as top-level qualifiers
    "http",
    "json",
    "math",
    "io",
    "os",
    "time",
    "fs",
    "net",
    "sync",
    "fmt",
    "bytes",
    "crypto",
    "rand",
    "env",
    "process",
    "regex",
    "log",
    "test",
    // well-known embedded platform crates that Resilient ships examples for
    "cortex_m",
    "riscv",
    "embedded_hal",
];

/// Load declared dependency names from the nearest `resilient.toml`
/// found by walking up from `base_dir`. Reuses the real manifest
/// infrastructure (`pkg_init`/`pkg_deps`) rather than re-parsing TOML,
/// so this stays in sync with the actual dependency resolution rules
/// (inline-table and string-shorthand syntax, `[[...]]` table skipping).
fn declared_dep_names(base_dir: &Path) -> Vec<String> {
    let Some(manifest_path) = crate::pkg_init::find_manifest_upwards(base_dir) else {
        return Vec::new();
    };
    crate::pkg_deps::parse_dependencies(&manifest_path)
        .map(|deps| deps.into_iter().map(|d| d.name).collect())
        .unwrap_or_default()
}

/// Verify that `pkg` (the top-level segment of a `use pkg::module`
/// import) is a known package: either a built-in or a dependency
/// declared in the project's `resilient.toml`. `full_path` is the
/// complete import path, used only for the diagnostic; `base_dir` is
/// the directory to search upward from for a manifest.
///
/// Called from `imports::expand_recursive` after the std-import and
/// dependency-module lookups have both declined the path — i.e. only
/// for paths that are about to be (mis-)treated as a literal file path.
pub(crate) fn check_known_package(
    pkg: &str,
    full_path: &str,
    base_dir: &Path,
) -> Result<(), String> {
    if BUILTIN_PACKAGES.contains(&pkg) {
        return Ok(());
    }
    if declared_dep_names(base_dir).iter().any(|d| d == pkg) {
        return Ok(());
    }
    Err(format!(
        "unknown package `{pkg}` in `use {full_path}` — package does not exist in \
         the built-in registry or resilient.toml dependencies; add it to \
         `[dependencies]` or fix the import"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "res_pkg_existence_{}_{}_{}",
            tag,
            std::process::id(),
            n
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).expect("mkdir tmp");
        p
    }

    #[test]
    fn builtin_top_level_packages_allowed() {
        let dir = tmp_dir("builtin");
        assert!(check_known_package("http", "http::client", &dir).is_ok());
        assert!(check_known_package("math", "math::trig", &dir).is_ok());
    }

    #[test]
    fn unknown_package_is_rejected() {
        let dir = tmp_dir("unknown");
        let err =
            check_known_package("hallucinated_pkg", "hallucinated_pkg::Foo", &dir).unwrap_err();
        assert!(
            err.contains("unknown package `hallucinated_pkg`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn declared_manifest_dependency_is_allowed() {
        let dir = tmp_dir("manifest_dep");
        fs::write(
            dir.join("resilient.toml"),
            "[package]\nname = \"proj\"\n\n[dependencies]\nmylib = { path = \"mylib\" }\n",
        )
        .unwrap();
        assert!(check_known_package("mylib", "mylib::helpers", &dir).is_ok());
    }

    #[test]
    fn undeclared_package_with_manifest_present_is_still_rejected() {
        let dir = tmp_dir("manifest_no_dep");
        fs::write(
            dir.join("resilient.toml"),
            "[package]\nname = \"proj\"\n\n[dependencies]\nmylib = { path = \"mylib\" }\n",
        )
        .unwrap();
        let err = check_known_package("ghost_pkg", "ghost_pkg::Foo", &dir).unwrap_err();
        assert!(err.contains("ghost_pkg"), "unexpected: {err}");
    }

    #[test]
    fn no_manifest_present_only_builtins_allowed() {
        let dir = tmp_dir("no_manifest");
        assert!(check_known_package("std", "std::io", &dir).is_ok());
        assert!(check_known_package("random_dep", "random_dep::X", &dir).is_err());
    }
}
