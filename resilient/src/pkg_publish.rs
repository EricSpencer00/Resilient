//! RES-342: `resilient pkg publish` — package the current project
//! and (eventually) upload it to a registry.
//!
//! Per the ticket's note "the registry itself is out of scope", this
//! V1 ships:
//!
//! 1. Manifest reading — `read_publish_manifest` parses
//!    `resilient.toml` for the four fields the registry needs:
//!    `name`, `version`, `description`, `entry`.
//! 2. File enumeration — `collect_publishable_files` walks the
//!    project tree gathering source files, excluding patterns from
//!    `.gitignore` (a small subset — directory and `*.ext` shapes —
//!    sufficient for the `target/` and `cert/` lines `pkg init`
//!    writes by default).
//! 3. Tarball synthesis — `make_tarball` produces a deterministic
//!    in-memory tar archive of the collected files. We use a
//!    minimal hand-rolled tar writer (the format is stable and the
//!    fields we need fit in a single header struct) rather than
//!    pulling a `tar` crate dependency.
//! 4. Auth resolution — `resolve_auth` checks `RESILIENT_TOKEN` then
//!    `~/.resilient/credentials.toml`.
//! 5. Dry-run printing — `print_publish_summary` emits a
//!    user-readable report of what would be uploaded (filename,
//!    file count, tarball size, manifest fields, auth source).
//!
//! What this module deliberately does NOT do (deferred):
//!
//! - The HTTP POST to the registry. No registry endpoint is
//!   committed yet; until one is, `pkg publish` requires
//!   `--dry-run` and surfaces a clear "registry endpoint not
//!   configured" error otherwise.
//! - Version-collision detection. Without a registry to query,
//!   we can't know what's already been published.
//! - Source signing (RES-194-style certificates over the
//!   archive). A reasonable follow-up; the byte stream the dry
//!   run prints is the same input a future signer would consume.
//!
//! The module is self-contained and can be reused by `pkg pack`,
//! `pkg vendor`, or other future subcommands that need the same
//! "list publishable files / build a tarball" capability.

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Manifest fields needed for publication. Mirrors the `[package]`
/// keys `pkg init` writes today, plus `description` and `entry`
/// which the published-side schema requires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishManifest {
    pub name: String,
    pub version: String,
    /// Optional in `pkg init`'s manifest; required for publish.
    pub description: Option<String>,
    /// Path (relative to project root) of the entry-point source file.
    /// Defaults to `src/main.rz` when absent — matches the `pkg init`
    /// scaffold layout.
    pub entry: PathBuf,
}

/// Errors `pkg publish` can surface. Intentionally narrow — each
/// variant maps to a single, actionable user-facing message.
#[derive(Debug)]
pub enum PkgPublishError {
    ManifestNotFound {
        searched_from: PathBuf,
    },
    ManifestUnreadable {
        path: PathBuf,
        source: io::Error,
    },
    ManifestMissingField {
        field: &'static str,
    },
    Io {
        context: String,
        source: io::Error,
    },
    /// Caller invoked `pkg publish` without `--dry-run` while the
    /// registry endpoint is unconfigured. Returns the user back
    /// to a workable invocation.
    RegistryNotConfigured,
}

impl std::fmt::Display for PkgPublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
            Self::ManifestMissingField { field } => write!(
                f,
                "manifest is missing required field `[package].{}` \
                 — `pkg publish` needs name, version, and entry (src/main.rz by default)",
                field
            ),
            Self::Io { context, source } => write!(f, "{}: {}", context, source),
            Self::RegistryNotConfigured => write!(
                f,
                "no registry endpoint configured for `pkg publish`. \
                 Use `--dry-run` to verify what would be uploaded; \
                 wiring an actual registry is tracked separately"
            ),
        }
    }
}

impl std::error::Error for PkgPublishError {}

/// Source of the auth token, surfaced to the user during dry-run so
/// they can confirm the right credentials are in scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthSource {
    EnvVar,
    CredentialsFile(PathBuf),
    None,
}

/// Outcome of an auth lookup.
#[derive(Debug, Clone)]
pub struct AuthResolution {
    pub source: AuthSource,
    /// `Some(...)` when a token was located. The token itself is
    /// **not** printed during dry-run — only its source is. The
    /// field is read by the future HTTP-POST path; tests assert
    /// on it.
    #[allow(dead_code)]
    pub token: Option<String>,
}

