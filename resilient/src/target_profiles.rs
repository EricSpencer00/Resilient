//! RES-2614: Cross-compilation target profiles in rz.toml.
//!
//! Parses `[target.TRIPLE]` sections from a Resilient manifest and
//! exposes the resolved profile for `rz build --target TRIPLE`.
//!
//! ## Manifest shape
//!
//! ```toml
//! [package]
//! name = "myapp"
//! version = "1.0.0"
//!
//! [target.thumbv7em-none-eabihf]
//! features  = ["no_std", "cortex-m"]
//! opt_level = "s"
//! stack_size = 8192
//!
//! [target.x86_64-unknown-linux]
//! features  = ["std", "networking"]
//! opt_level = "3"
//! ```
//!
//! ## Merge rules
//!
//! Profile fields are *merged* on top of a bare default:
//! - `features`   — replaces the default empty list.
//! - `opt_level`  — replaces "0" (the default).
//! - `stack_size` — replaces `None` (not set by default).
//! - `cfg`        — per-target key/value cfg pairs added to the cfg map.
//!
//! When `--target TRIPLE` is given but no matching section exists in
//! rz.toml the compiler warns and falls back to the default profile.
//!
//! ## Validation (typecheck pass)
//!
//! The `check` function is called from the `<EXTENSION_PASSES>` block in
//! `typechecker.rs`. It is a no-op when no manifest is present; when a
//! manifest is found it validates that every `[target.X]` section has a
//! syntactically valid `opt_level` and a positive `stack_size`.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;

// ── Data types ──────────────────────────────────────────────────────────────

/// A single `[target.TRIPLE]` profile as parsed from rz.toml.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TargetProfile {
    /// Feature flags activated for this target.
    pub features: Vec<String>,
    /// Optimization level: "0", "1", "2", "3", or "s".
    pub opt_level: String,
    /// Optional stack size in bytes (embedded linker hint).
    pub stack_size: Option<u64>,
    /// Additional target-specific cfg key/value pairs.
    pub cfg: HashMap<String, String>,
}

impl TargetProfile {
    /// Construct the built-in default profile.
    ///
    /// Used when no manifest exists or no `[target.X]` section matches.
    pub fn default_profile() -> Self {
        Self {
            features: Vec::new(),
            opt_level: "0".to_string(),
            stack_size: None,
            cfg: HashMap::new(),
        }
    }
}

// ── Valid opt_level values ──────────────────────────────────────────────────

const VALID_OPT_LEVELS: &[&str] = &["0", "1", "2", "3", "s"];

fn is_valid_opt_level(s: &str) -> bool {
    VALID_OPT_LEVELS.contains(&s)
}

// ── Parser ──────────────────────────────────────────────────────────────────

/// Parse all `[target.TRIPLE]` sections from a manifest string.
///
/// Returns a map of target triple → profile.  Sections for other headers
/// (`[package]`, `[dependencies]`) are silently skipped.
pub fn parse_target_profiles(manifest: &str) -> HashMap<String, TargetProfile> {
    let mut profiles: HashMap<String, TargetProfile> = HashMap::new();
    let mut current_triple: Option<String> = None;

    for raw in manifest.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Section header detection.
        if let Some(rest) = line.strip_prefix('[') {
            // Skip `[[` array-of-tables headers (lock-file format).
            if rest.starts_with('[') {
                current_triple = None;
                continue;
            }
            let header = rest.trim_end_matches(']').trim();
            // `[target.TRIPLE]` — enter the profile for TRIPLE.
            if let Some(triple) = header.strip_prefix("target.") {
                let triple = triple.trim().to_string();
                profiles.entry(triple.clone()).or_default();
                current_triple = Some(triple);
            } else {
                // Any other section exits target-profile mode.
                current_triple = None;
            }
            continue;
        }

        // Key = value lines inside a [target.TRIPLE] section.
        let Some(ref triple) = current_triple else {
            continue;
        };
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim();
        let profile = profiles.entry(triple.clone()).or_default();

        match key {
            "opt_level" => {
                // Accept both `"s"` (quoted) and `3` (unquoted integers).
                let v = val.trim_matches('"').to_string();
                profile.opt_level = v;
            }
            "stack_size" => {
                // Plain integer, no quotes.
                if let Ok(n) = val.parse::<u64>() {
                    profile.stack_size = Some(n);
                }
            }
            "features" => {
                // `["a", "b", ...]` — parse quoted strings inside brackets.
                profile.features = parse_string_array(val);
            }
            other => {
                // Treat remaining key = "value" pairs as cfg entries.
                if let Some(v) = extract_quoted_string(val) {
                    profile.cfg.insert(other.to_string(), v);
                }
            }
        }
    }

    profiles
}

