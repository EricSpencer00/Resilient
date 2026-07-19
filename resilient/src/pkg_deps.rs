//! Package dependency resolution for Resilient.
//!
//! Parses `[dependencies]` from `resilient.toml`, resolves path-based
//! and git-based deps, and manages a `resilient.lock` lockfile.
//!
//! Dependency shapes:
//!
//! ```toml
//! [dependencies]
//! mylib = { path = "../libs/mylib" }
//! netutil = { git = "https://github.com/user/netutil", rev = "abc123" }
//! ```
//!
//! Path deps are validated to have a `resilient.toml` and a `src/`
//! directory. Git deps are cloned into `~/.resilient/cache/git/<hash>/`
//! and checked out at the specified rev/tag/branch.
//!
//! The `rz pkg add` CLI command appends entries:
//!
//! ```text
//! rz pkg add mylib path:../libs/mylib
//! rz pkg add netutil git:https://github.com/user/netutil --rev abc123
//! ```
//!
//! Lockfile format (`resilient.lock`):
//!
//! ```toml
//! [[package]]
//! name = "mylib"
//! source = "path:../libs/mylib"
//!
//! [[package]]
//! name = "netutil"
//! source = "git:https://github.com/user/netutil"
//! rev = "abc123"
//! ```

// RES-4114: static JSON registry index format + checksum verification
// (increment 1 of E-E2). See `docs/PACKAGE_REGISTRY.md`.
#[path = "pkg_registry.rs"]
pub mod pkg_registry;

use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::pkg_init;

// ── Dependency types ──────────────────────────────────────────────

/// A single dependency declared in `[dependencies]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub name: String,
    pub source: DepSource,
}

/// Where a dependency comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DepSource {
    Path {
        path: String,
    },
    Git {
        url: String,
        rev: Option<String>,
        tag: Option<String>,
        branch: Option<String>,
    },
    /// RES-4114: resolved from a static JSON registry index (see
    /// `pkg_registry.rs`). Always an exact, already-resolved version
    /// pin — "latest" is resolved to a concrete version at `pkg add`
    /// / `pkg update` time, never re-resolved implicitly on a plain
    /// build, so builds stay reproducible without a network call.
    Registry {
        version: String,
    },
}

/// A resolved dependency — we know where its source lives on disk.
#[derive(Debug, Clone)]
pub struct ResolvedDep {
    pub name: String,
    pub source: DepSource,
    /// The on-disk directory containing the dep's project root
    /// (the directory that holds `resilient.toml`). Used by lockfile
    /// generation and will be read by future build-graph code.
    #[allow(dead_code)]
    pub root: PathBuf,
    /// The `src/` directory within the dep's project root.
    pub src_dir: PathBuf,
}

// ── Lockfile types ────────────────────────────────────────────────

/// An entry in `resilient.lock`. Read by tests and future lockfile
/// consumers (e.g. `rz pkg update`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct LockEntry {
    pub name: String,
    /// `"path:../libs/mylib"` or `"git:https://..."`.
    pub source: String,
    /// Populated for git deps — the pinned revision.
    pub rev: Option<String>,
}

// ── Errors ────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum PkgDepsError {
    ManifestNotFound {
        searched_from: PathBuf,
    },
    ManifestUnreadable {
        path: PathBuf,
        source: io::Error,
    },
    MalformedDependency {
        name: String,
        detail: String,
    },
    PathDepNotFound {
        name: String,
        path: PathBuf,
    },
    PathDepMissingManifest {
        name: String,
        path: PathBuf,
    },
    PathDepMissingSrc {
        name: String,
        path: PathBuf,
    },
    GitCloneFailed {
        name: String,
        url: String,
        detail: String,
    },
    GitCheckoutFailed {
        name: String,
        detail: String,
    },
    LockfileIo {
        path: PathBuf,
        source: io::Error,
    },
    ManifestWriteError {
        path: PathBuf,
        source: io::Error,
    },
    DependencyNotFound {
        name: String,
    },
    Io {
        context: String,
        source: io::Error,
    },
    /// RES-4114: a `registry = "..."` dependency (or `pkg add`/`pkg
    /// update` with no source specifier) needs an index location, but
    /// none was given via `--index` and none is configured in
    /// `[registry]` in `resilient.toml`.
    RegistryNotConfigured,
    /// Wraps a `pkg_registry::PkgRegistryError` (malformed index,
    /// package/version not found, checksum mismatch, fetch/extract
    /// failure) with the offending index/package location for
    /// context.
    RegistryError {
        detail: String,
    },
}

impl fmt::Display for PkgDepsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ManifestNotFound { searched_from } => write!(
                f,
                "no resilient.toml found at or above {} \
                 — run `rz pkg init <name>` to scaffold one",
                searched_from.display()
            ),
            Self::ManifestUnreadable { path, source } => {
                write!(f, "could not read {}: {}", path.display(), source)
            }
            Self::MalformedDependency { name, detail } => {
                write!(f, "malformed dependency `{}`: {}", name, detail)
            }
            Self::PathDepNotFound { name, path } => write!(
                f,
                "path dependency `{}` not found at {}",
                name,
                path.display()
            ),
            Self::PathDepMissingManifest { name, path } => write!(
                f,
                "path dependency `{}` at {} has no resilient.toml",
                name,
                path.display()
            ),
            Self::PathDepMissingSrc { name, path } => write!(
                f,
                "path dependency `{}` at {} has no src/ directory",
                name,
                path.display()
            ),
            Self::GitCloneFailed { name, url, detail } => {
                write!(f, "git clone failed for `{}` ({}): {}", name, url, detail)
            }
            Self::GitCheckoutFailed { name, detail } => {
                write!(f, "git checkout failed for `{}`: {}", name, detail)
            }
            Self::LockfileIo { path, source } => {
                write!(f, "lockfile error at {}: {}", path.display(), source)
            }
            Self::ManifestWriteError { path, source } => {
                write!(f, "could not write manifest {}: {}", path.display(), source)
            }
            Self::DependencyNotFound { name } => write!(
                f,
                "no dependency named `{}` in [dependencies] — nothing to remove",
                name
            ),
            Self::Io { context, source } => write!(f, "{}: {}", context, source),
            Self::RegistryNotConfigured => write!(
                f,
                "no registry index configured — pass `--index <path-or-url>` or add \
                 `[registry]\\nindex = \"...\"` to resilient.toml"
            ),
            Self::RegistryError { detail } => write!(f, "registry error: {}", detail),
        }
    }
}

impl From<pkg_registry::PkgRegistryError> for PkgDepsError {
    fn from(e: pkg_registry::PkgRegistryError) -> Self {
        PkgDepsError::RegistryError {
            detail: e.to_string(),
        }
    }
}

impl std::error::Error for PkgDepsError {}

// ── TOML parsing of [dependencies] ───────────────────────────────

/// Parse the `[dependencies]` table from a `resilient.toml` file.
///
/// Uses the same hand-rolled TOML-ish approach as `pkg_init` and
/// `pkg_publish` — no external `toml` crate. Supports the inline
/// table syntax:
///
/// ```toml
/// [dependencies]
/// mylib = { path = "../libs/mylib" }
/// netutil = { git = "https://...", rev = "abc123" }
/// ```
pub fn parse_dependencies(manifest_path: &Path) -> Result<Vec<Dependency>, PkgDepsError> {
    let contents =
        fs::read_to_string(manifest_path).map_err(|e| PkgDepsError::ManifestUnreadable {
            path: manifest_path.to_path_buf(),
            source: e,
        })?;
    parse_dependencies_from_str(&contents)
}

/// Parse dependencies from a TOML string (testable without disk).
pub fn parse_dependencies_from_str(contents: &str) -> Result<Vec<Dependency>, PkgDepsError> {
    let mut deps = Vec::new();
    let mut in_deps = false;

    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Section header detection.
        if let Some(rest) = line.strip_prefix('[') {
            // Skip `[[` (array-of-tables) headers — those belong to
            // the lockfile format, not the manifest.
            if rest.starts_with('[') {
                in_deps = false;
                continue;
            }
            let header = rest.trim_end_matches(']').trim();
            in_deps = header == "dependencies";
            continue;
        }
        if !in_deps {
            continue;
        }
        // Parse `name = { ... }` lines.
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let name = key.trim().to_string();
        let val = val.trim();

        // Inline table: `{ key = "val", ... }`.
        if let Some(inner) = val.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            let fields = parse_inline_table(inner);
            let source = dep_source_from_fields(&name, &fields)?;
            deps.push(Dependency { name, source });
        } else if let Some(v) = extract_quoted_string(val) {
            // Simple string value — treat as path shorthand:
            // `mylib = "../libs/mylib"` equivalent to `{ path = "..." }`.
            deps.push(Dependency {
                name,
                source: DepSource::Path { path: v },
            });
        }
    }
    Ok(deps)
}

/// Parse key-value pairs from a TOML inline table body (the part
/// between `{` and `}`). Returns a map of string keys to string
/// values. Handles double-quoted values only.
fn parse_inline_table(inner: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for pair in inner.split(',') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let k = k.trim().to_string();
        if let Some(val) = extract_quoted_string(v.trim()) {
            map.insert(k, val);
        }
    }
    map
}