/// Read `resilient.toml` at `manifest_path` into a `PublishManifest`.
///
/// Uses the same hand-rolled TOML-ish reader pattern as
/// `pkg_init::read_package_name`. Supports the manifest shape `pkg
/// init` writes (lines like `key = "value"` under `[package]`).
pub fn read_publish_manifest(manifest_path: &Path) -> Result<PublishManifest, PkgPublishError> {
    let contents =
        fs::read_to_string(manifest_path).map_err(|e| PkgPublishError::ManifestUnreadable {
            path: manifest_path.to_path_buf(),
            source: e,
        })?;
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    let mut description: Option<String> = None;
    let mut entry: Option<String> = None;
    let mut in_package = false;
    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let header = rest.trim_end_matches(']').trim();
            in_package = header == "package";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let v = val.trim().strip_prefix('"').unwrap_or(val.trim());
        let end = v.find('"').unwrap_or(v.len());
        let value = v[..end].to_string();
        match key {
            "name" if !value.is_empty() => name = Some(value),
            "version" if !value.is_empty() => version = Some(value),
            "description" if !value.is_empty() => description = Some(value),
            "entry" if !value.is_empty() => entry = Some(value),
            _ => {}
        }
    }
    let name = name.ok_or(PkgPublishError::ManifestMissingField { field: "name" })?;
    let version = version.ok_or(PkgPublishError::ManifestMissingField { field: "version" })?;
    let entry = entry.unwrap_or_else(|| "src/main.rz".to_string());
    Ok(PublishManifest {
        name,
        version,
        description,
        entry: PathBuf::from(entry),
    })
}

/// Walk `project_root` collecting source files for publication.
///
/// Excludes any path that matches a pattern from `.gitignore` at
/// the project root. Pattern support is intentionally narrow:
/// directory entries (e.g. `target/`) and `*.ext` glob shapes are
/// honored; full gitignore semantics (negation, recursion, double-
/// star, anchored patterns) are out of scope. The `.gitignore` that
/// `pkg init` writes uses only the simple shapes.
///
/// Hidden files / directories (anything starting with `.`) are
/// excluded by default — keeps `.git`, `.idea`, etc. out of the
/// archive without forcing every user to list them.
///
/// Returns paths relative to `project_root`, sorted, so the tarball
/// is deterministic across runs.
pub fn collect_publishable_files(project_root: &Path) -> Result<Vec<PathBuf>, PkgPublishError> {
    let ignore_patterns = read_gitignore_patterns(project_root);
    let mut out: Vec<PathBuf> = Vec::new();
    walk(project_root, project_root, &ignore_patterns, &mut out).map_err(|e| {
        PkgPublishError::Io {
            context: format!("walking {}", project_root.display()),
            source: e,
        }
    })?;
    out.sort();
    Ok(out)
}

fn read_gitignore_patterns(project_root: &Path) -> Vec<String> {
    let path = project_root.join(".gitignore");
    let Ok(contents) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    contents
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

fn walk(
    root: &Path,
    dir: &Path,
    ignore_patterns: &[String],
    out: &mut Vec<PathBuf>,
) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        // Skip hidden entries — keeps `.git`, `.cache`, `.DS_Store`
        // etc. out of every archive.
        if name.starts_with('.') {
            continue;
        }
        let rel = match path.strip_prefix(root) {
            Ok(r) => r.to_path_buf(),
            Err(_) => continue,
        };
        if matches_ignore(&rel, ignore_patterns) {
            continue;
        }
        if path.is_dir() {
            walk(root, &path, ignore_patterns, out)?;
        } else if path.is_file() {
            out.push(rel);
        }
    }
    Ok(())
}

/// Test whether a relative path matches any of the simple gitignore
/// patterns. Supported shapes:
///
/// * `dir/`  — exclude any path whose first segment is `dir`.
/// * `*.ext` — exclude any path whose final segment ends in `.ext`.
/// * `name`  — exclude any path with that exact final segment, or
///   whose first segment matches `name` (treated as a directory).
fn matches_ignore(rel: &Path, patterns: &[String]) -> bool {
    let s = rel.to_string_lossy();
    let first_segment = s.split(['/', '\\']).next().unwrap_or("");
    let last_segment = rel.file_name().and_then(|n| n.to_str()).unwrap_or("");
    for pat in patterns {
        // Normalize trailing slash on directory patterns.
        let p = pat.trim_end_matches('/');
        if let Some(ext) = p.strip_prefix("*.") {
            if last_segment.ends_with(&format!(".{}", ext)) {
                return true;
            }
            continue;
        }
        if p.contains('/') {
            // Sub-path patterns aren't supported — treat as no-match
            // rather than a parse error.
            continue;
        }
        if first_segment == p || last_segment == p {
            return true;
        }
    }
    false
}