/// Look up the profile for `target_triple` in the map, falling back to
/// [`TargetProfile::default_profile`] when the triple is absent.
///
/// Emits a warning to stderr if the triple is explicitly provided but
/// no matching section was found.
pub fn resolve_profile<'a>(
    profiles: &'a HashMap<String, TargetProfile>,
    target_triple: Option<&str>,
) -> Option<&'a TargetProfile> {
    let Some(triple) = target_triple else {
        return None; // no --target flag → caller uses default
    };
    if let Some(p) = profiles.get(triple) {
        return Some(p);
    }
    // No match — warn and signal the fallback via None; the caller will
    // use TargetProfile::default_profile().
    eprintln!(
        "warning[target-profiles]: no `[target.{}]` section found in rz.toml; \
         using default profile",
        triple
    );
    None
}

// ── Typecheck pass ──────────────────────────────────────────────────────────

/// Validate `[target.TRIPLE]` sections nearest `rz.toml`.
///
/// Called from `typechecker.rs` `<EXTENSION_PASSES>`.  No-op when no
/// manifest is found.  Errors are reported as structured diagnostics.
pub(crate) fn check(_program: &crate::Node, source_path: &str) -> Result<(), String> {
    let source_dir = Path::new(source_path).parent().unwrap_or(Path::new("."));

    let manifest_path = ["rz.toml", "resilient.toml"]
        .iter()
        .map(|name| source_dir.join(name))
        .find(|p| p.exists());

    let manifest_path = match manifest_path {
        Some(path) => path,
        None => return Ok(()),
    };

    let manifest_content = match std::fs::read_to_string(&manifest_path) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };

    let manifest_display = manifest_path.display().to_string();
    let mut errors: Vec<String> = Vec::new();
    let mut seen_triples: HashMap<String, usize> = HashMap::new();
    let mut current_section: Option<SectionState> = None;

    for (idx, raw_line) in manifest_content.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw_line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with('[') {
            if let Some(section) = current_section.take()
                && !section.saw_field
            {
                errors.push(diagnostic(
                    &manifest_display,
                    section.header_line,
                    1,
                    &format!(
                        "invalid [target.{}] declaration: missing required fields",
                        section.triple
                    ),
                ));
            }

            if trimmed.starts_with("[[") {
                if trimmed.contains("[target.") {
                    errors.push(diagnostic(
                        &manifest_display,
                        line_no,
                        raw_line.find('[').map(|idx| idx + 1).unwrap_or(1),
                        "invalid target profile declaration shape: array-of-tables syntax is not supported",
                    ));
                }
                continue;
            }

            let Some(header) = trimmed
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            else {
                if trimmed.contains("[target") {
                    errors.push(diagnostic(
                        &manifest_display,
                        line_no,
                        raw_line.find('[').map(|idx| idx + 1).unwrap_or(1),
                        "invalid target profile declaration shape: malformed section header",
                    ));
                }
                continue;
            };

            let header = header.trim();
            let header_col = raw_line.find('[').map(|idx| idx + 1).unwrap_or(1);

            if header == "target" {
                errors.push(diagnostic(
                    &manifest_display,
                    line_no,
                    header_col,
                    "invalid target profile declaration: missing required field `triple`",
                ));
                continue;
            }

            let Some(triple) = header.strip_prefix("target.") else {
                current_section = None;
                continue;
            };

            let triple = triple.trim();
            if triple.is_empty() {
                errors.push(diagnostic(
                    &manifest_display,
                    line_no,
                    header_col,
                    "invalid target profile declaration: missing required field `triple`",
                ));
                continue;
            }

            if !is_valid_target_triple(triple) {
                errors.push(diagnostic(
                    &manifest_display,
                    line_no,
                    header_col,
                    &format!(
                        "invalid target profile declaration shape: malformed target triple `{triple}`"
                    ),
                ));
                continue;
            }

            if let Some(previous_line) = seen_triples.insert(triple.to_string(), line_no) {
                errors.push(diagnostic(
                    &manifest_display,
                    line_no,
                    header_col,
                    &format!(
                        "invalid target profile combination: duplicate `[target.{triple}]` declaration; previous declaration at line {previous_line}"
                    ),
                ));
                current_section = None;
                continue;
            }

            current_section = Some(SectionState::new(triple.to_string(), line_no));
            continue;
        }

        let Some(section) = current_section.as_mut() else {
            continue;
        };

        let Some((raw_key, raw_value)) = trimmed.split_once('=') else {
            errors.push(diagnostic(
                &manifest_display,
                line_no,
                raw_line.find(trimmed).map(|idx| idx + 1).unwrap_or(1),
                &format!(
                    "invalid [target.{}] declaration: malformed entry `{trimmed}`; expected `key = value`",
                    section.triple
                ),
            ));
            continue;
        };

        let key = raw_key.trim();
        let value = raw_value.trim();
        if key.is_empty() {
            errors.push(diagnostic(
                &manifest_display,
                line_no,
                raw_line.find(trimmed).map(|idx| idx + 1).unwrap_or(1),
                &format!(
                    "invalid [target.{}] declaration: malformed entry `{trimmed}`; expected a field name",
                    section.triple
                ),
            ));
            continue;
        }

        section.saw_field = true;
        let key_col = raw_line.find(key).map(|idx| idx + 1).unwrap_or(1);
        if let Some(previous_line) = section.seen_keys.insert(key.to_string(), line_no) {
            errors.push(diagnostic(
                &manifest_display,
                line_no,
                key_col,
                &format!(
                    "invalid [target.{}] combination: duplicate `{key}` field; previous declaration at line {previous_line}",
                    section.triple
                ),
            ));
            continue;
        }

        match key {
            "features" => {
                if let Err(message) = validate_features_value(value, &section.triple) {
                    errors.push(diagnostic(&manifest_display, line_no, key_col, &message));
                }
            }
            "opt_level" => {
                if let Err(message) = validate_opt_level_value(value, &section.triple) {
                    errors.push(diagnostic(&manifest_display, line_no, key_col, &message));
                }
            }
            "stack_size" => {
                if let Err(message) = validate_stack_size_value(value, &section.triple) {
                    errors.push(diagnostic(&manifest_display, line_no, key_col, &message));
                }
            }
            _ => {
                if let Err(message) = validate_cfg_value(value, &section.triple, key) {
                    errors.push(diagnostic(&manifest_display, line_no, key_col, &message));
                }
            }
        }
    }

    if let Some(section) = current_section
        && !section.saw_field
    {
        errors.push(diagnostic(
            &manifest_display,
            section.header_line,
            1,
            &format!(
                "invalid [target.{}] declaration: missing required fields",
                section.triple
            ),
        ));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(
            "
",
        ))
    }
}

