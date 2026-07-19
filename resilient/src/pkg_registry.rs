//! RES-4114: static JSON package-registry index + checksum verification.
//!
//! This is increment 1 of E-E2 (package-manager registry protocol):
//! the index *format* and integrity verification, with no network
//! access. A future increment wires `rz pkg add <name>` /
//! `rz pkg update` to fetch an index (and package contents) over
//! HTTP and call into this module to resolve + verify what comes
//! back. Schema is documented in `docs/PACKAGE_REGISTRY.md`.
//!
//! Index shape:
//!
//! ```json
//! {
//!   "packages": {
//!     "mylib": {
//!       "versions": {
//!         "1.0.0": {
//!           "source": "https://example.com/mylib-1.0.0.tar.gz",
//!           "sha256": "<64 hex chars>"
//!         }
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! Checksum verification is integrity-only (detects corruption /
//! tampering / bad mirrors), not a signing or trust mechanism — a
//! mismatch is always a hard error, never a silent fallback.

// Public API surface for this increment is exercised by the tests
// below; the CLI wiring (`rz pkg add <name>` / `rz pkg update`
// resolving through an index) lands in a follow-up PR on #4114, so
// `cargo build` alone sees these as unused. Mirrors the
// `#[allow(dead_code)]` pattern already used for forward-looking
// lockfile APIs elsewhere in this crate (see `pkg_deps::LockEntry`).
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::process::Command;

use sha2::{Digest, Sha256};

/// Hex SHA-256 of `bytes` (64 lowercase hex chars). `sha2` is an
/// unconditional dependency of this crate (see `crypto_hash.rs`),
/// so this does not need the `z3` feature that gates `cert_sign`.
fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// A parsed, validated registry index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryIndex {
    /// Package name -> package entry.
    pub packages: BTreeMap<String, PackageEntry>,
}

/// All published versions of a single package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageEntry {
    /// Version string -> version entry. Kept as a `BTreeMap` for
    /// deterministic iteration; version *ordering* for "latest"
    /// resolution is lexicographic on the raw string (see
    /// `latest_version`), which is a known limitation for non-padded
    /// semver (documented in `docs/PACKAGE_REGISTRY.md`).
    pub versions: BTreeMap<String, PackageVersion>,
}

/// A single published version of a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageVersion {
    /// Where the package contents live. Opaque to this module; a
    /// future fetch increment interprets it (e.g. an HTTP(S) URL to
    /// a tarball).
    pub source: String,
    /// Lowercase hex SHA-256 of the package contents referenced by
    /// `source`, checked via [`verify_checksum`] before the contents
    /// are used.
    pub sha256: String,
}

#[derive(Debug)]
pub enum PkgRegistryError {
    /// The index JSON did not parse.
    Malformed { detail: String },
    /// The index parsed as JSON but violated the schema (missing
    /// field, wrong type, malformed sha256, etc).
    InvalidSchema { detail: String },
    /// `name` has no entry in the index.
    PackageNotFound { name: String },
    /// `name` exists but not at the requested version.
    VersionNotFound { name: String, version: String },
    /// A package has no versions at all, so "latest" is undefined.
    NoVersions { name: String },
    /// Checksum verification failed — a hard error, never a silent
    /// fallback.
    ChecksumMismatch {
        name: String,
        version: String,
        expected: String,
        actual: String,
    },
    /// Fetching bytes from a `source`/index location failed (bad
    /// path, non-zero `curl`, missing `curl` binary, etc).
    FetchFailed { location: String, detail: String },
    /// The fetched package contents did not parse as a valid USTAR
    /// archive produced by `pkg_publish::make_tarball`.
    ExtractFailed { detail: String },
}

