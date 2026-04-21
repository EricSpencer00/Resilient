//! RES-205 / RES-212: `resilient pkg init [<name>] [--name <n>]` —
//! scaffolds a minimal project.
//!
//! Not a full package manager (follow-ups will handle deps, build
//! graph, publish). This module just lays down three files so a new
//! user has something to run:
//!
//! - `resilient.toml`  — manifest with `[package]` and `[dependencies]`
//! - `src/main.rz`     — hello-world entry point
//! - `.gitignore`      — ignore build artifacts
//!
//! Design rules:
//! - **Refuse to clobber.** If `resilient.toml` already exists in the
//!   target, bail — don't overwrite a user's manifest silently.
//!   Empty-directory scaffolding is still allowed.
//! - **Two-phase fallback** isn't worth the complexity — if any of the
//!   three writes fails mid-stream, we surface the error and stop; the
//!   user can re-run after fixing whatever blocked us.
//! - **Edition string** is a date. Not semantic versioning, so it's
//!   clear this is a "breaking-changes window" field rather than a
//!   compiler version. `2026-04` matches today's manager date; future
//!   editions will bump monotonically.
//!
//! RES-212 additions on top of RES-205:
//! - Manifest file is now lowercase `resilient.toml` (was `Resilient.toml`).
//! - Manifest carries `author = "..."` and an empty `[dependencies]`
//!   table so downstream tooling has a stable shape to target.
//! - `--name foo` flag supported as a non-interactive alternative to
//!   the positional `<name>` arg (handled in the CLI dispatcher).
//! - A tiny `read_package_name` helper lives here so the run path can
//!   surface the package name in error messages without dragging in a
//!   full TOML parser.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The edition string embedded in freshly-generated manifests. A
/// date rather than a semver — see module doc-comment.
pub const DEFAULT_EDITION: &str = "2026-04";

/// Default author string stamped into generated manifests when the
/// caller didn't supply one. Kept generic because `pkg init` can't
/// reliably discover the user's identity cross-platform (no git
/// config read, no `whoami` probe — both are footguns under sandbox).
pub const DEFAULT_AUTHOR: &str = "unknown";