fn diagnostic(source_path: &str, line: usize, column: usize, message: &str) -> String {
    format!("{source_path}:{line}:{column}: error[target-profiles]: {message}")
}

#[derive(Default)]
struct SectionState {
    triple: String,
    header_line: usize,
    saw_field: bool,
    seen_keys: HashMap<String, usize>,
}

impl SectionState {
    fn new(triple: String, header_line: usize) -> Self {
        Self {
            triple,
            header_line,
            saw_field: false,
            seen_keys: HashMap::new(),
        }
    }
}

fn is_valid_target_triple(triple: &str) -> bool {
    !triple.is_empty()
        && triple
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '+'))
}

fn validate_features_value(value: &str, triple: &str) -> Result<(), String> {
    let Some(items) = value
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    else {
        return Err(format!(
            "invalid [target.{triple}] declaration: `features` must be a string array"
        ));
    };

    for item in items.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        if extract_quoted_string(item).is_none() {
            return Err(format!(
                "invalid [target.{triple}] declaration: `features` entries must be double-quoted strings"
            ));
        }
    }

    Ok(())
}

fn validate_opt_level_value(value: &str, triple: &str) -> Result<(), String> {
    let raw = value.trim().trim_matches('"');
    if is_valid_opt_level(raw) {
        Ok(())
    } else {
        Err(format!(
            "invalid [target.{triple}] declaration: `opt_level` `{raw}`; expected one of: 0, 1, 2, 3, s"
        ))
    }
}