impl fmt::Display for PkgRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed { detail } => write!(f, "malformed registry index: {}", detail),
            Self::InvalidSchema { detail } => {
                write!(f, "invalid registry index schema: {}", detail)
            }
            Self::PackageNotFound { name } => {
                write!(f, "no package named `{}` in the registry index", name)
            }
            Self::VersionNotFound { name, version } => write!(
                f,
                "package `{}` has no version `{}` in the registry index",
                name, version
            ),
            Self::NoVersions { name } => write!(
                f,
                "package `{}` has no published versions in the registry index",
                name
            ),
            Self::ChecksumMismatch {
                name,
                version,
                expected,
                actual,
            } => write!(
                f,
                "checksum mismatch for `{}@{}`: expected sha256 {}, got {} \
                 — refusing to use possibly-corrupted or tampered package contents",
                name, version, expected, actual
            ),
            Self::FetchFailed { location, detail } => {
                write!(f, "failed to fetch `{}`: {}", location, detail)
            }
            Self::ExtractFailed { detail } => {
                write!(f, "failed to extract package archive: {}", detail)
            }
        }
    }
}

impl std::error::Error for PkgRegistryError {}

/// Parse and validate a registry index from a JSON string.
pub fn parse_index(json: &str) -> Result<RegistryIndex, PkgRegistryError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| PkgRegistryError::Malformed {
            detail: e.to_string(),
        })?;
    index_from_value(&value)
}

fn index_from_value(value: &serde_json::Value) -> Result<RegistryIndex, PkgRegistryError> {
    let obj = value
        .as_object()
        .ok_or_else(|| PkgRegistryError::InvalidSchema {
            detail: "top-level value must be a JSON object".to_string(),
        })?;
    let packages_val = obj
        .get("packages")
        .ok_or_else(|| PkgRegistryError::InvalidSchema {
            detail: "missing top-level `packages` field".to_string(),
        })?;
    let packages_obj = packages_val
        .as_object()
        .ok_or_else(|| PkgRegistryError::InvalidSchema {
            detail: "`packages` must be a JSON object".to_string(),
        })?;

    let mut packages = BTreeMap::new();
    for (name, entry_val) in packages_obj {
        let entry = package_entry_from_value(name, entry_val)?;
        packages.insert(name.clone(), entry);
    }
    Ok(RegistryIndex { packages })
}

fn package_entry_from_value(
    name: &str,
    value: &serde_json::Value,
) -> Result<PackageEntry, PkgRegistryError> {
    let obj = value
        .as_object()
        .ok_or_else(|| PkgRegistryError::InvalidSchema {
            detail: format!("package `{}` entry must be a JSON object", name),
        })?;
    let versions_val = obj
        .get("versions")
        .ok_or_else(|| PkgRegistryError::InvalidSchema {
            detail: format!("package `{}` is missing a `versions` field", name),
        })?;
    let versions_obj = versions_val
        .as_object()
        .ok_or_else(|| PkgRegistryError::InvalidSchema {
            detail: format!("package `{}` `versions` must be a JSON object", name),
        })?;

    let mut versions = BTreeMap::new();
    for (ver, ver_val) in versions_obj {
        let pv = package_version_from_value(name, ver, ver_val)?;
        versions.insert(ver.clone(), pv);
    }
    Ok(PackageEntry { versions })
}

fn package_version_from_value(
    name: &str,
    version: &str,
    value: &serde_json::Value,
) -> Result<PackageVersion, PkgRegistryError> {
    let obj = value
        .as_object()
        .ok_or_else(|| PkgRegistryError::InvalidSchema {
            detail: format!("`{}@{}` entry must be a JSON object", name, version),
        })?;
    let source = obj
        .get("source")
        .and_then(|v| v.as_str())
        .ok_or_else(|| PkgRegistryError::InvalidSchema {
            detail: format!("`{}@{}` is missing a string `source` field", name, version),
        })?
        .to_string();
    let sha256 = obj
        .get("sha256")
        .and_then(|v| v.as_str())
        .ok_or_else(|| PkgRegistryError::InvalidSchema {
            detail: format!("`{}@{}` is missing a string `sha256` field", name, version),
        })?
        .to_string();
    if !is_valid_sha256_hex(&sha256) {
        return Err(PkgRegistryError::InvalidSchema {
            detail: format!(
                "`{}@{}` has a malformed `sha256` field — expected 64 lowercase hex chars, got `{}`",
                name, version, sha256
            ),
        });
    }
    if source.trim().is_empty() {
        return Err(PkgRegistryError::InvalidSchema {
            detail: format!("`{}@{}` has an empty `source` field", name, version),
        });
    }
    Ok(PackageVersion { source, sha256 })
}

