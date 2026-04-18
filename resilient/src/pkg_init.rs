//! RES-205: `resilient pkg init <name>` — scaffolds a minimal project.
//!
//! Not a full package manager (follow-ups under G?? will handle deps,
//! build graph, publish). This module just lays down three files so a
//! new user has something to run:
//!
//! - `Resilient.toml`  — manifest with `[package]` table
//! - `src/main.rs`     — hello-world entry point
//! - `.gitignore`      — ignore build artifacts
//!
//! Design rules:
//! - **Refuse to clobber.** If the target directory already exists and
//!   is non-empty, bail. Fresh directory creation is allowed.
//! - **Two-phase fallback** isn't worth the complexity — if any of the
//!   three writes fails mid-stream, we surface the error and stop; the
//!   user can re-run after fixing whatever blocked us.
//! - **Edition string** is a date. Not semantic versioning, so it's
//!   clear this is a "breaking-changes window" field rather than a
//!   compiler version. `2026-04` matches today's manager date; future
//!   editions will bump monotonically.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The edition string embedded in freshly-generated manifests. A
/// date rather than a semver — see module doc-comment.
pub const DEFAULT_EDITION: &str = "2026-04";

/// Result of scaffolding. Carries the directory we created so the
/// CLI can print a helpful "cd into it and run" line on success.
#[derive(Debug)]
pub struct Scaffold {
    pub root: PathBuf,
    pub wrote: Vec<PathBuf>,
}

/// Errors that can happen during `pkg init`. Kept as an enum rather
/// than a string so tests can match on the variant and the CLI can
/// format each case with a tailored message.
#[derive(Debug)]
pub enum PkgInitError {
    /// The user didn't supply `<name>`.
    MissingName,
    /// `<name>` contains a character we refuse (path separator, …).
    /// Carries the offending name so the error message can echo it.
    InvalidName(String),
    /// Target directory exists and is non-empty.
    DirectoryNotEmpty(PathBuf),
    /// Something went wrong on disk — surface the `io::Error`.
    Io(io::Error),
}

impl From<io::Error> for PkgInitError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl std::fmt::Display for PkgInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingName => write!(
                f,
                "`resilient pkg init` requires a project name: \
                 `resilient pkg init <name>`"
            ),
            Self::InvalidName(n) => write!(
                f,
                "invalid project name `{}`: names must be non-empty \
                 and may not contain path separators or whitespace",
                n
            ),
            Self::DirectoryNotEmpty(p) => write!(
                f,
                "refusing to scaffold into `{}`: directory already \
                 exists and is non-empty",
                p.display()
            ),
            Self::Io(e) => write!(f, "i/o error: {}", e),
        }
    }
}

/// Validate a user-supplied project name. We intentionally allow a
/// permissive character set (letters, digits, `_`, `-`, `.`) so the
/// name can double as a directory AND a Cargo-style identifier —
/// the manifest just stores it verbatim, no further munging.
fn validate_name(name: &str) -> Result<(), PkgInitError> {
    if name.is_empty() {
        return Err(PkgInitError::InvalidName(String::new()));
    }
    for c in name.chars() {
        let ok = c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.');
        if !ok {
            return Err(PkgInitError::InvalidName(name.to_string()));
        }
    }
    // Additional guard: reject `.` and `..` as project names since
    // they'd cause surprising behaviour (scaffold into cwd / parent).
    if matches!(name, "." | "..") {
        return Err(PkgInitError::InvalidName(name.to_string()));
    }
    Ok(())
}

/// Scaffold `<parent>/<name>`. `parent` lets tests redirect into a
/// temp dir; the CLI always passes `std::env::current_dir()`.
///
/// Behaviour:
/// - Non-existent target → create it and the three files.
/// - Existing empty target → reuse and create the three files.
/// - Existing non-empty target → `Err(DirectoryNotEmpty)`, no writes.
///
/// Returns the paths of every file we wrote in the order written.
pub fn scaffold_in(parent: &Path, name: &str) -> Result<Scaffold, PkgInitError> {
    validate_name(name)?;
    let root = parent.join(name);

    // Directory-state check. If it exists and has any entries,
    // refuse — users can opt in to "merge into empty dir" by
    // creating an empty dir first; a non-empty dir is opt-out
    // territory.
    if root.exists() {
        let is_non_empty = fs::read_dir(&root)?.next().is_some();
        if is_non_empty {
            return Err(PkgInitError::DirectoryNotEmpty(root));
        }
    } else {
        fs::create_dir(&root)?;
    }

    let src_dir = root.join("src");
    fs::create_dir_all(&src_dir)?;

    // Write manifest. Keep formatting byte-for-byte stable so the
    // unit test can assert verbatim.
    let manifest_path = root.join("Resilient.toml");
    let manifest = render_manifest(name);
    fs::write(&manifest_path, manifest)?;

    // Write hello-world entry point. The `int _d` param is the
    // idiom examples/*.rs use — every `fn main` in this codebase
    // takes a dummy int so the caller (`main(0);`) always supplies
    // one arg.
    let main_path = src_dir.join("main.rs");
    let main_src = render_hello_world();
    fs::write(&main_path, main_src)?;

    // Write .gitignore. Cargo-like conventions: one entry per line,
    // trailing newline.
    let gitignore_path = root.join(".gitignore");
    fs::write(&gitignore_path, render_gitignore())?;

    Ok(Scaffold {
        root,
        wrote: vec![manifest_path, main_path, gitignore_path],
    })
}