fn validate_stack_size_value(value: &str, triple: &str) -> Result<(), String> {
    let raw = value.trim().trim_matches('"');
    match raw.parse::<u64>() {
        Ok(0) | Err(_) => Err(format!(
            "invalid [target.{triple}] declaration: `stack_size` must be a positive integer, got `{raw}`"
        )),
        Ok(_) => Ok(()),
    }
}

fn validate_cfg_value(value: &str, triple: &str, key: &str) -> Result<(), String> {
    if extract_quoted_string(value).is_some() {
        Ok(())
    } else {
        Err(format!(
            "invalid [target.{triple}] declaration: cfg field `{key}` must use a double-quoted string value"
        ))
    }
}

// ── Small helpers ───────────────────────────────────────────────────────────

/// Parse TOML string array: `["a", "b", "c"]`.
fn parse_string_array(s: &str) -> Vec<String> {
    let inner = s.trim();
    let inner = inner
        .strip_prefix('[')
        .and_then(|r| r.strip_suffix(']'))
        .unwrap_or(inner);
    inner
        .split(',')
        .filter_map(|item| {
            let item = item.trim();
            if item.is_empty() {
                return None;
            }
            extract_quoted_string(item)
        })
        .collect()
}

/// Extract the content of a double-quoted string: `"foo"` → `Some("foo")`.
fn extract_quoted_string(s: &str) -> Option<String> {
    let s = s.trim();
    let rest = s.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    const MANIFEST_PREFIX: &str = "[package]\nname = \"myapp\"\nversion = \"1.0.0\"\n\n";

    fn make_manifest(extra: &str) -> String {
        format!(
            "[package]\nname = \"myapp\"\nversion = \"1.0.0\"\n\n{}",
            extra
        )
    }

    #[test]
    fn parse_single_target_section() {
        let manifest = make_manifest(
            "[target.thumbv7em-none-eabihf]\n\
             features = [\"no_std\", \"cortex-m\"]\n\
             opt_level = \"s\"\n\
             stack_size = 8192\n",
        );
        let profiles = parse_target_profiles(&manifest);
        assert_eq!(profiles.len(), 1);
        let p = &profiles["thumbv7em-none-eabihf"];
        assert_eq!(p.features, vec!["no_std", "cortex-m"]);
        assert_eq!(p.opt_level, "s");
        assert_eq!(p.stack_size, Some(8192));
    }

    #[test]
    fn parse_multiple_target_sections() {
        let manifest = make_manifest(
            "[target.thumbv7em-none-eabihf]\n\
             features = [\"no_std\"]\n\
             opt_level = \"s\"\n\
             stack_size = 8192\n\
             \n\
             [target.x86_64-unknown-linux]\n\
             features = [\"std\", \"networking\"]\n\
             opt_level = \"3\"\n",
        );
        let profiles = parse_target_profiles(&manifest);
        assert_eq!(profiles.len(), 2);

        let arm = &profiles["thumbv7em-none-eabihf"];
        assert_eq!(arm.opt_level, "s");
        assert_eq!(arm.stack_size, Some(8192));
        assert_eq!(arm.features, vec!["no_std"]);

        let linux = &profiles["x86_64-unknown-linux"];
        assert_eq!(linux.opt_level, "3");
        assert!(linux.stack_size.is_none());
        assert_eq!(linux.features, vec!["std", "networking"]);
    }

    #[test]
    fn parse_no_target_sections() {
        let manifest = make_manifest(
            "[dependencies]\n\
             foo = \"^1.0.0\"\n",
        );
        let profiles = parse_target_profiles(&manifest);
        assert!(profiles.is_empty());
    }

    #[test]
    fn parse_target_with_cfg_entries() {
        let manifest = make_manifest(
            "[target.riscv32]\n\
             opt_level = \"2\"\n\
             linker = \"riscv32-elf-gcc\"\n",
        );
        let profiles = parse_target_profiles(&manifest);
        let p = &profiles["riscv32"];
        assert_eq!(
            p.cfg.get("linker").map(|s| s.as_str()),
            Some("riscv32-elf-gcc")
        );
        assert_eq!(p.opt_level, "2");
    }

    #[test]
    fn parse_target_unquoted_opt_level_integer() {
        // `opt_level = 2` (no quotes) should parse to "2"
        let manifest = make_manifest("[target.host]\nopt_level = 2\n");
        let profiles = parse_target_profiles(&manifest);
        // The unquoted integer is treated like a non-quoted value; the
        // parser reads the raw token including the integer.
        let p = &profiles["host"];
        assert_eq!(p.opt_level, "2");
    }

    #[test]
    fn resolve_profile_known_triple() {
        let mut profiles = HashMap::new();
        let mut arm_profile = TargetProfile::default_profile();
        arm_profile.opt_level = "s".to_string();
        profiles.insert("thumbv7em-none-eabihf".to_string(), arm_profile.clone());

        let resolved = resolve_profile(&profiles, Some("thumbv7em-none-eabihf"));
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().opt_level, "s");
    }

    #[test]
    fn resolve_profile_unknown_triple_falls_back() {
        let profiles: HashMap<String, TargetProfile> = HashMap::new();
        // Unknown triple → None (caller uses default_profile()).
        let resolved = resolve_profile(&profiles, Some("mips-unknown-linux"));
        assert!(resolved.is_none());
    }

    #[test]
    fn resolve_profile_no_target_returns_none() {
        let profiles: HashMap<String, TargetProfile> = HashMap::new();
        let resolved = resolve_profile(&profiles, None);
        assert!(resolved.is_none());
    }

    #[test]
    fn default_profile_values() {
        let p = TargetProfile::default_profile();
        assert!(p.features.is_empty());
        assert_eq!(p.opt_level, "0");
        assert!(p.stack_size.is_none());
        assert!(p.cfg.is_empty());
    }

    #[test]
    fn valid_opt_levels_accepted() {
        for lvl in &["0", "1", "2", "3", "s"] {
            assert!(is_valid_opt_level(lvl), "expected valid: {}", lvl);
        }
    }

    #[test]
    fn invalid_opt_levels_rejected() {
        for lvl in &["4", "fast", "", "O2", "z"] {
            assert!(!is_valid_opt_level(lvl), "expected invalid: {}", lvl);
        }
    }

    #[test]
    fn check_ok_when_no_manifest_in_tmpdir() {
        let tmp = std::env::temp_dir().join("__resilient_target_profiles_no_manifest.rz");
        std::fs::write(&tmp, b"fn f() {}").unwrap();
        let (prog, _) = crate::parse("fn f() {}");
        let result = check(&prog, tmp.to_str().unwrap());
        assert!(result.is_ok());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn check_valid_manifest_passes() {
        let dir = std::env::temp_dir().join("__resilient_tp_valid");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"[package]
name = "a"
version = "1.0.0"
[target.arm]
opt_level = "s"
stack_size = 4096
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src = dir.join("main.rz");
        std::fs::write(&src, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src.to_str().unwrap());
        assert!(result.is_ok(), "unexpected error: {:?}", result);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_invalid_opt_level_is_error() {
        let dir = std::env::temp_dir().join("__resilient_tp_bad_opt");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"[package]
name = "a"
version = "1.0.0"
[target.arm]
opt_level = "fast"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src = dir.join("main.rz");
        std::fs::write(&src, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src.to_str().unwrap());
        assert!(result.is_err(), "expected error for invalid opt_level");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_invalid_stack_size_is_error() {
        let dir = std::env::temp_dir().join("__resilient_tp_bad_stack");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"[package]
name = "a"
version = "1.0.0"
[target.arm]
stack_size = notanumber
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src = dir.join("main.rz");
        std::fs::write(&src, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src.to_str().unwrap());
        assert!(result.is_err(), "expected error for invalid stack_size");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_rejects_malformed_target_header_shape() {
        let dir = std::env::temp_dir().join("__resilient_tp_bad_header_shape");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"[package]
name = "a"
version = "1.0.0"
                        [target.arm.extra]
opt_level = "s"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src = dir.join("main.rz");
        std::fs::write(&src, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src.to_str().unwrap());
        let err = result.expect_err("expected malformed target header to fail");
        assert!(
            err.contains("malformed target triple `arm.extra`"),
            "unexpected error: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_rejects_missing_target_triple() {
        let dir = std::env::temp_dir().join("__resilient_tp_missing_triple");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"[package]
name = "a"
version = "1.0.0"
                        [target]
opt_level = "s"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src = dir.join("main.rz");
        std::fs::write(&src, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src.to_str().unwrap());
        let err = result.expect_err("expected missing triple to fail");
        assert!(
            err.contains("missing required field `triple`"),
            "unexpected error: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_rejects_empty_target_section() {
        let dir = std::env::temp_dir().join("__resilient_tp_empty_section");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"[package]
name = "a"
version = "1.0.0"
                        [target.arm]
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src = dir.join("main.rz");
        std::fs::write(&src, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src.to_str().unwrap());
        let err = result.expect_err("expected empty target section to fail");
        assert!(
            err.contains("missing required fields"),
            "unexpected error: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_rejects_duplicate_target_sections() {
        let dir = std::env::temp_dir().join("__resilient_tp_dup_section");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"[package]
name = "a"
version = "1.0.0"
                        [target.arm]
opt_level = "s"
                        [target.arm]
stack_size = 4096
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src = dir.join("main.rz");
        std::fs::write(&src, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src.to_str().unwrap());
        let err = result.expect_err("expected duplicate target section to fail");
        assert!(
            err.contains("duplicate `[target.arm]` declaration"),
            "unexpected error: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_rejects_duplicate_fields_in_target_section() {
        let dir = std::env::temp_dir().join("__resilient_tp_dup_field");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = r#"[package]
name = "a"
version = "1.0.0"
                        [target.arm]
opt_level = "s"
opt_level = "3"
"#;
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src = dir.join("main.rz");
        std::fs::write(&src, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src.to_str().unwrap());
        let err = result.expect_err("expected duplicate field to fail");
        assert!(
            err.contains("duplicate `opt_level` field"),
            "unexpected error: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_features_array_empty() {
        let manifest = make_manifest("[target.x]\nfeatures = []\n");
        let profiles = parse_target_profiles(&manifest);
        assert_eq!(profiles["x"].features, Vec::<String>::new());
    }

    #[test]
    fn parse_features_array_single() {
        let manifest = make_manifest("[target.x]\nfeatures = [\"std\"]\n");
        let profiles = parse_target_profiles(&manifest);
        assert_eq!(profiles["x"].features, vec!["std"]);
    }

    #[test]
    fn parse_target_section_does_not_bleed_into_next_section() {
        let manifest =
            make_manifest("[target.arm]\nopt_level = \"s\"\n[dependencies]\nfoo = \"^1.0\"\n");
        let profiles = parse_target_profiles(&manifest);
        // Only one target section; `foo` is not a target profile.
        assert_eq!(profiles.len(), 1);
        let p = &profiles["arm"];
        assert_eq!(p.opt_level, "s");
        // `foo` should NOT appear in cfg.
        assert!(!p.cfg.contains_key("foo"));
    }

    struct ManifestFixture {
        dir: PathBuf,
    }

    impl Drop for ManifestFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn manifest_text(body: &str) -> String {
        format!("{MANIFEST_PREFIX}{body}")
    }

    fn prepare_manifest_fixture(name: &str, body: &str) -> (ManifestFixture, PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("__resilient_tp_regression_{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest_path = dir.join("rz.toml");
        std::fs::write(&manifest_path, manifest_text(body)).unwrap();
        let source_path = dir.join("main.rz");
        std::fs::write(&source_path, b"fn main() {}\n").unwrap();
        (ManifestFixture { dir }, manifest_path, source_path)
    }

    fn expected_error(manifest_path: &Path, line: usize, column: usize, message: &str) -> String {
        format!(
            "{}:{}:{}: error[target-profiles]: {}",
            manifest_path.display(),
            line,
            column,
            message
        )
    }

    fn assert_check_error(case_name: &str, body: &str, line: usize, column: usize, message: &str) {
        let (_fixture, manifest_path, source_path) = prepare_manifest_fixture(case_name, body);
        let (prog, _) = crate::parse("fn main() {}");
        let err = check(&prog, source_path.to_str().unwrap())
            .expect_err("expected target profile validation failure");
        assert_eq!(
            err.trim(),
            expected_error(&manifest_path, line, column, message)
        );
    }

    #[test]
    fn check_rejects_malformed_target_profile_declarations_regression() {
        let cases = [
            (
                "array_of_tables",
                "[[target.thumbv7em-none-eabihf]]\nopt_level = \"s\"\n",
                5,
                1,
                "invalid target profile declaration shape: array-of-tables syntax is not supported",
            ),
            (
                "missing_triple",
                "[target]\nopt_level = \"s\"\n",
                5,
                1,
                "invalid target profile declaration: missing required field `triple`",
            ),
            (
                "bad_triple",
                "[target.thumbv7em-none-eabihf!]\nopt_level = \"s\"\n",
                5,
                1,
                "invalid target profile declaration shape: malformed target triple `thumbv7em-none-eabihf!`",
            ),
            (
                "invalid_opt_level",
                "[target.arm]\nopt_level = \"fast\"\n",
                6,
                1,
                "invalid [target.arm] declaration: `opt_level` `fast`; expected one of: 0, 1, 2, 3, s",
            ),
            (
                "invalid_stack_size",
                "[target.arm]\nstack_size = 0\n",
                6,
                1,
                "invalid [target.arm] declaration: `stack_size` must be a positive integer, got `0`",
            ),
            (
                "invalid_features",
                "[target.arm]\nfeatures = [std]\n",
                6,
                1,
                "invalid [target.arm] declaration: `features` entries must be double-quoted strings",
            ),
        ];

        for (case_name, body, line, column, message) in cases {
            assert_check_error(case_name, body, line, column, message);
        }
    }

    #[test]
    fn check_rejects_duplicate_target_profile_forms_regression() {
        let (_fixture, manifest_path, source_path) = prepare_manifest_fixture(
            "duplicate_sections",
            "[target.arm]\nopt_level = \"s\"\n[target.arm]\nstack_size = 4096\n",
        );
        let (prog, _) = crate::parse("fn main() {}");
        let err = check(&prog, source_path.to_str().unwrap())
            .expect_err("expected duplicate target section to fail");
        let expected_prefix = format!(
            "{}:7:1: error[target-profiles]: invalid target profile combination:",
            manifest_path.display()
        );
        assert!(
            err.trim().starts_with(&expected_prefix),
            "unexpected error: {err}"
        );
        assert!(
            err.contains("duplicate `[target.arm]` declaration"),
            "unexpected error: {err}"
        );
        assert!(
            err.contains("previous declaration at line 5"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn check_accepts_valid_target_profile_baselines_regression() {
        let (_fixture, _manifest_path, source_path) = prepare_manifest_fixture(
            "minimal",
            "[target.arm]\nopt_level = \"s\"\nstack_size = 4096\n",
        );
        let (prog, _) = crate::parse("fn main() {}");
        assert!(check(&prog, source_path.to_str().unwrap()).is_ok());
        let profiles = parse_target_profiles(&manifest_text(
            "[target.arm]\nopt_level = \"s\"\nstack_size = 4096\n",
        ));
        let p = &profiles["arm"];
        assert_eq!(p.opt_level, "s");
        assert_eq!(p.stack_size, Some(4096));
        assert!(p.features.is_empty());
        assert!(p.cfg.is_empty());

        let (_fixture, _manifest_path, source_path) = prepare_manifest_fixture(
            "features_and_cfg",
            "[target.thumbv7em-none-eabihf]\nfeatures = [\"no_std\", \"cortex-m\"]\nopt_level = 3\nstack_size = 8192\nlinker = \"ld.lld\"\n",
        );
        let (prog, _) = crate::parse("fn main() {}");
        assert!(check(&prog, source_path.to_str().unwrap()).is_ok());
        let profiles = parse_target_profiles(&manifest_text(
            "[target.thumbv7em-none-eabihf]\nfeatures = [\"no_std\", \"cortex-m\"]\nopt_level = 3\nstack_size = 8192\nlinker = \"ld.lld\"\n",
        ));
        let p = &profiles["thumbv7em-none-eabihf"];
        assert_eq!(p.features, vec!["no_std", "cortex-m"]);
        assert_eq!(p.opt_level, "3");
        assert_eq!(p.stack_size, Some(8192));
        assert_eq!(p.cfg.get("linker").map(|s| s.as_str()), Some("ld.lld"));

        let (_fixture, _manifest_path, source_path) = prepare_manifest_fixture(
            "multiple_sections",
            "[target.arm]\nopt_level = \"s\"\nstack_size = 4096\n\n[target.x86_64-unknown-linux]\nfeatures = [\"std\", \"networking\"]\nopt_level = \"3\"\n",
        );
        let (prog, _) = crate::parse("fn main() {}");
        assert!(check(&prog, source_path.to_str().unwrap()).is_ok());
        let profiles = parse_target_profiles(&manifest_text(
            "[target.arm]\nopt_level = \"s\"\nstack_size = 4096\n\n[target.x86_64-unknown-linux]\nfeatures = [\"std\", \"networking\"]\nopt_level = \"3\"\n",
        ));
        assert_eq!(profiles.len(), 2);
        let arm = &profiles["arm"];
        assert_eq!(arm.opt_level, "s");
        assert_eq!(arm.stack_size, Some(4096));
        let linux = &profiles["x86_64-unknown-linux"];
        assert_eq!(linux.features, vec!["std", "networking"]);
        assert_eq!(linux.opt_level, "3");
    }
}