/// Extract the content of a double-quoted string: `"foo"` -> `Some("foo")`.
fn extract_quoted_string(s: &str) -> Option<String> {
    let s = s.trim();
    let rest = s.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Build a `DepSource` from parsed inline-table fields.
fn dep_source_from_fields(
    name: &str,
    fields: &BTreeMap<String, String>,
) -> Result<DepSource, PkgDepsError> {
    if let Some(path) = fields.get("path") {
        return Ok(DepSource::Path { path: path.clone() });
    }
    if let Some(url) = fields.get("git") {
        return Ok(DepSource::Git {
            url: url.clone(),
            rev: fields.get("rev").cloned(),
            tag: fields.get("tag").cloned(),
            branch: fields.get("branch").cloned(),
        });
    }
    if let Some(version) = fields.get("registry") {
        return Ok(DepSource::Registry {
            version: version.clone(),
        });
    }
    Err(PkgDepsError::MalformedDependency {
        name: name.to_string(),
        detail: "expected `path`, `git`, or `registry` key in dependency table".to_string(),
    })
}

// ── Dependency resolution ────────────────────────────────────────

/// Resolve all dependencies declared in the manifest at
/// `manifest_path`. Returns a list of resolved deps with their
/// on-disk source directories populated.
pub fn resolve_all(manifest_path: &Path) -> Result<Vec<ResolvedDep>, PkgDepsError> {
    let deps = parse_dependencies(manifest_path)?;
    let project_root = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let registry_index = read_registry_index(manifest_path);
    let mut resolved = Vec::with_capacity(deps.len());
    for dep in deps {
        resolved.push(resolve_one(project_root, &dep, registry_index.as_deref())?);
    }
    Ok(resolved)
}

/// Resolve a single dependency relative to `project_root`.
/// `registry_index` is the configured `[registry]` index location
/// (from `resilient.toml`), needed only for `DepSource::Registry`
/// deps.
fn resolve_one(
    project_root: &Path,
    dep: &Dependency,
    registry_index: Option<&str>,
) -> Result<ResolvedDep, PkgDepsError> {
    match &dep.source {
        DepSource::Path { path } => resolve_path_dep(project_root, &dep.name, path),
        DepSource::Git {
            url,
            rev,
            tag,
            branch,
        } => resolve_git_dep(
            &dep.name,
            url,
            rev.as_deref(),
            tag.as_deref(),
            branch.as_deref(),
        ),
        DepSource::Registry { version } => {
            let index = registry_index.ok_or(PkgDepsError::RegistryNotConfigured)?;
            let cache_root = default_registry_cache_dir()?;
            resolve_registry_dep(&cache_root, &dep.name, version, index)
        }
    }
}

/// Resolve a path-based dependency.
fn resolve_path_dep(
    project_root: &Path,
    name: &str,
    path: &str,
) -> Result<ResolvedDep, PkgDepsError> {
    let dep_root = project_root.join(path);
    if !dep_root.exists() {
        return Err(PkgDepsError::PathDepNotFound {
            name: name.to_string(),
            path: dep_root,
        });
    }
    let manifest = dep_root.join(pkg_init::MANIFEST_FILENAME);
    if !manifest.exists() {
        return Err(PkgDepsError::PathDepMissingManifest {
            name: name.to_string(),
            path: dep_root,
        });
    }
    let src_dir = dep_root.join("src");
    if !src_dir.is_dir() {
        return Err(PkgDepsError::PathDepMissingSrc {
            name: name.to_string(),
            path: dep_root,
        });
    }
    Ok(ResolvedDep {
        name: name.to_string(),
        source: DepSource::Path {
            path: path.to_string(),
        },
        root: dep_root,
        src_dir,
    })
}

/// Resolve a git-based dependency. Clones to `~/.resilient/cache/git/<hash>/`
/// if not already cached, then checks out the specified ref.
fn resolve_git_dep(
    name: &str,
    url: &str,
    rev: Option<&str>,
    tag: Option<&str>,
    branch: Option<&str>,
) -> Result<ResolvedDep, PkgDepsError> {
    let cache_dir = git_cache_dir(url)?;

    if !cache_dir.exists() {
        fs::create_dir_all(cache_dir.parent().unwrap_or_else(|| Path::new("."))).map_err(|e| {
            PkgDepsError::Io {
                context: format!("creating cache dir for `{}`", name),
                source: e,
            }
        })?;
        let output = Command::new("git")
            .args(["clone", url, &cache_dir.to_string_lossy()])
            .output()
            .map_err(|e| PkgDepsError::GitCloneFailed {
                name: name.to_string(),
                url: url.to_string(),
                detail: format!("failed to run git: {}", e),
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PkgDepsError::GitCloneFailed {
                name: name.to_string(),
                url: url.to_string(),
                detail: stderr.trim().to_string(),
            });
        }
    }

    // Checkout the specified ref (rev > tag > branch).
    let checkout_ref = rev.or(tag).or(branch);
    if let Some(r) = checkout_ref {
        let output = Command::new("git")
            .args(["checkout", r])
            .current_dir(&cache_dir)
            .output()
            .map_err(|e| PkgDepsError::GitCheckoutFailed {
                name: name.to_string(),
                detail: format!("failed to run git: {}", e),
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PkgDepsError::GitCheckoutFailed {
                name: name.to_string(),
                detail: stderr.trim().to_string(),
            });
        }
    }

    // Validate the cloned repo has a manifest and src/.
    let manifest = cache_dir.join(pkg_init::MANIFEST_FILENAME);
    if !manifest.exists() {
        return Err(PkgDepsError::PathDepMissingManifest {
            name: name.to_string(),
            path: cache_dir,
        });
    }
    let src_dir = cache_dir.join("src");
    if !src_dir.is_dir() {
        return Err(PkgDepsError::PathDepMissingSrc {
            name: name.to_string(),
            path: cache_dir,
        });
    }

    Ok(ResolvedDep {
        name: name.to_string(),
        source: DepSource::Git {
            url: url.to_string(),
            rev: rev.map(str::to_string),
            tag: tag.map(str::to_string),
            branch: branch.map(str::to_string),
        },
        root: cache_dir.clone(),
        src_dir,
    })
}

/// Compute the cache directory for a git URL. Uses a simple hash of
/// the URL to produce a deterministic, filesystem-safe directory name.
fn git_cache_dir(url: &str) -> Result<PathBuf, PkgDepsError> {
    let home = home_dir().ok_or_else(|| PkgDepsError::Io {
        context: "could not determine home directory".to_string(),
        source: io::Error::new(io::ErrorKind::NotFound, "HOME not set"),
    })?;
    let hash = simple_hash(url);
    Ok(home.join(".resilient").join("cache").join("git").join(hash))
}

/// Cross-platform `~` resolution matching pkg_publish's approach.
fn home_dir() -> Option<PathBuf> {
    if let Ok(h) = env::var("HOME")
        && !h.is_empty()
    {
        return Some(PathBuf::from(h));
    }
    if let Ok(h) = env::var("USERPROFILE")
        && !h.is_empty()
    {
        return Some(PathBuf::from(h));
    }
    None
}

/// Deterministic hash of a string. Not cryptographic — just for
/// cache directory naming. Produces a 16-char hex string.
fn simple_hash(s: &str) -> String {
    // FNV-1a 64-bit.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h)
}

// ── Registry-index dependency resolution (RES-4114 increment 2) ──

/// Default cache root for registry-resolved packages:
/// `~/.resilient/cache/registry/`. Callers that need testability
/// without touching `$HOME` should call [`resolve_registry_dep`]
/// directly with their own cache root instead.
pub fn default_registry_cache_dir() -> Result<PathBuf, PkgDepsError> {
    let home = home_dir().ok_or_else(|| PkgDepsError::Io {
        context: "could not determine home directory".to_string(),
        source: io::Error::new(io::ErrorKind::NotFound, "HOME not set"),
    })?;
    Ok(home.join(".resilient").join("cache").join("registry"))
}

/// Read the `[registry]` section's `index = "..."` value out of a
/// manifest, if present. Uses the same hand-rolled line-based TOML
/// approach as `parse_dependencies_from_str`.
pub fn read_registry_index(manifest_path: &Path) -> Option<String> {
    let contents = fs::read_to_string(manifest_path).ok()?;
    read_registry_index_from_str(&contents)
}

fn read_registry_index_from_str(contents: &str) -> Option<String> {
    let mut in_registry = false;
    for raw in contents.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix('[') {
            if rest.starts_with('[') {
                in_registry = false;
                continue;
            }
            let header = rest.trim_end_matches(']').trim();
            in_registry = header == "registry";
            continue;
        }
        if !in_registry {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "index"
            && let Some(v) = extract_quoted_string(val.trim())
        {
            return Some(v);
        }
    }
    None
}

/// Write (or update in place) the `[registry]\nindex = "..."` section
/// of a manifest so a `--index` passed to `pkg add`/`pkg update`
/// doesn't have to be repeated on every invocation.
fn set_registry_index(manifest_path: &Path, index: &str) -> Result<(), PkgDepsError> {
    let contents =
        fs::read_to_string(manifest_path).map_err(|e| PkgDepsError::ManifestUnreadable {
            path: manifest_path.to_path_buf(),
            source: e,
        })?;
    if read_registry_index_from_str(&contents).as_deref() == Some(index) {
        return Ok(()); // Already configured with this exact value.
    }

    let mut lines: Vec<String> = contents.lines().map(str::to_string).collect();
    let mut in_registry = false;
    let mut header_at: Option<usize> = None;
    let mut index_line_at: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('[') {
            if rest.starts_with('[') {
                in_registry = false;
                continue;
            }
            let header = rest.trim_end_matches(']').trim();
            in_registry = header == "registry";
            if in_registry {
                header_at = Some(i);
            }
            continue;
        }
        if in_registry
            && trimmed
                .split_once('=')
                .map(|(k, _)| k.trim() == "index")
                .unwrap_or(false)
        {
            index_line_at = Some(i);
        }
    }

    let new_line = format!("index = \"{}\"", index);
    if let Some(i) = index_line_at {
        lines[i] = new_line;
    } else if let Some(h) = header_at {
        lines.insert(h + 1, new_line);
    } else {
        if !contents.is_empty() && !contents.ends_with('\n') {
            lines.push(String::new());
        }
        lines.push(String::new());
        lines.push("[registry]".to_string());
        lines.push(new_line);
    }

    let mut joined = lines.join("\n");
    if !joined.ends_with('\n') {
        joined.push('\n');
    }
    fs::write(manifest_path, joined).map_err(|e| PkgDepsError::ManifestWriteError {
        path: manifest_path.to_path_buf(),
        source: e,
    })
}