/// Pack `files` (each relative to `project_root`) into a deterministic
/// USTAR-format tar archive in memory. No compression — the registry
/// (or a future caller) can layer gzip on top if it wants to.
///
/// The archive uses a fixed prefix derived from `<name>-<version>/`
/// so unpackers see a single top-level directory matching the
/// canonical "downloaded crate" layout.
pub fn make_tarball(
    project_root: &Path,
    manifest: &PublishManifest,
    files: &[PathBuf],
) -> Result<Vec<u8>, PkgPublishError> {
    let prefix = format!("{}-{}", manifest.name, manifest.version);
    let mut out: Vec<u8> = Vec::new();
    for rel in files {
        let abs = project_root.join(rel);
        let body = fs::read(&abs).map_err(|e| PkgPublishError::Io {
            context: format!("reading {}", abs.display()),
            source: e,
        })?;
        let archived_name = format!("{}/{}", prefix, rel.to_string_lossy().replace('\\', "/"));
        write_tar_entry(&mut out, &archived_name, &body).map_err(|e| PkgPublishError::Io {
            context: format!("writing tar entry {}", archived_name),
            source: e,
        })?;
    }
    // Two trailing 512-byte zero blocks per the USTAR end-of-archive
    // convention.
    out.extend(std::iter::repeat_n(0u8, 1024));
    Ok(out)
}

/// Append one regular-file entry to a USTAR archive in `buf`. The
/// header is the classic 512-byte block; body is padded to the next
/// 512-byte boundary.
fn write_tar_entry(buf: &mut Vec<u8>, name: &str, body: &[u8]) -> io::Result<()> {
    if name.len() > 100 {
        // USTAR allows up to 255 with the `prefix` field; for our
        // packages the 100-byte cap is fine — surface a clear error
        // if anyone hits it.
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path too long for ustar header: {}", name),
        ));
    }
    let mut header = [0u8; 512];
    header[..name.len()].copy_from_slice(name.as_bytes());
    // mode (0644 for regular files), as octal in 8 bytes including NUL.
    write_octal(&mut header[100..108], 0o0000644);
    // owner / group uid + gid: 0 (root) — published archives don't
    // carry meaningful ownership.
    write_octal(&mut header[108..116], 0);
    write_octal(&mut header[116..124], 0);
    // size (12 bytes, octal).
    write_octal(&mut header[124..136], body.len() as u64);
    // mtime — fixed (epoch) for deterministic builds.
    write_octal(&mut header[136..148], 0);
    // checksum field — written below after the rest of the header
    // is filled. Initialize to 8 spaces per spec.
    for b in &mut header[148..156] {
        *b = b' ';
    }
    header[156] = b'0'; // typeflag: '0' = regular file
    // magic + version: "ustar\0" + "00"
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    // uname / gname: "root"
    header[265..269].copy_from_slice(b"root");
    header[297..301].copy_from_slice(b"root");
    // checksum: sum of all header bytes treated as unsigned, with
    // the checksum field counted as 8 spaces. Encoded as 6-octal
    // digits + NUL + space.
    let sum: u32 = header.iter().map(|&b| b as u32).sum();
    write_octal(&mut header[148..154], u64::from(sum));
    header[154] = 0; // NUL terminator
    header[155] = b' '; // followed by space, per the standard
    buf.extend_from_slice(&header);
    buf.extend_from_slice(body);
    let pad = (512 - (body.len() % 512)) % 512;
    buf.extend(std::iter::repeat_n(0u8, pad));
    Ok(())
}

/// Write `value` as an octal ASCII number into `out`, NUL-padded
/// on the right. This is the format every fixed-width numeric field
/// in a USTAR header uses (mode, uid, gid, size, mtime, ...).
fn write_octal(out: &mut [u8], value: u64) {
    // The last byte is always NUL; the rest hold the octal digits
    // right-justified, with leading zeros if necessary.
    let last = out.len() - 1;
    let mut v = value;
    let mut i = last - 1;
    loop {
        out[i] = b'0' + ((v & 0b111) as u8);
        v >>= 3;
        if i == 0 {
            break;
        }
        i -= 1;
    }
    out[last] = 0;
}

/// Resolve the publish auth token. Order of precedence:
///
/// 1. `RESILIENT_TOKEN` env var — preferred for CI environments.
/// 2. `~/.resilient/credentials.toml` with a `token = "..."` line.
/// 3. None — the dry-run still works (auth source reported as
///    `None`); a real upload would fail.
pub fn resolve_auth() -> AuthResolution {
    if let Ok(t) = env::var("RESILIENT_TOKEN")
        && !t.is_empty()
    {
        return AuthResolution {
            source: AuthSource::EnvVar,
            token: Some(t),
        };
    }
    if let Some(home) = home_dir() {
        let cred = home.join(".resilient").join("credentials.toml");
        if let Ok(contents) = fs::read_to_string(&cred)
            && let Some(token) = extract_token_line(&contents)
        {
            return AuthResolution {
                source: AuthSource::CredentialsFile(cred),
                token: Some(token),
            };
        }
    }
    AuthResolution {
        source: AuthSource::None,
        token: None,
    }
}