/// Canonical manifest filename (lowercase per RES-212). Exposed as a
/// constant so the run path, tests, and future tooling all agree.
pub const MANIFEST_FILENAME: &str = "resilient.toml";

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
    /// `resilient.toml` already exists at the target — refuse to
    /// clobber the user's manifest even when the dir is otherwise
    /// empty (RES-212 idempotency guard).
    ManifestExists(PathBuf),
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
                 `resilient pkg init <name>` or `resilient pkg init --name <n>`"
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
            Self::ManifestExists(p) => write!(
                f,
                "refusing to overwrite existing manifest `{}`: \
                 remove it first if you really want to reinitialize",
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
/// - Pre-existing `resilient.toml` at the target → `Err(ManifestExists)`,
///   even if the directory itself is otherwise empty.
///
/// Returns the paths of every file we wrote in the order written.
pub fn scaffold_in(parent: &Path, name: &str) -> Result<Scaffold, PkgInitError> {
    validate_name(name)?;
    let root = parent.join(name);

    // Directory-state checks. Manifest-exists wins over the general
    // non-empty-dir error so callers get a sharper message — the
    // idempotency guard is the common case ("I already ran init").
    if root.exists() {
        let manifest_path = root.join(MANIFEST_FILENAME);
        if manifest_path.exists() {
            return Err(PkgInitError::ManifestExists(manifest_path));
        }
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
    let manifest_path = root.join(MANIFEST_FILENAME);
    let manifest = render_manifest(name, DEFAULT_AUTHOR);
    fs::write(&manifest_path, manifest)?;

    // Write hello-world entry point. The `int _d` param is the
    // idiom examples/*.rs use — every `fn main` in this codebase
    // takes a dummy int so the caller (`main(0);`) always supplies
    // one arg.
    let main_path = src_dir.join("main.rz");
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

/// Render the `resilient.toml` body. Pure — factored out so tests
/// can assert on the exact bytes without an on-disk round-trip.
///
/// `author` is injected so tests can stamp in a known value; the
/// CLI passes `DEFAULT_AUTHOR` unless the user supplied their own
/// in some future flag.
pub fn render_manifest(name: &str, author: &str) -> String {
    format!(
        "[package]\n\
         name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         author = \"{author}\"\n\
         edition = \"{edition}\"\n\
         \n\
         [dependencies]\n",
        name = name,
        author = author,
        edition = DEFAULT_EDITION,
    )
}

/// The hello-world `src/main.rz` body. Pinned so the template
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
//   resilient src/main.rz\n\
fn main(int _d) {\n    println(\"Hello, world!\");\n    return 0;\n}\nmain(0);\n"
}

/// The `.gitignore` body. Kept minimal — just the two directories
/// the ticket calls out; users will add more as they go.
pub fn render_gitignore() -> &'static str {
    "target/\n\
     cert/\n"
}

/// RES-212: minimal TOML-ish reader for the `[package].name` field.
///
/// Intentionally hand-rolled to keep the compiler's dep tree small.
/// We don't need a general TOML parser — just scan for a `name = "..."`
/// line that appears under the `[package]` table and before any
/// subsequent `[section]` header. Returns `None` when the file
/// doesn't exist, can't be read, or doesn't contain a parseable
/// `name`. Callers treat `None` as "no manifest, fall back to the
/// raw filename."
///
/// Supported shape (lenient on whitespace):
/// ```toml
/// [package]
/// name = "my-project"
/// version = "0.1.0"
/// ```
///
/// NOT supported (by design — these would drag in a real parser):
/// - Multi-line basic strings (`"""..."""`).
/// - Literal strings (`'...'`).
/// - Escape sequences inside the name (`"\u0041"` stays raw).
/// - Inline tables.
///
/// The manifests this function reads are the ones we write in
/// `render_manifest`, so the tight shape is fine in practice.
pub fn read_package_name(manifest_path: &Path) -> Option<String> {
    let contents = fs::read_to_string(manifest_path).ok()?;
    let mut in_package = false;
    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            // New section header. Enter `[package]`, leave anything
            // else.
            let header = rest.trim_end_matches(']').trim();
            in_package = header == "package";
            continue;
        }
        if !in_package {
            continue;
        }
        // Look for `name = "..."`. Split on the first `=`.
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "name" {
            continue;
        }
        // Expect `"..."` (double-quoted basic string).
        let v = val.trim().strip_prefix('"')?;
        // Tolerate a trailing comment: `name = "foo" # …`.
        let end = v.find('"')?;
        let name = &v[..end];
        if name.is_empty() {
            return None;
        }
        return Some(name.to_string());
    }
    None
}

/// RES-212: walk upward from `start` looking for a sibling
/// `resilient.toml`. Returns the manifest path if one is found.
///
/// `resilient run path/to/file.rz` conventionally lives inside a
/// project; rather than require the user to be in the project root,
/// we search `start`, `start/..`, … up to the filesystem root. This
/// matches how cargo finds `Cargo.toml`.
pub fn find_manifest_upwards(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };
    loop {
        let candidate = dir.join(MANIFEST_FILENAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn tmp_parent(tag: &str) -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let p =
            std::env::temp_dir().join(format!("res_pkg_init_{}_{}_{}", tag, std::process::id(), n));
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
        assert!(parent.join("my-proj/resilient.toml").exists());
        assert!(parent.join("my-proj/src/main.rz").exists());
        assert!(parent.join("my-proj/.gitignore").exists());

        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn manifest_contents_match_template() {
        let parent = tmp_parent("manifest");
        scaffold_in(&parent, "cool_proj").expect("scaffold");
        let got =
            fs::read_to_string(parent.join("cool_proj/resilient.toml")).expect("read manifest");
        let expected = format!(
            "[package]\nname = \"cool_proj\"\nversion = \"0.1.0\"\nauthor = \"{}\"\nedition = \"{}\"\n\n[dependencies]\n",
            DEFAULT_AUTHOR, DEFAULT_EDITION,
        );
        assert_eq!(got, expected);
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn manifest_includes_dependencies_table() {
        // Separate assertion so a future template change that drops
        // `[dependencies]` fails loudly rather than silently.
        let parent = tmp_parent("deps");
        scaffold_in(&parent, "projdeps").expect("scaffold");
        let got =
            fs::read_to_string(parent.join("projdeps/resilient.toml")).expect("read manifest");
        assert!(
            got.contains("[dependencies]"),
            "missing [dependencies] table in: {got}"
        );
        assert!(got.contains("author = "), "missing author field in: {got}");
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn hello_world_main_runs_via_template() {
        let parent = tmp_parent("hello");
        scaffold_in(&parent, "greetings").expect("scaffold");
        let got = fs::read_to_string(parent.join("greetings/src/main.rz")).expect("read main.rz");
        assert!(got.contains("fn main"), "expected fn main in: {got}");
        assert!(got.contains("Hello, world!"), "expected greeting in: {got}");
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn gitignore_ignores_target_and_cert() {
        let parent = tmp_parent("gitignore");
        scaffold_in(&parent, "proj").expect("scaffold");
        let got = fs::read_to_string(parent.join("proj/.gitignore")).expect("read gitignore");
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

        let err = scaffold_in(&parent, "existing").expect_err("scaffold should refuse");
        assert!(
            matches!(err, PkgInitError::DirectoryNotEmpty(_)),
            "unexpected error: {:?}",
            err
        );
        // Guarantee: the stray file survived unchanged.
        assert_eq!(
            fs::read_to_string(target.join("stray.txt")).unwrap(),
            "preexisting content",
        );
        // AND no manifest was written.
        assert!(!target.join("resilient.toml").exists());
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn scaffold_refuses_when_manifest_already_exists() {
        // RES-212 idempotency guard: a pre-existing `resilient.toml`
        // should trigger `ManifestExists`, distinct from the
        // general non-empty-dir error. This lets the CLI print a
        // sharper message and hint at removing the file.
        let parent = tmp_parent("manifest_exists");
        let target = parent.join("already");
        fs::create_dir(&target).unwrap();
        fs::write(
            target.join("resilient.toml"),
            "[package]\nname = \"already\"\n",
        )
        .unwrap();

        let err = scaffold_in(&parent, "already").expect_err("scaffold should refuse");
        assert!(
            matches!(err, PkgInitError::ManifestExists(_)),
            "unexpected error: {:?}",
            err
        );
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
        assert!(target.join("resilient.toml").exists());
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

    #[test]
    fn read_package_name_picks_up_field() {
        let parent = tmp_parent("readname");
        scaffold_in(&parent, "pkg_read_ok").expect("scaffold");
        let name = read_package_name(&parent.join("pkg_read_ok/resilient.toml"))
            .expect("should find name");
        assert_eq!(name, "pkg_read_ok");
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn read_package_name_ignores_unrelated_sections() {
        // `name = "…"` inside a non-`[package]` section must not be
        // reported; otherwise a future `[dependencies]` entry like
        // `name = "sqlite"` would masquerade as the package name.
        let parent = tmp_parent("readname_unrelated");
        let manifest = parent.join("m.toml");
        fs::write(
            &manifest,
            "[dependencies]\nname = \"not-the-package\"\n\n[package]\nname = \"real\"\n",
        )
        .unwrap();
        assert_eq!(read_package_name(&manifest).as_deref(), Some("real"),);
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn read_package_name_missing_file_is_none() {
        let p = std::env::temp_dir().join("definitely-not-here-912374.toml");
        assert!(read_package_name(&p).is_none());
    }

    #[test]
    fn find_manifest_upwards_walks_parents() {
        // Set up parent/proj/resilient.toml and start from
        // parent/proj/src/nested/ — the search should climb up.
        let parent = tmp_parent("walkup");
        let proj = parent.join("proj");
        let nested = proj.join("src/nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(proj.join("resilient.toml"), "[package]\nname = \"p\"\n").unwrap();
        let found = find_manifest_upwards(&nested).expect("should find");
        assert_eq!(found, proj.join("resilient.toml"));
        let _ = fs::remove_dir_all(&parent);
    }
}