/// Resolve a `registry` dependency: reuse the on-disk cache at
/// `cache_root/<name>/<version>/` if it already has a valid
/// manifest+src (no network I/O), otherwise fetch the index, resolve
/// `name@version` against it, fetch the package bytes, checksum-verify
/// them, and extract into the cache.
pub fn resolve_registry_dep(
    cache_root: &Path,
    name: &str,
    version: &str,
    index_location: &str,
) -> Result<ResolvedDep, PkgDepsError> {
    let dest = cache_root.join(name).join(version);
    let manifest = dest.join(pkg_init::MANIFEST_FILENAME);
    let src_dir = dest.join("src");
    if manifest.exists() && src_dir.is_dir() {
        return Ok(ResolvedDep {
            name: name.to_string(),
            source: DepSource::Registry {
                version: version.to_string(),
            },
            root: dest,
            src_dir,
        });
    }

    let index_bytes = pkg_registry::fetch_bytes(index_location)?;
    let index_json = String::from_utf8(index_bytes).map_err(|e| PkgDepsError::RegistryError {
        detail: format!(
            "registry index at `{}` is not valid UTF-8: {}",
            index_location, e
        ),
    })?;
    let index = pkg_registry::parse_index(&index_json)?;
    let (resolved_version, pv) = pkg_registry::resolve_package(&index, name, Some(version))?;
    let resolved_version = resolved_version.to_string();

    let contents = pkg_registry::fetch_bytes(&pv.source)?;
    pkg_registry::verify_checksum(name, &resolved_version, pv, &contents)?;
    pkg_registry::extract_ustar(&contents, &dest)?;

    if !manifest.exists() {
        return Err(PkgDepsError::PathDepMissingManifest {
            name: name.to_string(),
            path: dest,
        });
    }
    if !src_dir.is_dir() {
        return Err(PkgDepsError::PathDepMissingSrc {
            name: name.to_string(),
            path: dest,
        });
    }
    Ok(ResolvedDep {
        name: name.to_string(),
        source: DepSource::Registry {
            version: resolved_version,
        },
        root: dest,
        src_dir,
    })
}

/// `rz pkg add <name>` with no `path:`/`git:` spec — resolve a bare
/// package name against a registry index. `version` pins an exact
/// version; `None` resolves "latest" (lexicographically-greatest
/// version string, see `pkg_registry::resolve_package`).
/// `index_override` takes precedence over any `[registry]` section
/// already in `resilient.toml`, and is persisted there for future
/// `pkg add`/`pkg update` calls that don't repeat `--index`.
pub fn add_registry_dependency(
    name: &str,
    version: Option<&str>,
    index_override: Option<&str>,
) -> Result<(), PkgDepsError> {
    let cwd = env::current_dir().map_err(|e| PkgDepsError::Io {
        context: "reading current directory".to_string(),
        source: e,
    })?;
    let manifest_path =
        pkg_init::find_manifest_upwards(&cwd).ok_or_else(|| PkgDepsError::ManifestNotFound {
            searched_from: cwd.clone(),
        })?;
    let project_root = manifest_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or(cwd);

    let configured = read_registry_index(&manifest_path);
    let index_location = index_override
        .map(str::to_string)
        .or(configured)
        .ok_or(PkgDepsError::RegistryNotConfigured)?;

    let cache_root = default_registry_cache_dir()?;
    // Resolve against the index first (even on a cache hit for an
    // already-pinned exact version) so "latest" resolves to a
    // concrete version and unknown packages/versions fail fast.
    let index_bytes = pkg_registry::fetch_bytes(&index_location)?;
    let index_json = String::from_utf8(index_bytes).map_err(|e| PkgDepsError::RegistryError {
        detail: format!(
            "registry index at `{}` is not valid UTF-8: {}",
            index_location, e
        ),
    })?;
    let index = pkg_registry::parse_index(&index_json)?;
    let (resolved_version, _pv) = pkg_registry::resolve_package(&index, name, version)?;
    let resolved_version = resolved_version.to_string();

    let resolved = resolve_registry_dep(&cache_root, name, &resolved_version, &index_location)?;

    let dep = Dependency {
        name: name.to_string(),
        source: DepSource::Registry {
            version: resolved_version.clone(),
        },
    };
    append_dep_to_manifest(&manifest_path, &dep)?;
    if index_override.is_some() {
        set_registry_index(&manifest_path, &index_location)?;
    }

    let all = resolve_all(&manifest_path)?;
    write_lockfile(&project_root, &all)?;

    println!(
        "Added `{}@{}` from registry {} (resolved to {})",
        name,
        resolved_version,
        index_location,
        resolved.src_dir.display()
    );
    Ok(())
}

/// One reported version change from `rz pkg update`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReport {
    pub name: String,
    pub old_version: String,
    pub new_version: String,
}

/// `rz pkg update` — re-resolve every `registry`-sourced dependency
/// against the configured (or overridden) index to "latest", refetch
/// and checksum-verify anything whose version changed, rewrite the
/// manifest's version pins, and refresh the lockfile for every
/// dependency (path/git included, since their resolution can also
/// have moved, e.g. a git dependency tracking a branch).
pub fn update_dependencies(
    index_override: Option<&str>,
) -> Result<Vec<UpdateReport>, PkgDepsError> {
    let cwd = env::current_dir().map_err(|e| PkgDepsError::Io {
        context: "reading current directory".to_string(),
        source: e,
    })?;
    let manifest_path =
        pkg_init::find_manifest_upwards(&cwd).ok_or_else(|| PkgDepsError::ManifestNotFound {
            searched_from: cwd.clone(),
        })?;
    let project_root = manifest_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or(cwd);

    let deps = parse_dependencies(&manifest_path)?;
    let registry_deps: Vec<&Dependency> = deps
        .iter()
        .filter(|d| matches!(d.source, DepSource::Registry { .. }))
        .collect();

    let mut reports = Vec::new();

    if !registry_deps.is_empty() {
        let configured = read_registry_index(&manifest_path);
        let index_location = index_override
            .map(str::to_string)
            .or(configured)
            .ok_or(PkgDepsError::RegistryNotConfigured)?;

        let index_bytes = pkg_registry::fetch_bytes(&index_location)?;
        let index_json =
            String::from_utf8(index_bytes).map_err(|e| PkgDepsError::RegistryError {
                detail: format!(
                    "registry index at `{}` is not valid UTF-8: {}",
                    index_location, e
                ),
            })?;
        let index = pkg_registry::parse_index(&index_json)?;
        let cache_root = default_registry_cache_dir()?;

        for dep in &registry_deps {
            let DepSource::Registry { version: old } = &dep.source else {
                unreachable!("filtered to Registry sources above");
            };
            let (latest, _pv) = pkg_registry::resolve_package(&index, &dep.name, None)?;
            let latest = latest.to_string();
            if &latest != old {
                resolve_registry_dep(&cache_root, &dep.name, &latest, &index_location)?;
                update_manifest_dep_version(&manifest_path, &dep.name, &latest)?;
                reports.push(UpdateReport {
                    name: dep.name.clone(),
                    old_version: old.clone(),
                    new_version: latest,
                });
            }
        }

        if index_override.is_some() {
            set_registry_index(&manifest_path, &index_location)?;
        }
    }

    let all = resolve_all(&manifest_path)?;
    write_lockfile(&project_root, &all)?;
    Ok(reports)
}

/// Rewrite the single `<name> = { registry = "..." }` line in
/// `[dependencies]` to pin `new_version` instead.
fn update_manifest_dep_version(
    manifest_path: &Path,
    name: &str,
    new_version: &str,
) -> Result<(), PkgDepsError> {
    let contents =
        fs::read_to_string(manifest_path).map_err(|e| PkgDepsError::ManifestUnreadable {
            path: manifest_path.to_path_buf(),
            source: e,
        })?;
    let new_entry = format_dep_entry(&Dependency {
        name: name.to_string(),
        source: DepSource::Registry {
            version: new_version.to_string(),
        },
    });

    let mut in_deps = false;
    let mut lines: Vec<String> = contents.lines().map(str::to_string).collect();
    let mut replaced = false;
    for line in &mut lines {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('[') {
            in_deps = !rest.starts_with('[') && rest.trim_end_matches(']').trim() == "dependencies";
            continue;
        }
        if !in_deps {
            continue;
        }
        if let Some((key, _)) = trimmed.split_once('=')
            && key.trim() == name
        {
            *line = new_entry.clone();
            replaced = true;
            break;
        }
    }
    if !replaced {
        return Err(PkgDepsError::DependencyNotFound {
            name: name.to_string(),
        });
    }

    let mut joined = lines.join("\n");
    if contents.ends_with('\n') && !joined.ends_with('\n') {
        joined.push('\n');
    }
    fs::write(manifest_path, joined).map_err(|e| PkgDepsError::ManifestWriteError {
        path: manifest_path.to_path_buf(),
        source: e,
    })
}

// ── Lockfile ─────────────────────────────────────────────────────

/// Name of the lockfile.
pub const LOCKFILE_NAME: &str = "resilient.lock";