/// Cross-platform `~` resolution that doesn't pull a dirs crate.
/// Returns `None` when neither `HOME` nor `USERPROFILE` is set —
/// matches the (intentional) behaviour of cargo's bootstrap code.
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

/// Pull the first `token = "..."` value out of a credentials file.
/// Same lenient TOML-ish shape used elsewhere in this module.
fn extract_token_line(contents: &str) -> Option<String> {
    for raw in contents.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=')
            && k.trim() == "token"
        {
            let v = v.trim().strip_prefix('"')?;
            let end = v.find('"')?;
            let token = &v[..end];
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
}

/// Print a human-readable dry-run summary to `w` (typically stdout).
/// Lists the manifest fields, the file count + total raw size, the
/// resolved tarball size, and the auth source.
pub fn print_publish_summary<W: Write>(
    w: &mut W,
    manifest: &PublishManifest,
    files: &[PathBuf],
    tarball_size: usize,
    auth: &AuthResolution,
) -> io::Result<()> {
    writeln!(
        w,
        "Dry-run: would publish {}-{}",
        manifest.name, manifest.version
    )?;
    writeln!(
        w,
        "  description: {}",
        manifest.description.as_deref().unwrap_or("(none)")
    )?;
    writeln!(w, "  entry:       {}", manifest.entry.display())?;
    writeln!(w, "  files:       {}", files.len())?;
    for p in files {
        writeln!(w, "    {}", p.display())?;
    }
    writeln!(
        w,
        "  archive:     {} bytes (uncompressed tar)",
        tarball_size
    )?;
    writeln!(w, "  auth:        {}", describe_auth(&auth.source))?;
    writeln!(w)?;
    writeln!(
        w,
        "(no upload performed — `pkg publish` ships dry-run-only \
         until a registry endpoint is configured)"
    )?;
    Ok(())
}

fn describe_auth(source: &AuthSource) -> String {
    match source {
        AuthSource::EnvVar => "from RESILIENT_TOKEN env var".to_string(),
        AuthSource::CredentialsFile(p) => format!("from {}", p.display()),
        AuthSource::None => {
            "none (set RESILIENT_TOKEN or ~/.resilient/credentials.toml)".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trip() {
        let tmp = std::env::temp_dir().join("res-pub-test-1");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("resilient.toml");
        fs::write(
            &path,
            "[package]\n\
             name = \"demo\"\n\
             version = \"1.2.3\"\n\
             description = \"hello\"\n\
             entry = \"src/main.rz\"\n",
        )
        .unwrap();
        let m = read_publish_manifest(&path).unwrap();
        assert_eq!(m.name, "demo");
        assert_eq!(m.version, "1.2.3");
        assert_eq!(m.description.as_deref(), Some("hello"));
        assert_eq!(m.entry, PathBuf::from("src/main.rz"));
    }

    #[test]
    fn manifest_defaults_entry_when_absent() {
        let tmp = std::env::temp_dir().join("res-pub-test-2");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("resilient.toml");
        fs::write(&path, "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n").unwrap();
        let m = read_publish_manifest(&path).unwrap();
        assert_eq!(m.entry, PathBuf::from("src/main.rz"));
    }

    #[test]
    fn manifest_errors_on_missing_name_or_version() {
        let tmp = std::env::temp_dir().join("res-pub-test-3");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("resilient.toml");
        fs::write(&path, "[package]\nversion = \"0.1.0\"\n").unwrap();
        let err = read_publish_manifest(&path).unwrap_err();
        assert!(matches!(
            err,
            PkgPublishError::ManifestMissingField { field: "name" }
        ));
    }

    #[test]
    fn collects_files_excluding_gitignore_and_hidden() {
        let tmp = std::env::temp_dir().join("res-pub-test-4");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("src")).unwrap();
        fs::create_dir_all(tmp.join("target")).unwrap();
        fs::create_dir_all(tmp.join(".git")).unwrap();
        fs::write(tmp.join("resilient.toml"), "").unwrap();
        fs::write(tmp.join("src/main.rz"), "fn main() {}").unwrap();
        fs::write(tmp.join("src/util.rz"), "// util").unwrap();
        fs::write(tmp.join("target/build.log"), "log").unwrap();
        fs::write(tmp.join(".git/HEAD"), "ref").unwrap();
        fs::write(tmp.join("README.md"), "# demo").unwrap();
        fs::write(tmp.join(".gitignore"), "target/\n").unwrap();
        let files = collect_publishable_files(&tmp).unwrap();
        let names: Vec<String> = files
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"resilient.toml".to_string()));
        assert!(names.contains(&"README.md".to_string()));
        assert!(
            names
                .iter()
                .any(|n| n == "src/main.rz" || n == "src\\main.rz")
        );
        assert!(
            names
                .iter()
                .any(|n| n == "src/util.rz" || n == "src\\util.rz")
        );
        // Excluded:
        assert!(!names.iter().any(|n| n.starts_with("target")));
        assert!(!names.iter().any(|n| n.starts_with(".git")));
    }

    #[test]
    fn ignore_patterns_handle_extension_glob() {
        let tmp = std::env::temp_dir().join("res-pub-test-5");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("a.rz"), "").unwrap();
        fs::write(tmp.join("a.tmp"), "").unwrap();
        fs::write(tmp.join(".gitignore"), "*.tmp\n").unwrap();
        let files = collect_publishable_files(&tmp).unwrap();
        assert!(files.iter().any(|p| p.to_string_lossy() == "a.rz"));
        assert!(!files.iter().any(|p| p.to_string_lossy() == "a.tmp"));
    }

    #[test]
    fn tarball_is_deterministic() {
        let tmp = std::env::temp_dir().join("res-pub-test-6");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("src")).unwrap();
        fs::write(
            tmp.join("resilient.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1\"\n",
        )
        .unwrap();
        fs::write(tmp.join("src/main.rz"), "fn main() {}").unwrap();
        let manifest = PublishManifest {
            name: "x".into(),
            version: "0.1".into(),
            description: None,
            entry: PathBuf::from("src/main.rz"),
        };
        let files = collect_publishable_files(&tmp).unwrap();
        let a = make_tarball(&tmp, &manifest, &files).unwrap();
        let b = make_tarball(&tmp, &manifest, &files).unwrap();
        assert_eq!(a, b, "tarball must be deterministic across runs");
        // USTAR magic appears at offset 257 in the first header.
        assert_eq!(&a[257..262], b"ustar");
    }

    #[test]
    fn auth_env_var_takes_precedence() {
        // Save/restore env so concurrent tests don't see leakage.
        let prev = env::var("RESILIENT_TOKEN").ok();
        unsafe {
            env::set_var("RESILIENT_TOKEN", "secret-from-env");
        }
        let r = resolve_auth();
        assert!(matches!(r.source, AuthSource::EnvVar));
        assert_eq!(r.token.as_deref(), Some("secret-from-env"));
        unsafe {
            match prev {
                Some(v) => env::set_var("RESILIENT_TOKEN", v),
                None => env::remove_var("RESILIENT_TOKEN"),
            }
        }
    }

    #[test]
    fn auth_returns_none_when_neither_source_present() {
        let prev = env::var("RESILIENT_TOKEN").ok();
        let prev_home = env::var("HOME").ok();
        let tmp = std::env::temp_dir().join("res-pub-no-creds");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        unsafe {
            env::remove_var("RESILIENT_TOKEN");
            env::set_var("HOME", &tmp);
        }
        let r = resolve_auth();
        assert!(matches!(r.source, AuthSource::None));
        assert!(r.token.is_none());
        unsafe {
            match prev {
                Some(v) => env::set_var("RESILIENT_TOKEN", v),
                None => env::remove_var("RESILIENT_TOKEN"),
            }
            match prev_home {
                Some(v) => env::set_var("HOME", v),
                None => env::remove_var("HOME"),
            }
        }
    }

    #[test]
    fn dry_run_summary_is_human_readable() {
        let manifest = PublishManifest {
            name: "demo".into(),
            version: "0.2.0".into(),
            description: Some("demo project".into()),
            entry: PathBuf::from("src/main.rz"),
        };
        let files = vec![
            PathBuf::from("resilient.toml"),
            PathBuf::from("src/main.rz"),
        ];
        let auth = AuthResolution {
            source: AuthSource::None,
            token: None,
        };
        let mut buf: Vec<u8> = Vec::new();
        print_publish_summary(&mut buf, &manifest, &files, 4096, &auth).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("demo-0.2.0"));
        assert!(s.contains("demo project"));
        assert!(s.contains("src/main.rz"));
        assert!(s.contains("4096 bytes"));
        assert!(s.contains("(no upload performed"));
    }
}
