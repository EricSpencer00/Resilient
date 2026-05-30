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

/// Validate `[target.TRIPLE]` sections from the nearest `rz.toml`.
///
/// Called from `typechecker.rs` `<EXTENSION_PASSES>`.  No-op when no
/// manifest is found.  Errors are reported as structured diagnostics.
pub(crate) fn check(_program: &crate::Node, source_path: &str) -> Result<(), String> {
    let source_dir = Path::new(source_path).parent().unwrap_or(Path::new("."));

    let manifest_path = ["rz.toml", "resilient.toml"]
        .iter()
        .map(|name| source_dir.join(name))
        .find(|p| p.exists());

    let manifest_content = match manifest_path {
        Some(ref p) => match std::fs::read_to_string(p) {
            Ok(s) => s,
            Err(_) => return Ok(()),
        },
        None => return Ok(()),
    };

    let profiles = parse_target_profiles(&manifest_content);
    let mut errors: Vec<String> = Vec::new();

    for (triple, profile) in &profiles {
        // Validate opt_level.
        if !profile.opt_level.is_empty() && !is_valid_opt_level(&profile.opt_level) {
            errors.push(format!(
                "{source_path}:0:0: error[target-profiles]: \
                 [target.{triple}] has invalid `opt_level` `{}`; \
                 expected one of: 0, 1, 2, 3, s",
                profile.opt_level
            ));
        }

        // Validate stack_size parsed successfully if a value was present
        // (parse failure means the field was ignored; detect it via raw re-scan).
        for raw_line in manifest_content.lines() {
            let line = raw_line.trim();
            if line.starts_with(&format!("[target.{}]", triple)) {
                break; // stop when we exit this section's re-scan
            }
        }
        // Re-scan for this section's raw stack_size to catch non-integer values.
        let mut in_section = false;
        for raw_line in manifest_content.lines() {
            let l = raw_line.trim();
            if l == format!("[target.{}]", triple) {
                in_section = true;
                continue;
            }
            if in_section {
                if l.starts_with('[') {
                    break;
                }
                if let Some((k, v)) = l.split_once('=') {
                    let (k, v) = (k.trim(), v.trim());
                    let _ = v; // suppress unused_variables until more checks are added
                    if k == "stack_size" && v.parse::<u64>().is_err() {
                        errors.push(format!(
                            "{source_path}:0:0: error[target-profiles]: \
                             [target.{triple}] `stack_size` must be a positive integer, \
                             got `{v}`",
                        ));
                    }
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

// ── Small helpers ───────────────────────────────────────────────────────────

/// Parse a TOML string array: `["a", "b", "c"]`.
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
        let manifest = "[package]\nname = \"a\"\nversion = \"1.0.0\"\n\
                        [target.arm]\nopt_level = \"s\"\nstack_size = 4096\n";
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
        let manifest = "[package]\nname = \"a\"\nversion = \"1.0.0\"\n\
                        [target.arm]\nopt_level = \"fast\"\n";
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
        let manifest = "[package]\nname = \"a\"\nversion = \"1.0.0\"\n\
                        [target.arm]\nstack_size = notanumber\n";
        std::fs::write(dir.join("rz.toml"), manifest).unwrap();
        let src = dir.join("main.rz");
        std::fs::write(&src, b"fn main() {}").unwrap();
        let (prog, _) = crate::parse("fn main() {}");
        let result = check(&prog, src.to_str().unwrap());
        assert!(result.is_err(), "expected error for invalid stack_size");
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
}