/// Write a lockfile from a set of resolved dependencies.
pub fn write_lockfile(project_root: &Path, deps: &[ResolvedDep]) -> Result<(), PkgDepsError> {
    let lock_path = project_root.join(LOCKFILE_NAME);
    let content = render_lockfile(deps);
    fs::write(&lock_path, content).map_err(|e| PkgDepsError::LockfileIo {
        path: lock_path,
        source: e,
    })
}

/// Render lockfile content from resolved deps.
pub fn render_lockfile(deps: &[ResolvedDep]) -> String {
    let mut out = String::new();
    for dep in deps {
        out.push_str("[[package]]\n");
        out.push_str(&format!("name = \"{}\"\n", dep.name));
        match &dep.source {
            DepSource::Path { path } => {
                out.push_str(&format!("source = \"path:{}\"\n", path));
            }
            DepSource::Git { url, rev, .. } => {
                out.push_str(&format!("source = \"git:{}\"\n", url));
                if let Some(r) = rev {
                    out.push_str(&format!("rev = \"{}\"\n", r));
                }
            }
            DepSource::Registry { version } => {
                out.push_str(&format!("source = \"registry:{}\"\n", dep.name));
                out.push_str(&format!("rev = \"{}\"\n", version));
            }
        }
        out.push('\n');
    }
    out
}

/// Parse a lockfile back into lock entries. Used by tests and future
/// `rz pkg update` / `rz pkg install` commands.
#[allow(dead_code)]
pub fn parse_lockfile(lock_path: &Path) -> Result<Vec<LockEntry>, PkgDepsError> {
    let contents = match fs::read_to_string(lock_path) {
        Ok(c) => c,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(PkgDepsError::LockfileIo {
                path: lock_path.to_path_buf(),
                source: e,
            });
        }
    };
    parse_lockfile_from_str(&contents)
}

/// Parse lockfile content from a string (testable without disk).
#[allow(dead_code)]
pub fn parse_lockfile_from_str(contents: &str) -> Result<Vec<LockEntry>, PkgDepsError> {
    let mut entries = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_source: Option<String> = None;
    let mut current_rev: Option<String> = None;

    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            // An empty line after fields signals the end of a package
            // entry (if one is open).
            if current_name.is_some() && current_source.is_some() {
                entries.push(LockEntry {
                    name: current_name.take().unwrap(),
                    source: current_source.take().unwrap(),
                    rev: current_rev.take(),
                });
            }
            continue;
        }
        if line == "[[package]]" {
            // Flush any pending entry before starting a new one.
            if current_name.is_some() && current_source.is_some() {
                entries.push(LockEntry {
                    name: current_name.take().unwrap(),
                    source: current_source.take().unwrap(),
                    rev: current_rev.take(),
                });
            }
            current_name = None;
            current_source = None;
            current_rev = None;
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if let Some(v) = extract_quoted_string(val) {
            match key {
                "name" => current_name = Some(v),
                "source" => current_source = Some(v),
                "rev" => current_rev = Some(v),
                _ => {}
            }
        }
    }
    // Flush trailing entry (no trailing blank line).
    if let (Some(name), Some(source)) = (current_name, current_source) {
        entries.push(LockEntry {
            name,
            source,
            rev: current_rev,
        });
    }
    Ok(entries)
}

// ── CLI: `rz pkg add` ───────────────────────────────────────────

/// Add a dependency to `resilient.toml` and write the lockfile.
///
/// `spec` is a source specifier: `path:../libs/mylib` or
/// `git:https://github.com/user/netutil`.
///
/// `opts` carries optional flags (`--rev`, `--tag`, `--branch`).
pub fn add_dependency(name: &str, spec: &str, opts: &AddOpts) -> Result<(), PkgDepsError> {
    let cwd = env::current_dir().map_err(|e| PkgDepsError::Io {
        context: "reading current directory".to_string(),
        source: e,
    })?;
    let manifest_path =
        pkg_init::find_manifest_upwards(&cwd).ok_or_else(|| PkgDepsError::ManifestNotFound {
            searched_from: cwd.clone(),
        })?;
    let project_root = manifest_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or(cwd);

    let source = parse_add_spec(name, spec, opts)?;
    let dep = Dependency {
        name: name.to_string(),
        source,
    };

    // Validate the dep resolves before writing.
    let registry_index = read_registry_index(&manifest_path);
    let resolved = resolve_one(&project_root, &dep, registry_index.as_deref())?;

    // Append to manifest.
    append_dep_to_manifest(&manifest_path, &dep)?;

    // Write lockfile with all deps (re-parse to include the one we
    // just added plus any existing ones).
    let all = resolve_all(&manifest_path)?;
    write_lockfile(&project_root, &all)?;

    println!(
        "Added `{}` to {} (resolved to {})",
        resolved.name,
        manifest_path.display(),
        resolved.src_dir.display()
    );
    Ok(())
}

/// Parsed CLI flags for `rz pkg add`.
#[derive(Debug, Default)]
pub struct AddOpts {
    pub rev: Option<String>,
    pub tag: Option<String>,
    pub branch: Option<String>,
}

/// Parse a `path:...` or `git:...` specifier into a `DepSource`.
fn parse_add_spec(name: &str, spec: &str, opts: &AddOpts) -> Result<DepSource, PkgDepsError> {
    if let Some(path) = spec.strip_prefix("path:") {
        return Ok(DepSource::Path {
            path: path.to_string(),
        });
    }
    if let Some(url) = spec.strip_prefix("git:") {
        return Ok(DepSource::Git {
            url: url.to_string(),
            rev: opts.rev.clone(),
            tag: opts.tag.clone(),
            branch: opts.branch.clone(),
        });
    }
    Err(PkgDepsError::MalformedDependency {
        name: name.to_string(),
        detail: format!(
            "source specifier must start with `path:` or `git:`, got `{}`",
            spec
        ),
    })
}

/// Append a dependency entry to the `[dependencies]` table in the
/// manifest. If `[dependencies]` doesn't exist, appends the section
/// header first.
fn append_dep_to_manifest(manifest_path: &Path, dep: &Dependency) -> Result<(), PkgDepsError> {
    let contents =
        fs::read_to_string(manifest_path).map_err(|e| PkgDepsError::ManifestUnreadable {
            path: manifest_path.to_path_buf(),
            source: e,
        })?;

    let entry_line = format_dep_entry(dep);

    // Check if [dependencies] section exists.
    let has_deps_section = contents.lines().any(|l| l.trim() == "[dependencies]");

    let new_contents = if has_deps_section {
        // Find the [dependencies] line and append after any existing
        // entries in that section (before the next section or EOF).
        let mut lines: Vec<&str> = contents.lines().collect();
        let mut insert_at = lines.len();
        let mut found_deps = false;
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed == "[dependencies]" {
                found_deps = true;
                continue;
            }
            if found_deps {
                // If we hit a new section header, insert before it.
                if trimmed.starts_with('[') && !trimmed.starts_with("[[") {
                    insert_at = i;
                    break;
                }
                // Track the last non-empty line in the section.
                if !trimmed.is_empty() {
                    insert_at = i + 1;
                }
            }
        }
        lines.insert(insert_at, &entry_line);
        let mut joined = lines.join("\n");
        // Preserve trailing newline if original had one.
        if contents.ends_with('\n') && !joined.ends_with('\n') {
            joined.push('\n');
        }
        joined
    } else {
        let mut out = contents.clone();
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("\n[dependencies]\n");
        out.push_str(&entry_line);
        out.push('\n');
        out
    };

    fs::write(manifest_path, new_contents).map_err(|e| PkgDepsError::ManifestWriteError {
        path: manifest_path.to_path_buf(),
        source: e,
    })
}

/// Format a dependency as a TOML inline table line.
fn format_dep_entry(dep: &Dependency) -> String {
    match &dep.source {
        DepSource::Path { path } => {
            format!("{} = {{ path = \"{}\" }}", dep.name, path)
        }
        DepSource::Git {
            url,
            rev,
            tag,
            branch,
        } => {
            let mut s = format!("{} = {{ git = \"{}\"", dep.name, url);
            if let Some(r) = rev {
                s.push_str(&format!(", rev = \"{}\"", r));
            }
            if let Some(t) = tag {
                s.push_str(&format!(", tag = \"{}\"", t));
            }
            if let Some(b) = branch {
                s.push_str(&format!(", branch = \"{}\"", b));
            }
            s.push_str(" }");
            s
        }
        DepSource::Registry { version } => {
            format!("{} = {{ registry = \"{}\" }}", dep.name, version)
        }
    }
}

// ── CLI: `rz pkg remove` ─────────────────────────────────────────

/// Remove a dependency from `resilient.toml` and rewrite the lockfile
/// to match. Errors if the dependency isn't declared.
pub fn remove_dependency(name: &str) -> Result<(), PkgDepsError> {
    let cwd = env::current_dir().map_err(|e| PkgDepsError::Io {
        context: "reading current directory".to_string(),
        source: e,
    })?;
    let manifest_path =
        pkg_init::find_manifest_upwards(&cwd).ok_or_else(|| PkgDepsError::ManifestNotFound {
            searched_from: cwd.clone(),
        })?;
    let project_root = manifest_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or(cwd);

    remove_dep_from_manifest(&manifest_path, name)?;

    // Re-resolve the remaining deps and rewrite the lockfile so it
    // never drifts from the manifest.
    let all = resolve_all(&manifest_path)?;
    write_lockfile(&project_root, &all)?;

    println!(
        "Removed `{}` from {} ({} dependency/ies remain)",
        name,
        manifest_path.display(),
        all.len()
    );
    Ok(())
}