/// A hex SHA-256 digest: exactly 64 lowercase hex characters.
fn is_valid_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Resolve `name` (optionally pinned to `version`) against `index`.
/// Returns the resolved version string and its entry. If `version`
/// is `None`, resolves to the lexicographically-greatest version
/// string (see the caveat on [`PackageEntry::versions`]).
pub fn resolve_package<'a>(
    index: &'a RegistryIndex,
    name: &str,
    version: Option<&str>,
) -> Result<(&'a str, &'a PackageVersion), PkgRegistryError> {
    let entry = index
        .packages
        .get(name)
        .ok_or_else(|| PkgRegistryError::PackageNotFound {
            name: name.to_string(),
        })?;

    match version {
        Some(v) => {
            let (k, pv) = entry.versions.get_key_value(v).ok_or_else(|| {
                PkgRegistryError::VersionNotFound {
                    name: name.to_string(),
                    version: v.to_string(),
                }
            })?;
            Ok((k.as_str(), pv))
        }
        None => {
            let (k, pv) =
                entry
                    .versions
                    .iter()
                    .next_back()
                    .ok_or_else(|| PkgRegistryError::NoVersions {
                        name: name.to_string(),
                    })?;
            Ok((k.as_str(), pv))
        }
    }
}

/// Verify that `contents` hashes to the checksum recorded for
/// `name@version` in `expected`. A mismatch is a hard error — callers
/// must never fall back to using unverified contents.
pub fn verify_checksum(
    name: &str,
    version: &str,
    expected: &PackageVersion,
    contents: &[u8],
) -> Result<(), PkgRegistryError> {
    let actual = sha256_hex(contents);
    if actual.eq_ignore_ascii_case(&expected.sha256) {
        Ok(())
    } else {
        Err(PkgRegistryError::ChecksumMismatch {
            name: name.to_string(),
            version: version.to_string(),
            expected: expected.sha256.clone(),
            actual,
        })
    }
}

/// Fetch raw bytes from `location`, which is either:
///
/// - a bare filesystem path or `file://` URI (read directly, no
///   network I/O) — used for local indexes/packages and all hermetic
///   tests; or
/// - an `http://`/`https://` URL, fetched by shelling out to the
///   system `curl` binary.
///
/// Shelling out to `curl` (rather than adding an HTTP client crate)
/// mirrors the existing `git` dependency-resolution pattern in
/// `pkg_deps::resolve_git_dep` — see the supply-chain hygiene rule in
/// CLAUDE.md: prefer what's already on the system over a new
/// dependency for something this crate does rarely (fetching a
/// handful of index/package files, not serving high-throughput
/// traffic).
pub fn fetch_bytes(location: &str) -> Result<Vec<u8>, PkgRegistryError> {
    if let Some(path) = location.strip_prefix("file://") {
        return std::fs::read(path).map_err(|e| PkgRegistryError::FetchFailed {
            location: location.to_string(),
            detail: e.to_string(),
        });
    }
    if location.starts_with("http://") || location.starts_with("https://") {
        let output = Command::new("curl")
            .args(["-sSL", "--fail", location])
            .output()
            .map_err(|e| PkgRegistryError::FetchFailed {
                location: location.to_string(),
                detail: format!("failed to run curl (is it installed?): {}", e),
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PkgRegistryError::FetchFailed {
                location: location.to_string(),
                detail: if stderr.trim().is_empty() {
                    format!("curl exited with {}", output.status)
                } else {
                    stderr.trim().to_string()
                },
            });
        }
        return Ok(output.stdout);
    }
    // Bare path — no scheme prefix. Treat as a plain filesystem path,
    // which covers both absolute paths and paths relative to the
    // process's current directory.
    std::fs::read(location).map_err(|e| PkgRegistryError::FetchFailed {
        location: location.to_string(),
        detail: e.to_string(),
    })
}