/// Render the `Resilient.toml` body. Pure — factored out so tests
/// can assert on the exact bytes without an on-disk round-trip.
pub fn render_manifest(name: &str) -> String {
    format!(
        "[package]\n\
         name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         edition = \"{edition}\"\n",
        name = name,
        edition = DEFAULT_EDITION,
    )
}

/// The hello-world `src/main.rs` body. Pinned so the template
/// stays deterministic across runs.
///
/// Written with explicit `\n    ` rather than Rust's line-
/// continuation `\<newline>` so the leading indentation on body
/// lines survives. Rust collapses the whitespace after a
/// trailing backslash in a string literal.
pub fn render_hello_world() -> &'static str {
    "// Welcome to Resilient.\n\
//\n\
// Run with:\n\
//   resilient src/main.rs\n\
fn main(int _d) {\n    println(\"Hello, world!\");\n    return 0;\n}\nmain(0);\n"
}

/// The `.gitignore` body. Kept minimal — just the two directories
/// the ticket calls out; users will add more as they go.
pub fn render_gitignore() -> &'static str {
    "target/\n\
     cert/\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn tmp_parent(tag: &str) -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "res_pkg_init_{}_{}_{}",
            tag,
            std::process::id(),
            n
        ));
        fs::create_dir_all(&p).expect("mkdir tmp parent");
        p
    }

    #[test]
    fn scaffold_creates_all_three_files() {
        let parent = tmp_parent("create");
        let out = scaffold_in(&parent, "my-proj").expect("scaffold ok");
        assert_eq!(out.root, parent.join("my-proj"));
        assert_eq!(out.wrote.len(), 3);
        // Every promised file exists.
        assert!(parent.join("my-proj/Resilient.toml").exists());
        assert!(parent.join("my-proj/src/main.rs").exists());
        assert!(parent.join("my-proj/.gitignore").exists());

        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn manifest_contents_match_template() {
        let parent = tmp_parent("manifest");
        scaffold_in(&parent, "cool_proj").expect("scaffold");
        let got = fs::read_to_string(parent.join("cool_proj/Resilient.toml"))
            .expect("read manifest");
        let expected = format!(
            "[package]\nname = \"cool_proj\"\nversion = \"0.1.0\"\nedition = \"{}\"\n",
            DEFAULT_EDITION,
        );
        assert_eq!(got, expected);
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn hello_world_main_runs_via_template() {
        let parent = tmp_parent("hello");
        scaffold_in(&parent, "greetings").expect("scaffold");
        let got = fs::read_to_string(parent.join("greetings/src/main.rs"))
            .expect("read main.rs");
        assert!(got.contains("fn main"), "expected fn main in: {got}");
        assert!(got.contains("Hello, world!"), "expected greeting in: {got}");
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn gitignore_ignores_target_and_cert() {
        let parent = tmp_parent("gitignore");
        scaffold_in(&parent, "proj").expect("scaffold");
        let got = fs::read_to_string(parent.join("proj/.gitignore"))
            .expect("read gitignore");
        assert!(got.contains("target/"));
        assert!(got.contains("cert/"));
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn scaffold_refuses_non_empty_directory() {
        let parent = tmp_parent("nonempty");
        let target = parent.join("existing");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("stray.txt"), "preexisting content").unwrap();

        let err = scaffold_in(&parent, "existing")
            .expect_err("scaffold should refuse");
        assert!(
            matches!(err, PkgInitError::DirectoryNotEmpty(_)),
            "unexpected error: {:?}", err
        );
        // Guarantee: the stray file survived unchanged.
        assert_eq!(
            fs::read_to_string(target.join("stray.txt")).unwrap(),
            "preexisting content",
        );
        // AND no manifest was written.
        assert!(!target.join("Resilient.toml").exists());
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn scaffold_accepts_preexisting_empty_directory() {
        // The policy: if a user `mkdir`s the target first, we still
        // scaffold. Only NON-empty refuses.
        let parent = tmp_parent("emptydir");
        let target = parent.join("fresh");
        fs::create_dir(&target).unwrap();
        scaffold_in(&parent, "fresh").expect("scaffold should succeed");
        assert!(target.join("Resilient.toml").exists());
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn validate_name_rejects_path_separators() {
        assert!(matches!(
            validate_name("foo/bar"),
            Err(PkgInitError::InvalidName(_))
        ));
        assert!(matches!(
            validate_name("../escape"),
            Err(PkgInitError::InvalidName(_))
        ));
    }

    #[test]
    fn validate_name_rejects_whitespace_and_empty() {
        assert!(matches!(
            validate_name(""),
            Err(PkgInitError::InvalidName(_))
        ));
        assert!(matches!(
            validate_name("has space"),
            Err(PkgInitError::InvalidName(_))
        ));
    }

    #[test]
    fn validate_name_rejects_dot_and_dotdot() {
        assert!(matches!(
            validate_name("."),
            Err(PkgInitError::InvalidName(_))
        ));
        assert!(matches!(
            validate_name(".."),
            Err(PkgInitError::InvalidName(_))
        ));
    }

    #[test]
    fn validate_name_accepts_common_forms() {
        assert!(validate_name("foo").is_ok());
        assert!(validate_name("foo_bar").is_ok());
        assert!(validate_name("foo-bar").is_ok());
        assert!(validate_name("Foo.Bar").is_ok());
        assert!(validate_name("proj123").is_ok());
    }
}