/// Drop the `name = { ... }` (or shorthand string) line for `name`
/// from the `[dependencies]` section of the manifest at
/// `manifest_path`. Errors with `DependencyNotFound` if no such
/// entry exists — mirrors `parse_dependencies_from_str`'s section
/// detection so the two stay in lockstep.
fn remove_dep_from_manifest(manifest_path: &Path, name: &str) -> Result<(), PkgDepsError> {
    let contents =
        fs::read_to_string(manifest_path).map_err(|e| PkgDepsError::ManifestUnreadable {
            path: manifest_path.to_path_buf(),
            source: e,
        })?;

    let deps = parse_dependencies_from_str(&contents)?;
    if !deps.iter().any(|d| d.name == name) {
        return Err(PkgDepsError::DependencyNotFound {
            name: name.to_string(),
        });
    }

    let mut in_deps = false;
    let mut out_lines: Vec<&str> = Vec::new();
    for raw in contents.lines() {
        let trimmed = raw.trim();
        if let Some(rest) = trimmed.strip_prefix('[') {
            in_deps = if rest.starts_with('[') {
                false
            } else {
                rest.trim_end_matches(']').trim() == "dependencies"
            };
            out_lines.push(raw);
            continue;
        }
        if in_deps
            && let Some((key, _)) = trimmed.split_once('=')
            && key.trim() == name
        {
            // Drop this line — it's the entry being removed.
            continue;
        }
        out_lines.push(raw);
    }

    let mut joined = out_lines.join("\n");
    if contents.ends_with('\n') && !joined.ends_with('\n') {
        joined.push('\n');
    }

    fs::write(manifest_path, joined).map_err(|e| PkgDepsError::ManifestWriteError {
        path: manifest_path.to_path_buf(),
        source: e,
    })
}

// ── CLI: `rz pkg search` ─────────────────────────────────────────

/// A local dependency-search result. There is no remote registry
/// index yet (tracked as E-E2 follow-up work), so search covers only
/// what's already resolvable from the manifest and lockfile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    pub name: String,
    /// `"path:../libs/mylib"` or `"git:https://..."`.
    pub source: String,
    /// Whether this dependency also has a `resilient.lock` entry.
    pub locked: bool,
}

/// Search the nearest `resilient.toml` (and its `resilient.lock`, if
/// present) for dependencies whose name contains `query`
/// (case-insensitive substring match).
pub fn search_dependencies(query: &str) -> Result<Vec<SearchMatch>, PkgDepsError> {
    let cwd = env::current_dir().map_err(|e| PkgDepsError::Io {
        context: "reading current directory".to_string(),
        source: e,
    })?;
    let manifest_path =
        pkg_init::find_manifest_upwards(&cwd).ok_or_else(|| PkgDepsError::ManifestNotFound {
            searched_from: cwd.clone(),
        })?;
    search_dependencies_in(&manifest_path, query)
}

/// Core, disk-path-driven implementation of `search_dependencies` —
/// testable without relying on the process's current directory.
fn search_dependencies_in(
    manifest_path: &Path,
    query: &str,
) -> Result<Vec<SearchMatch>, PkgDepsError> {
    let deps = parse_dependencies(manifest_path)?;
    let project_root = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let lock_path = project_root.join(LOCKFILE_NAME);
    let locked = parse_lockfile(&lock_path)?
        .into_iter()
        .map(|entry| entry.name)
        .collect::<std::collections::BTreeSet<_>>();

    let query_lower = query.to_lowercase();
    let mut matches: Vec<SearchMatch> = deps
        .iter()
        .filter(|d| d.name.to_lowercase().contains(&query_lower))
        .map(|d| SearchMatch {
            name: d.name.clone(),
            source: dep_source_display(&d.source),
            locked: locked.contains(&d.name),
        })
        .collect();
    matches.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(matches)
}

/// Render a `DepSource` the same way the lockfile does
/// (`"path:..."` / `"git:..."`), for display in search results.
fn dep_source_display(source: &DepSource) -> String {
    match source {
        DepSource::Path { path } => format!("path:{}", path),
        DepSource::Git { url, rev, .. } => match rev {
            Some(r) => format!("git:{} (rev {})", url, r),
            None => format!("git:{}", url),
        },
        DepSource::Registry { version } => format!("registry:{}", version),
    }
}

// ── Import integration ───────────────────────────────────────────