/// Extract a USTAR archive (as produced by
/// `pkg_publish::make_tarball` — no compression, regular files only)
/// into `dest`. The archive's single top-level directory component
/// (e.g. `mylib-1.0.0/`) is stripped, so `dest` directly contains the
/// package's `resilient.toml` and `src/`.
pub fn extract_ustar(bytes: &[u8], dest: &Path) -> Result<(), PkgRegistryError> {
    std::fs::create_dir_all(dest).map_err(|e| PkgRegistryError::ExtractFailed {
        detail: format!("creating {}: {}", dest.display(), e),
    })?;

    let mut offset = 0usize;
    let mut wrote_any = false;
    while offset + 512 <= bytes.len() {
        let header = &bytes[offset..offset + 512];
        if header.iter().all(|&b| b == 0) {
            // End-of-archive marker (two consecutive zero blocks).
            break;
        }
        let name = parse_header_str(&header[0..100])?;
        let size = parse_octal(&header[124..136])?;
        let typeflag = header[156];
        offset += 512;
        let body_end =
            offset
                .checked_add(size as usize)
                .ok_or_else(|| PkgRegistryError::ExtractFailed {
                    detail: format!("archive entry `{}` has an invalid size", name),
                })?;
        if body_end > bytes.len() {
            return Err(PkgRegistryError::ExtractFailed {
                detail: format!("archive entry `{}` is truncated", name),
            });
        }
        let body = &bytes[offset..body_end];
        // Only regular files ('\0' or '0') are expected; make_tarball
        // never emits directories or links.
        if typeflag == b'0' || typeflag == 0 {
            // Strip the leading `<prefix>/` component so callers get
            // package contents rooted at `dest` directly.
            let rel = name
                .split_once('/')
                .map(|(_, rest)| rest)
                .unwrap_or(name.as_str());
            if !rel.is_empty() {
                let out_path = safe_join(dest, rel)?;
                if let Some(parent) = out_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        PkgRegistryError::ExtractFailed {
                            detail: format!("creating {}: {}", parent.display(), e),
                        }
                    })?;
                }
                std::fs::write(&out_path, body).map_err(|e| PkgRegistryError::ExtractFailed {
                    detail: format!("writing {}: {}", out_path.display(), e),
                })?;
                wrote_any = true;
            }
        }
        // Body is padded to the next 512-byte boundary.
        let pad = (512 - (size as usize % 512)) % 512;
        offset = body_end + pad;
    }
    if !wrote_any {
        return Err(PkgRegistryError::ExtractFailed {
            detail: "archive contained no regular files".to_string(),
        });
    }
    Ok(())
}

/// Join `rel` onto `dest`, rejecting any component that would escape
/// `dest` (`..` or an absolute path) — an archive should never be
/// able to write outside the directory it's being extracted into.
fn safe_join(dest: &Path, rel: &str) -> Result<std::path::PathBuf, PkgRegistryError> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() || rel_path.components().any(|c| c.as_os_str() == "..") {
        return Err(PkgRegistryError::ExtractFailed {
            detail: format!("archive entry path escapes destination: `{}`", rel),
        });
    }
    Ok(dest.join(rel_path))
}

fn parse_header_str(field: &[u8]) -> Result<String, PkgRegistryError> {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8(field[..end].to_vec()).map_err(|e| PkgRegistryError::ExtractFailed {
        detail: format!("non-UTF8 archive entry name: {}", e),
    })
}