/// Look up a dependency by name in the nearest `resilient.toml` and
/// return the path to a specific module file within it.
///
/// Given `use mylib::foo;`, `dep_name` is `"mylib"` and `module` is
/// `"foo"`. Returns the path to `<dep_root>/src/foo.rz` if it exists.
///
/// RES-4110: `module` may itself be a multi-segment path — e.g. given
/// `use mylib::sub::leaf;`, `module` is `"sub::leaf"`. Each `::`-
/// separated segment maps to a nested directory component, so this
/// resolves to `<dep_root>/src/sub/leaf.rz` rather than a literal
/// `sub::leaf.rz` file (which could never exist). A single-segment
/// module is unaffected: `"foo"` still resolves to `src/foo.rz`.
///
/// Called from `imports.rs` as a fallback when a `use X::Y;` is
/// neither a stdlib import nor a local file.
pub fn resolve_dep_module(
    start_dir: &Path,
    dep_name: &str,
    module: &str,
) -> Result<Option<PathBuf>, String> {
    let manifest_path = match pkg_init::find_manifest_upwards(start_dir) {
        Some(p) => p,
        None => return Ok(None),
    };
    let deps = parse_dependencies(&manifest_path).map_err(|e| e.to_string())?;
    let dep = match deps.iter().find(|d| d.name == dep_name) {
        Some(d) => d,
        None => return Ok(None),
    };
    let project_root = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let registry_index = read_registry_index(&manifest_path);
    let resolved =
        resolve_one(project_root, dep, registry_index.as_deref()).map_err(|e| e.to_string())?;
    let mut module_file = resolved.src_dir.clone();
    let segments: Vec<&str> = module.split("::").collect();
    let (last, dirs) = segments.split_last().expect("split on non-empty str");
    for dir in dirs {
        module_file.push(dir);
    }
    module_file.push(format!("{}.rz", last));
    if module_file.exists() {
        Ok(Some(module_file))
    } else {
        Ok(None)
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn tmp_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let p =
            std::env::temp_dir().join(format!("res_pkg_deps_{}_{}_{}", tag, std::process::id(), n));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).expect("mkdir tmp");
        p
    }

    // ── TOML parsing tests ───────────────────────────────────────

    #[test]
    fn parse_path_dep() {
        let toml = "\
[package]
name = \"myproject\"
version = \"0.1.0\"

[dependencies]
mylib = { path = \"../libs/mylib\" }
";
        let deps = parse_dependencies_from_str(toml).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "mylib");
        assert_eq!(
            deps[0].source,
            DepSource::Path {
                path: "../libs/mylib".to_string()
            }
        );
    }

    #[test]
    fn parse_git_dep() {
        let toml = "\
[package]
name = \"myproject\"

[dependencies]
netutil = { git = \"https://github.com/user/netutil\", rev = \"abc123\" }
";
        let deps = parse_dependencies_from_str(toml).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "netutil");
        assert_eq!(
            deps[0].source,
            DepSource::Git {
                url: "https://github.com/user/netutil".to_string(),
                rev: Some("abc123".to_string()),
                tag: None,
                branch: None,
            }
        );
    }

    #[test]
    fn parse_multiple_deps() {
        let toml = "\
[dependencies]
a = { path = \"./a\" }
b = { git = \"https://ex.com/b\", tag = \"v1\" }
c = { path = \"./c\" }
";
        let deps = parse_dependencies_from_str(toml).unwrap();
        assert_eq!(deps.len(), 3);
        assert_eq!(deps[0].name, "a");
        assert_eq!(deps[1].name, "b");
        assert_eq!(deps[2].name, "c");
        assert_eq!(
            deps[1].source,
            DepSource::Git {
                url: "https://ex.com/b".to_string(),
                rev: None,
                tag: Some("v1".to_string()),
                branch: None,
            }
        );
    }

    #[test]
    fn parse_shorthand_string_as_path() {
        let toml = "\
[dependencies]
x = \"../x\"
";
        let deps = parse_dependencies_from_str(toml).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(
            deps[0].source,
            DepSource::Path {
                path: "../x".to_string()
            }
        );
    }

    #[test]
    fn parse_empty_deps_section() {
        let toml = "\
[package]
name = \"empty\"

[dependencies]
";
        let deps = parse_dependencies_from_str(toml).unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn parse_no_deps_section() {
        let toml = "\
[package]
name = \"bare\"
version = \"0.1.0\"
";
        let deps = parse_dependencies_from_str(toml).unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn parse_dep_missing_source_key_is_error() {
        let toml = "\
[dependencies]
bad = { version = \"1.0\" }
";
        let err = parse_dependencies_from_str(toml).unwrap_err();
        assert!(matches!(err, PkgDepsError::MalformedDependency { .. }));
    }

    #[test]
    fn parse_git_dep_with_branch() {
        let toml = "\
[dependencies]
lib = { git = \"https://ex.com/lib\", branch = \"develop\" }
";
        let deps = parse_dependencies_from_str(toml).unwrap();
        assert_eq!(
            deps[0].source,
            DepSource::Git {
                url: "https://ex.com/lib".to_string(),
                rev: None,
                tag: None,
                branch: Some("develop".to_string()),
            }
        );
    }

    // ── Path dep resolution tests ────────────────────────────────

    #[test]
    fn resolve_path_dep_succeeds() {
        let project = tmp_dir("resolve_path");
        let dep_dir = project.join("libs").join("mylib");
        fs::create_dir_all(dep_dir.join("src")).unwrap();
        fs::write(
            dep_dir.join("resilient.toml"),
            "[package]\nname = \"mylib\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let result = resolve_path_dep(&project, "mylib", "libs/mylib");
        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert_eq!(resolved.name, "mylib");
        assert!(resolved.src_dir.ends_with("src"));
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn resolve_path_dep_missing_dir_is_error() {
        let project = tmp_dir("resolve_path_missing");
        let result = resolve_path_dep(&project, "nope", "nonexistent");
        assert!(matches!(result, Err(PkgDepsError::PathDepNotFound { .. })));
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn resolve_path_dep_missing_manifest_is_error() {
        let project = tmp_dir("resolve_path_nomanifest");
        let dep_dir = project.join("mylib");
        fs::create_dir_all(dep_dir.join("src")).unwrap();
        let result = resolve_path_dep(&project, "mylib", "mylib");
        assert!(matches!(
            result,
            Err(PkgDepsError::PathDepMissingManifest { .. })
        ));
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn resolve_path_dep_missing_src_is_error() {
        let project = tmp_dir("resolve_path_nosrc");
        let dep_dir = project.join("mylib");
        fs::create_dir_all(&dep_dir).unwrap();
        fs::write(
            dep_dir.join("resilient.toml"),
            "[package]\nname = \"mylib\"\n",
        )
        .unwrap();
        let result = resolve_path_dep(&project, "mylib", "mylib");
        assert!(matches!(
            result,
            Err(PkgDepsError::PathDepMissingSrc { .. })
        ));
        let _ = fs::remove_dir_all(&project);
    }

    // ── Lockfile tests ───────────────────────────────────────────

    #[test]
    fn lockfile_round_trip() {
        let deps = vec![
            ResolvedDep {
                name: "mylib".to_string(),
                source: DepSource::Path {
                    path: "../libs/mylib".to_string(),
                },
                root: PathBuf::from("/tmp/mylib"),
                src_dir: PathBuf::from("/tmp/mylib/src"),
            },
            ResolvedDep {
                name: "netutil".to_string(),
                source: DepSource::Git {
                    url: "https://github.com/user/netutil".to_string(),
                    rev: Some("abc123".to_string()),
                    tag: None,
                    branch: None,
                },
                root: PathBuf::from("/tmp/netutil"),
                src_dir: PathBuf::from("/tmp/netutil/src"),
            },
        ];
        let rendered = render_lockfile(&deps);
        let entries = parse_lockfile_from_str(&rendered).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "mylib");
        assert_eq!(entries[0].source, "path:../libs/mylib");
        assert!(entries[0].rev.is_none());
        assert_eq!(entries[1].name, "netutil");
        assert_eq!(entries[1].source, "git:https://github.com/user/netutil");
        assert_eq!(entries[1].rev.as_deref(), Some("abc123"));
    }

    #[test]
    fn lockfile_write_and_read_on_disk() {
        let project = tmp_dir("lockfile_disk");
        let deps = vec![ResolvedDep {
            name: "a".to_string(),
            source: DepSource::Path {
                path: "./a".to_string(),
            },
            root: PathBuf::from("/tmp/a"),
            src_dir: PathBuf::from("/tmp/a/src"),
        }];
        write_lockfile(&project, &deps).unwrap();
        let lock_path = project.join(LOCKFILE_NAME);
        assert!(lock_path.exists());
        let entries = parse_lockfile(&lock_path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "a");
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn lockfile_missing_file_returns_empty() {
        let entries = parse_lockfile(Path::new("/definitely/not/here/resilient.lock")).unwrap();
        assert!(entries.is_empty());
    }

    // ── Manifest append tests ────────────────────────────────────

    #[test]
    fn append_dep_to_existing_deps_section() {
        let project = tmp_dir("append_dep");
        let manifest = project.join("resilient.toml");
        fs::write(
            &manifest,
            "[package]\nname = \"proj\"\nversion = \"0.1.0\"\n\n[dependencies]\n",
        )
        .unwrap();
        let dep = Dependency {
            name: "mylib".to_string(),
            source: DepSource::Path {
                path: "../mylib".to_string(),
            },
        };
        append_dep_to_manifest(&manifest, &dep).unwrap();
        let content = fs::read_to_string(&manifest).unwrap();
        assert!(
            content.contains("mylib = { path = \"../mylib\" }"),
            "got: {}",
            content
        );
        assert!(content.contains("[package]"));
        assert!(content.contains("name = \"proj\""));
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn append_dep_creates_deps_section_when_missing() {
        let project = tmp_dir("append_dep_nosec");
        let manifest = project.join("resilient.toml");
        fs::write(&manifest, "[package]\nname = \"proj\"\n").unwrap();
        let dep = Dependency {
            name: "foo".to_string(),
            source: DepSource::Git {
                url: "https://ex.com/foo".to_string(),
                rev: Some("deadbeef".to_string()),
                tag: None,
                branch: None,
            },
        };
        append_dep_to_manifest(&manifest, &dep).unwrap();
        let content = fs::read_to_string(&manifest).unwrap();
        assert!(content.contains("[dependencies]"), "got: {}", content);
        assert!(
            content.contains("foo = { git = \"https://ex.com/foo\", rev = \"deadbeef\" }"),
            "got: {}",
            content
        );
        let _ = fs::remove_dir_all(&project);
    }

    // ── Format dep entry tests ───────────────────────────────────

    #[test]
    fn format_path_dep_entry() {
        let dep = Dependency {
            name: "mylib".to_string(),
            source: DepSource::Path {
                path: "../mylib".to_string(),
            },
        };
        assert_eq!(format_dep_entry(&dep), "mylib = { path = \"../mylib\" }");
    }

    #[test]
    fn format_git_dep_entry_with_rev() {
        let dep = Dependency {
            name: "net".to_string(),
            source: DepSource::Git {
                url: "https://ex.com/net".to_string(),
                rev: Some("abc".to_string()),
                tag: None,
                branch: None,
            },
        };
        assert_eq!(
            format_dep_entry(&dep),
            "net = { git = \"https://ex.com/net\", rev = \"abc\" }"
        );
    }

    #[test]
    fn format_git_dep_entry_with_tag_and_branch() {
        let dep = Dependency {
            name: "x".to_string(),
            source: DepSource::Git {
                url: "https://ex.com/x".to_string(),
                rev: None,
                tag: Some("v1".to_string()),
                branch: Some("main".to_string()),
            },
        };
        let s = format_dep_entry(&dep);
        assert!(s.contains("tag = \"v1\""), "got: {}", s);
        assert!(s.contains("branch = \"main\""), "got: {}", s);
    }

    // ── parse_add_spec tests ─────────────────────────────────────

    #[test]
    fn parse_add_spec_path() {
        let opts = AddOpts::default();
        let source = parse_add_spec("mylib", "path:../mylib", &opts).unwrap();
        assert_eq!(
            source,
            DepSource::Path {
                path: "../mylib".to_string()
            }
        );
    }

    #[test]
    fn parse_add_spec_git() {
        let opts = AddOpts {
            rev: Some("abc".to_string()),
            ..Default::default()
        };
        let source = parse_add_spec("net", "git:https://ex.com/net", &opts).unwrap();
        assert_eq!(
            source,
            DepSource::Git {
                url: "https://ex.com/net".to_string(),
                rev: Some("abc".to_string()),
                tag: None,
                branch: None,
            }
        );
    }

    #[test]
    fn parse_add_spec_invalid() {
        let opts = AddOpts::default();
        let err = parse_add_spec("x", "ftp:something", &opts).unwrap_err();
        assert!(matches!(err, PkgDepsError::MalformedDependency { .. }));
    }

    // ── Hash determinism tests ───────────────────────────────────

    #[test]
    fn simple_hash_is_deterministic() {
        let a = simple_hash("https://github.com/user/repo");
        let b = simple_hash("https://github.com/user/repo");
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn simple_hash_differs_for_different_inputs() {
        let a = simple_hash("https://github.com/user/repo1");
        let b = simple_hash("https://github.com/user/repo2");
        assert_ne!(a, b);
    }

    // ── dep_module resolution tests ──────────────────────────────

    #[test]
    fn resolve_dep_module_finds_file() {
        let project = tmp_dir("dep_module");
        fs::write(
            project.join("resilient.toml"),
            "[package]\nname = \"proj\"\n\n[dependencies]\nmylib = { path = \"mylib\" }\n",
        )
        .unwrap();
        let dep_dir = project.join("mylib");
        fs::create_dir_all(dep_dir.join("src")).unwrap();
        fs::write(
            dep_dir.join("resilient.toml"),
            "[package]\nname = \"mylib\"\n",
        )
        .unwrap();
        fs::write(dep_dir.join("src/foo.rz"), "pub fn hello() { return 1; }").unwrap();

        let result = resolve_dep_module(&project, "mylib", "foo").unwrap();
        assert!(result.is_some());
        let path = result.unwrap();
        assert!(path.ends_with("foo.rz"), "got: {}", path.display());
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn resolve_dep_module_finds_nested_submodule_path() {
        // RES-4110: `use mylib::sub::leaf;` must resolve to
        // `<dep_root>/src/sub/leaf.rz`, not a literal `sub::leaf.rz`.
        let project = tmp_dir("dep_module_nested");
        fs::write(
            project.join("resilient.toml"),
            "[package]\nname = \"proj\"\n\n[dependencies]\nmylib = { path = \"mylib\" }\n",
        )
        .unwrap();
        let dep_dir = project.join("mylib");
        fs::create_dir_all(dep_dir.join("src/sub")).unwrap();
        fs::write(
            dep_dir.join("resilient.toml"),
            "[package]\nname = \"mylib\"\n",
        )
        .unwrap();
        fs::write(
            dep_dir.join("src/sub/leaf.rz"),
            "pub fn hello() { return 1; }",
        )
        .unwrap();

        let result = resolve_dep_module(&project, "mylib", "sub::leaf").unwrap();
        assert!(result.is_some(), "expected nested module to resolve");
        let path = result.unwrap();
        assert!(
            path.ends_with("sub/leaf.rz") || path.ends_with("sub\\leaf.rz"),
            "got: {}",
            path.display()
        );
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn resolve_dep_module_nested_missing_returns_none() {
        let project = tmp_dir("dep_module_nested_missing");
        fs::write(
            project.join("resilient.toml"),
            "[package]\nname = \"proj\"\n\n[dependencies]\nmylib = { path = \"mylib\" }\n",
        )
        .unwrap();
        let dep_dir = project.join("mylib");
        fs::create_dir_all(dep_dir.join("src")).unwrap();
        fs::write(
            dep_dir.join("resilient.toml"),
            "[package]\nname = \"mylib\"\n",
        )
        .unwrap();

        let result = resolve_dep_module(&project, "mylib", "sub::missing").unwrap();
        assert!(result.is_none());
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn resolve_dep_module_returns_none_for_unknown_dep() {
        let project = tmp_dir("dep_module_unknown");
        fs::write(
            project.join("resilient.toml"),
            "[package]\nname = \"proj\"\n\n[dependencies]\n",
        )
        .unwrap();
        let result = resolve_dep_module(&project, "nope", "foo").unwrap();
        assert!(result.is_none());
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn resolve_dep_module_returns_none_when_no_manifest() {
        let project = tmp_dir("dep_module_nomanifest");
        let result = resolve_dep_module(&project, "x", "y").unwrap();
        assert!(result.is_none());
        let _ = fs::remove_dir_all(&project);
    }

    // ── resolve_all integration test ─────────────────────────────

    #[test]
    fn resolve_all_path_deps() {
        let project = tmp_dir("resolve_all");
        for name in &["libA", "libB"] {
            let d = project.join(name);
            fs::create_dir_all(d.join("src")).unwrap();
            fs::write(
                d.join("resilient.toml"),
                format!("[package]\nname = \"{}\"\n", name),
            )
            .unwrap();
        }
        let manifest = project.join("resilient.toml");
        fs::write(
            &manifest,
            "[package]\nname = \"proj\"\n\n[dependencies]\n\
             libA = { path = \"libA\" }\nlibB = { path = \"libB\" }\n",
        )
        .unwrap();
        let resolved = resolve_all(&manifest).unwrap();
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].name, "libA");
        assert_eq!(resolved[1].name, "libB");
        let _ = fs::remove_dir_all(&project);
    }

    // ── remove_dep_from_manifest tests ───────────────────────────

    #[test]
    fn remove_dep_drops_entry_and_keeps_others() {
        let project = tmp_dir("remove_dep");
        let manifest = project.join("resilient.toml");
        fs::write(
            &manifest,
            "[package]\nname = \"proj\"\nversion = \"0.1.0\"\n\n[dependencies]\n\
             mylib = { path = \"../mylib\" }\n\
             netutil = { git = \"https://ex.com/netutil\", rev = \"abc123\" }\n",
        )
        .unwrap();

        remove_dep_from_manifest(&manifest, "mylib").unwrap();

        let content = fs::read_to_string(&manifest).unwrap();
        assert!(
            !content.contains("mylib"),
            "mylib should be removed: {}",
            content
        );
        assert!(
            content.contains("netutil = { git = \"https://ex.com/netutil\", rev = \"abc123\" }"),
            "netutil should remain: {}",
            content
        );
        assert!(content.contains("[package]"));
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn remove_dep_errors_when_absent() {
        let project = tmp_dir("remove_dep_absent");
        let manifest = project.join("resilient.toml");
        fs::write(
            &manifest,
            "[package]\nname = \"proj\"\n\n[dependencies]\nmylib = { path = \"../mylib\" }\n",
        )
        .unwrap();

        let err = remove_dep_from_manifest(&manifest, "nope").unwrap_err();
        assert!(matches!(err, PkgDepsError::DependencyNotFound { name } if name == "nope"));

        // Manifest is untouched on error.
        let content = fs::read_to_string(&manifest).unwrap();
        assert!(content.contains("mylib = { path = \"../mylib\" }"));
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn remove_dependency_end_to_end_updates_lockfile() {
        let project = tmp_dir("remove_dependency_e2e");
        for name in &["keep", "drop"] {
            let d = project.join(name);
            fs::create_dir_all(d.join("src")).unwrap();
            fs::write(
                d.join("resilient.toml"),
                format!("[package]\nname = \"{}\"\n", name),
            )
            .unwrap();
        }
        let manifest = project.join("resilient.toml");
        fs::write(
            &manifest,
            "[package]\nname = \"proj\"\n\n[dependencies]\n\
             keep = { path = \"keep\" }\ndrop = { path = \"drop\" }\n",
        )
        .unwrap();

        // Simulate the CWD-driven CLI path via the pure core function,
        // then re-resolve + write the lockfile exactly as
        // `remove_dependency` does, since that function depends on
        // `env::current_dir()` and isn't directly unit-testable.
        remove_dep_from_manifest(&manifest, "drop").unwrap();
        let remaining = resolve_all(&manifest).unwrap();
        write_lockfile(&project, &remaining).unwrap();

        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].name, "keep");

        let lock_content = fs::read_to_string(project.join(LOCKFILE_NAME)).unwrap();
        assert!(lock_content.contains("name = \"keep\""));
        assert!(!lock_content.contains("name = \"drop\""));
        let _ = fs::remove_dir_all(&project);
    }

    // ── search_dependencies_in tests ─────────────────────────────

    #[test]
    fn search_finds_substring_match_case_insensitive() {
        let project = tmp_dir("search_basic");
        let manifest = project.join("resilient.toml");
        fs::write(
            &manifest,
            "[package]\nname = \"proj\"\n\n[dependencies]\n\
             mylib = { path = \"../mylib\" }\n\
             netutil = { git = \"https://ex.com/netutil\", rev = \"abc123\" }\n",
        )
        .unwrap();

        let matches = search_dependencies_in(&manifest, "LIB").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "mylib");
        assert_eq!(matches[0].source, "path:../mylib");
        assert!(!matches[0].locked);
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn search_marks_locked_dependencies() {
        let project = tmp_dir("search_locked");
        let manifest = project.join("resilient.toml");
        fs::write(
            &manifest,
            "[package]\nname = \"proj\"\n\n[dependencies]\n\
             netutil = { git = \"https://ex.com/netutil\", rev = \"abc123\" }\n",
        )
        .unwrap();
        fs::write(
            project.join(LOCKFILE_NAME),
            "[[package]]\nname = \"netutil\"\nsource = \"git:https://ex.com/netutil\"\nrev = \"abc123\"\n",
        )
        .unwrap();

        let matches = search_dependencies_in(&manifest, "net").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "netutil");
        assert_eq!(matches[0].source, "git:https://ex.com/netutil (rev abc123)");
        assert!(matches[0].locked);
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn search_returns_empty_for_no_matches() {
        let project = tmp_dir("search_empty");
        let manifest = project.join("resilient.toml");
        fs::write(
            &manifest,
            "[package]\nname = \"proj\"\n\n[dependencies]\nmylib = { path = \"../mylib\" }\n",
        )
        .unwrap();

        let matches = search_dependencies_in(&manifest, "nonexistent").unwrap();
        assert!(matches.is_empty());
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn search_results_are_sorted_by_name() {
        let project = tmp_dir("search_sorted");
        let manifest = project.join("resilient.toml");
        fs::write(
            &manifest,
            "[package]\nname = \"proj\"\n\n[dependencies]\n\
             zeta = { path = \"zeta\" }\nalpha = { path = \"alpha\" }\n",
        )
        .unwrap();

        let matches = search_dependencies_in(&manifest, "a").unwrap();
        let names: Vec<&str> = matches.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "zeta"]);
        let _ = fs::remove_dir_all(&project);
    }

    // ── Registry dependency tests (RES-4114 increment 2) ─────────

    fn make_registry_package_tarball(name: &str, version: &str, body: &str) -> Vec<u8> {
        use crate::pkg_publish::{PublishManifest, make_tarball};
        let src_dir = tmp_dir(&format!("registry_pkg_src_{}_{}", name, version));
        fs::create_dir_all(src_dir.join("src")).unwrap();
        fs::write(
            src_dir.join("resilient.toml"),
            format!(
                "[package]\nname = \"{}\"\nversion = \"{}\"\n",
                name, version
            ),
        )
        .unwrap();
        fs::write(src_dir.join("src/main.rz"), body).unwrap();
        let manifest = PublishManifest {
            name: name.to_string(),
            version: version.to_string(),
            description: None,
            entry: PathBuf::from("src/main.rz"),
        };
        let files = vec![
            PathBuf::from("resilient.toml"),
            PathBuf::from("src/main.rz"),
        ];
        let tarball = make_tarball(&src_dir, &manifest, &files).unwrap();
        let _ = fs::remove_dir_all(&src_dir);
        tarball
    }

    fn write_registry_index(dir: &Path, entries: &[(&str, &str, &Path)]) -> PathBuf {
        let mut packages = serde_json::Map::new();
        for (name, version, tarball_path) in entries {
            let bytes = fs::read(tarball_path).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let sha = hasher
                .finalize()
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>();
            let versions = packages
                .entry(name.to_string())
                .or_insert_with(|| serde_json::json!({ "versions": {} }));
            versions["versions"][*version] = serde_json::json!({
                "source": tarball_path.to_string_lossy(),
                "sha256": sha,
            });
        }
        let index_json = serde_json::json!({ "packages": packages }).to_string();
        let index_path = dir.join("index.json");
        fs::write(&index_path, index_json).unwrap();
        index_path
    }

    #[test]
    fn resolve_registry_dep_fetches_verifies_and_extracts() {
        let project = tmp_dir("registry_resolve");
        let tarball = make_registry_package_tarball("mylib", "1.0.0", "pub fn hi() { return 1; }");
        let tarball_path = project.join("mylib-1.0.0.tar");
        fs::write(&tarball_path, &tarball).unwrap();
        let index_path = write_registry_index(&project, &[("mylib", "1.0.0", &tarball_path)]);

        let cache_root = project.join("cache");
        let resolved =
            resolve_registry_dep(&cache_root, "mylib", "1.0.0", index_path.to_str().unwrap())
                .unwrap();
        assert_eq!(resolved.name, "mylib");
        assert!(resolved.src_dir.join("main.rz").exists());
        assert_eq!(
            resolved.source,
            DepSource::Registry {
                version: "1.0.0".to_string()
            }
        );
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn resolve_registry_dep_reuses_cache_without_refetch() {
        let project = tmp_dir("registry_cache_reuse");
        let tarball = make_registry_package_tarball("mylib", "1.0.0", "pub fn hi() { return 1; }");
        let tarball_path = project.join("mylib-1.0.0.tar");
        fs::write(&tarball_path, &tarball).unwrap();
        let index_path = write_registry_index(&project, &[("mylib", "1.0.0", &tarball_path)]);
        let cache_root = project.join("cache");

        resolve_registry_dep(&cache_root, "mylib", "1.0.0", index_path.to_str().unwrap()).unwrap();
        // Delete the tarball and index — a cache hit must not need
        // either of them again.
        fs::remove_file(&tarball_path).unwrap();
        fs::remove_file(&index_path).unwrap();
        let resolved =
            resolve_registry_dep(&cache_root, "mylib", "1.0.0", index_path.to_str().unwrap())
                .unwrap();
        assert!(resolved.src_dir.join("main.rz").exists());
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn resolve_registry_dep_rejects_checksum_mismatch() {
        let project = tmp_dir("registry_checksum_mismatch");
        let tarball = make_registry_package_tarball("mylib", "1.0.0", "pub fn hi() { return 1; }");
        let tarball_path = project.join("mylib-1.0.0.tar");
        fs::write(&tarball_path, &tarball).unwrap();
        let index_json = format!(
            r#"{{"packages":{{"mylib":{{"versions":{{"1.0.0":{{"source":"{}","sha256":"{}"}}}}}}}}}}"#,
            tarball_path.to_string_lossy(),
            "0".repeat(64),
        );
        let index_path = project.join("index.json");
        fs::write(&index_path, index_json).unwrap();

        let cache_root = project.join("cache");
        let err = resolve_registry_dep(&cache_root, "mylib", "1.0.0", index_path.to_str().unwrap())
            .unwrap_err();
        assert!(matches!(err, PkgDepsError::RegistryError { .. }));
        assert!(err.to_string().contains("checksum mismatch"));
        // No cache directory should have been left behind with valid
        // contents from the rejected fetch.
        assert!(!cache_root.join("mylib/1.0.0/resilient.toml").exists());
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn resolve_registry_dep_missing_package_is_an_error() {
        let project = tmp_dir("registry_missing_pkg");
        let tarball = make_registry_package_tarball("mylib", "1.0.0", "pub fn hi() { return 1; }");
        let tarball_path = project.join("mylib-1.0.0.tar");
        fs::write(&tarball_path, &tarball).unwrap();
        let index_path = write_registry_index(&project, &[("mylib", "1.0.0", &tarball_path)]);
        let cache_root = project.join("cache");

        let err = resolve_registry_dep(
            &cache_root,
            "does-not-exist",
            "1.0.0",
            index_path.to_str().unwrap(),
        )
        .unwrap_err();
        assert!(matches!(err, PkgDepsError::RegistryError { .. }));
        assert!(err.to_string().contains("does-not-exist"));
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn parse_registry_dep_from_manifest() {
        let toml = "\
[dependencies]
mylib = { registry = \"1.2.3\" }
";
        let deps = parse_dependencies_from_str(toml).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(
            deps[0].source,
            DepSource::Registry {
                version: "1.2.3".to_string()
            }
        );
    }

    #[test]
    fn format_registry_dep_entry() {
        let dep = Dependency {
            name: "mylib".to_string(),
            source: DepSource::Registry {
                version: "1.2.3".to_string(),
            },
        };
        assert_eq!(format_dep_entry(&dep), "mylib = { registry = \"1.2.3\" }");
    }

    #[test]
    fn read_registry_index_from_manifest() {
        let toml = "\
[package]
name = \"proj\"

[registry]
index = \"https://example.com/index.json\"

[dependencies]
mylib = { registry = \"1.0.0\" }
";
        assert_eq!(
            read_registry_index_from_str(toml),
            Some("https://example.com/index.json".to_string())
        );
    }

    #[test]
    fn read_registry_index_absent_is_none() {
        let toml = "[package]\nname = \"proj\"\n";
        assert_eq!(read_registry_index_from_str(toml), None);
    }

    #[test]
    fn set_registry_index_appends_new_section() {
        let project = tmp_dir("set_registry_index_new");
        let manifest = project.join("resilient.toml");
        fs::write(&manifest, "[package]\nname = \"proj\"\n").unwrap();
        set_registry_index(&manifest, "https://example.com/index.json").unwrap();
        let content = fs::read_to_string(&manifest).unwrap();
        assert_eq!(
            read_registry_index_from_str(&content),
            Some("https://example.com/index.json".to_string())
        );
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn set_registry_index_updates_existing_value() {
        let project = tmp_dir("set_registry_index_update");
        let manifest = project.join("resilient.toml");
        fs::write(
            &manifest,
            "[package]\nname = \"proj\"\n\n[registry]\nindex = \"old-location\"\n",
        )
        .unwrap();
        set_registry_index(&manifest, "new-location").unwrap();
        let content = fs::read_to_string(&manifest).unwrap();
        assert_eq!(
            read_registry_index_from_str(&content),
            Some("new-location".to_string())
        );
        assert!(!content.contains("old-location"));
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn update_manifest_dep_version_rewrites_pin() {
        let project = tmp_dir("update_manifest_version");
        let manifest = project.join("resilient.toml");
        fs::write(
            &manifest,
            "[package]\nname = \"proj\"\n\n[dependencies]\n\
             mylib = { registry = \"1.0.0\" }\nother = { path = \"../other\" }\n",
        )
        .unwrap();
        update_manifest_dep_version(&manifest, "mylib", "1.1.0").unwrap();
        let deps = parse_dependencies(&manifest).unwrap();
        let mylib = deps.iter().find(|d| d.name == "mylib").unwrap();
        assert_eq!(
            mylib.source,
            DepSource::Registry {
                version: "1.1.0".to_string()
            }
        );
        // Untouched sibling dependency.
        let other = deps.iter().find(|d| d.name == "other").unwrap();
        assert_eq!(
            other.source,
            DepSource::Path {
                path: "../other".to_string()
            }
        );
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn update_manifest_dep_version_missing_dep_is_an_error() {
        let project = tmp_dir("update_manifest_missing");
        let manifest = project.join("resilient.toml");
        fs::write(&manifest, "[package]\nname = \"proj\"\n\n[dependencies]\n").unwrap();
        let err = update_manifest_dep_version(&manifest, "nope", "1.0.0").unwrap_err();
        assert!(matches!(err, PkgDepsError::DependencyNotFound { .. }));
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn registry_dep_end_to_end_resolve_and_lockfile() {
        // Mirrors `remove_dependency_end_to_end_updates_lockfile`'s
        // approach: exercise the pure core (`resolve_all` +
        // `write_lockfile`) rather than the CWD-driven CLI entry
        // point, since `add_registry_dependency` depends on
        // `env::current_dir()`.
        let project = tmp_dir("registry_e2e");
        let tarball = make_registry_package_tarball("mylib", "2.0.0", "pub fn hi() { return 1; }");
        let tarball_path = project.join("mylib-2.0.0.tar");
        fs::write(&tarball_path, &tarball).unwrap();
        let index_path = write_registry_index(&project, &[("mylib", "2.0.0", &tarball_path)]);

        let manifest = project.join("resilient.toml");
        fs::write(
            &manifest,
            format!(
                "[package]\nname = \"proj\"\n\n[registry]\nindex = \"{}\"\n\n\
                 [dependencies]\nmylib = {{ registry = \"2.0.0\" }}\n",
                index_path.to_string_lossy()
            ),
        )
        .unwrap();

        let resolved = resolve_all(&manifest).unwrap();
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].src_dir.join("main.rz").exists());
        write_lockfile(&project, &resolved).unwrap();

        let lock_content = fs::read_to_string(project.join(LOCKFILE_NAME)).unwrap();
        assert!(lock_content.contains("source = \"registry:mylib\""));
        assert!(lock_content.contains("rev = \"2.0.0\""));
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn resolve_all_registry_dep_without_configured_index_errors() {
        let project = tmp_dir("registry_no_index");
        let manifest = project.join("resilient.toml");
        fs::write(
            &manifest,
            "[package]\nname = \"proj\"\n\n[dependencies]\nmylib = { registry = \"1.0.0\" }\n",
        )
        .unwrap();
        let err = resolve_all(&manifest).unwrap_err();
        assert!(matches!(err, PkgDepsError::RegistryNotConfigured));
        let _ = fs::remove_dir_all(&project);
    }
}