fn parse_octal(field: &[u8]) -> Result<u64, PkgRegistryError> {
    let s = std::str::from_utf8(field)
        .map_err(|e| PkgRegistryError::ExtractFailed {
            detail: format!("non-UTF8 size field: {}", e),
        })?
        .trim_matches(char::from(0))
        .trim();
    if s.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(s, 8).map_err(|e| PkgRegistryError::ExtractFailed {
        detail: format!("malformed octal size field `{}`: {}", s, e),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_index_json() -> String {
        format!(
            r#"{{
              "packages": {{
                "mylib": {{
                  "versions": {{
                    "1.0.0": {{ "source": "https://example.com/mylib-1.0.0.tar.gz", "sha256": "{}" }},
                    "1.1.0": {{ "source": "https://example.com/mylib-1.1.0.tar.gz", "sha256": "{}" }}
                  }}
                }}
              }}
            }}"#,
            sha256_hex(b"mylib-1.0.0-contents"),
            sha256_hex(b"mylib-1.1.0-contents"),
        )
    }

    #[test]
    fn parses_well_formed_index() {
        let idx = parse_index(&sample_index_json()).unwrap();
        assert_eq!(idx.packages.len(), 1);
        let entry = idx.packages.get("mylib").unwrap();
        assert_eq!(entry.versions.len(), 2);
    }

    #[test]
    fn resolve_pinned_version_succeeds() {
        let idx = parse_index(&sample_index_json()).unwrap();
        let (v, pv) = resolve_package(&idx, "mylib", Some("1.0.0")).unwrap();
        assert_eq!(v, "1.0.0");
        assert_eq!(pv.source, "https://example.com/mylib-1.0.0.tar.gz");
    }

    #[test]
    fn resolve_latest_version_picks_greatest_string() {
        let idx = parse_index(&sample_index_json()).unwrap();
        let (v, _pv) = resolve_package(&idx, "mylib", None).unwrap();
        assert_eq!(v, "1.1.0");
    }

    #[test]
    fn resolve_missing_package_is_an_error() {
        let idx = parse_index(&sample_index_json()).unwrap();
        let err = resolve_package(&idx, "does-not-exist", None).unwrap_err();
        assert!(matches!(err, PkgRegistryError::PackageNotFound { .. }));
        assert!(err.to_string().contains("does-not-exist"));
    }

    #[test]
    fn resolve_missing_version_is_an_error() {
        let idx = parse_index(&sample_index_json()).unwrap();
        let err = resolve_package(&idx, "mylib", Some("9.9.9")).unwrap_err();
        assert!(matches!(err, PkgRegistryError::VersionNotFound { .. }));
    }

    #[test]
    fn checksum_matches_recorded_digest() {
        let idx = parse_index(&sample_index_json()).unwrap();
        let (v, pv) = resolve_package(&idx, "mylib", Some("1.0.0")).unwrap();
        verify_checksum("mylib", v, pv, b"mylib-1.0.0-contents").unwrap();
    }

    #[test]
    fn checksum_mismatch_is_rejected() {
        let idx = parse_index(&sample_index_json()).unwrap();
        let (v, pv) = resolve_package(&idx, "mylib", Some("1.0.0")).unwrap();
        let err = verify_checksum("mylib", v, pv, b"tampered-contents").unwrap_err();
        match err {
            PkgRegistryError::ChecksumMismatch { name, version, .. } => {
                assert_eq!(name, "mylib");
                assert_eq!(version, "1.0.0");
            }
            other => panic!("expected ChecksumMismatch, got {:?}", other),
        }
    }

    #[test]
    fn malformed_json_is_rejected() {
        let err = parse_index("not json").unwrap_err();
        assert!(matches!(err, PkgRegistryError::Malformed { .. }));
    }

    #[test]
    fn missing_packages_field_is_rejected() {
        let err = parse_index("{}").unwrap_err();
        assert!(matches!(err, PkgRegistryError::InvalidSchema { .. }));
    }

    #[test]
    fn malformed_sha256_is_rejected() {
        let json = r#"{
          "packages": {
            "mylib": {
              "versions": {
                "1.0.0": { "source": "https://example.com/x.tar.gz", "sha256": "not-hex" }
              }
            }
          }
        }"#;
        let err = parse_index(json).unwrap_err();
        assert!(matches!(err, PkgRegistryError::InvalidSchema { .. }));
    }

    #[test]
    fn empty_source_is_rejected() {
        let sha = sha256_hex(b"x");
        let json = format!(
            r#"{{
              "packages": {{
                "mylib": {{
                  "versions": {{
                    "1.0.0": {{ "source": "", "sha256": "{}" }}
                  }}
                }}
              }}
            }}"#,
            sha
        );
        let err = parse_index(&json).unwrap_err();
        assert!(matches!(err, PkgRegistryError::InvalidSchema { .. }));
    }

    #[test]
    fn no_versions_is_an_error() {
        let json = r#"{ "packages": { "mylib": { "versions": {} } } }"#;
        let idx = parse_index(json).unwrap();
        let err = resolve_package(&idx, "mylib", None).unwrap_err();
        assert!(matches!(err, PkgRegistryError::NoVersions { .. }));
    }

    // ── fetch_bytes / extract_ustar (RES-4114 increment 2) ────────

    #[test]
    fn fetch_bytes_reads_a_bare_path() {
        let dir = std::env::temp_dir().join(format!("rz-fetch-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("index.json");
        std::fs::write(&file, b"hello registry").unwrap();
        let bytes = fetch_bytes(file.to_str().unwrap()).unwrap();
        assert_eq!(bytes, b"hello registry");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fetch_bytes_reads_a_file_uri() {
        let dir = std::env::temp_dir().join(format!("rz-fetch-uri-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("index.json");
        std::fs::write(&file, b"file uri contents").unwrap();
        let uri = format!("file://{}", file.to_str().unwrap());
        let bytes = fetch_bytes(&uri).unwrap();
        assert_eq!(bytes, b"file uri contents");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fetch_bytes_missing_path_is_an_error() {
        let err = fetch_bytes("/definitely/not/a/real/path/index.json").unwrap_err();
        assert!(matches!(err, PkgRegistryError::FetchFailed { .. }));
    }

    #[test]
    fn extract_ustar_round_trips_a_tarball() {
        use crate::pkg_publish::{PublishManifest, make_tarball};
        let manifest = PublishManifest {
            name: "mylib".to_string(),
            version: "1.0.0".to_string(),
            description: None,
            entry: std::path::PathBuf::from("src/lib.rz"),
        };
        let src_dir = std::env::temp_dir().join(format!("rz-extract-src-{}", std::process::id()));
        std::fs::create_dir_all(src_dir.join("src")).unwrap();
        std::fs::write(src_dir.join("resilient.toml"), "[package]\n").unwrap();
        std::fs::write(src_dir.join("src/lib.rz"), "pub fn hello() { return 1; }").unwrap();
        let files = vec![
            std::path::PathBuf::from("resilient.toml"),
            std::path::PathBuf::from("src/lib.rz"),
        ];
        let tarball = make_tarball(&src_dir, &manifest, &files).unwrap();

        let dest = std::env::temp_dir().join(format!("rz-extract-dest-{}", std::process::id()));
        extract_ustar(&tarball, &dest).unwrap();
        assert!(dest.join("resilient.toml").exists());
        assert_eq!(
            std::fs::read_to_string(dest.join("src/lib.rz")).unwrap(),
            "pub fn hello() { return 1; }"
        );

        let _ = std::fs::remove_dir_all(&src_dir);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn extract_ustar_rejects_empty_archive() {
        let dest =
            std::env::temp_dir().join(format!("rz-extract-empty-dest-{}", std::process::id()));
        let empty = vec![0u8; 1024];
        let err = extract_ustar(&empty, &dest).unwrap_err();
        assert!(matches!(err, PkgRegistryError::ExtractFailed { .. }));
        let _ = std::fs::remove_dir_all(&dest);
    }
}
