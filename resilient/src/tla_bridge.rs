//! TLA+ v2.0 bridge — `rz tla check <file.tla>`.
//!
//! Shells out to TLC (the TLA+ model checker) via `java -jar tla2tools.jar`
//! and surfaces results in Resilient's diagnostic format.
//!
//! # Discovery order for `tla2tools.jar`
//!
//! 1. `--tlc-jar <path>` CLI flag.
//! 2. `RESILIENT_TLC_JAR` environment variable.
//! 3. `tlc.jar` / `tla2tools.jar` anywhere on `PATH`.
//!
//! Without Java or `tla2tools.jar` the command prints a clear
//! "not available" message and exits non-zero — it never panics.
//!
//! # Output format
//!
//! TLC output is parsed into Resilient diagnostics:
//!
//! ```text
//! tla:1:0: error: Invariant Inv violated.
//! tla:0:0: info: Model checking completed — no errors found.
//! ```

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

// ── Result types ────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub enum TlaOutcome {
    /// TLC completed without finding any violations.
    Clean,
    /// TLC found at least one violation; diagnostics are populated.
    Violated,
    /// TLA+ file has syntax errors; diagnostics are populated.
    ParseError,
}

#[derive(Debug)]
pub struct TlaCheckResult {
    pub outcome: TlaOutcome,
    /// Human-readable diagnostics in Resilient `file:line:col: severity: msg` format.
    pub diagnostics: Vec<String>,
    /// Raw TLC stdout for `--verbose` consumers.
    pub raw_output: String,
}

// ── Jar discovery ────────────────────────────────────────────────────────────

/// Resolve the path to `tla2tools.jar`.
///
/// Returns `None` when the jar cannot be found anywhere.
pub fn find_tlc_jar(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }

    if let Ok(env_path) = std::env::var("RESILIENT_TLC_JAR") {
        let pb = PathBuf::from(&env_path);
        if pb.exists() {
            return Some(pb);
        }
    }

    // Search PATH for `tlc.jar` or `tla2tools.jar`.
    let candidates = ["tlc.jar", "tla2tools.jar"];
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for &name in &candidates {
                let candidate = dir.join(name);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }

    None
}

/// Returns true if `java` is available on PATH.
pub fn java_available() -> bool {
    Command::new("java")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── TLC output parser ────────────────────────────────────────────────────────

/// Parse TLC's stdout/stderr into Resilient-format diagnostics.
///
/// TLC emits lines like:
/// ```text
/// TLC2 Version 2.16 of 31 December 2021 (rev: ...)
/// @!@!@STARTMSG 2189:0 @!@!@
/// Model checking completed. No error has been found.
/// @!@!@ENDMSG 2189 @!@!@
/// @!@!@STARTMSG 2121:1 @!@!@
/// Invariant Inv is violated.
/// @!@!@ENDMSG 2121 @!@!@
/// Error: The invariant ...
/// ```
///
/// We parse the structured `@!@!@STARTMSG` markers when present and
/// fall back to keyword scanning otherwise.
pub fn parse_tlc_output(output: &str, tla_file: &str) -> (TlaOutcome, Vec<String>) {
    let mut diagnostics: Vec<String> = Vec::new();
    let mut outcome = TlaOutcome::Clean;

    // Fast keyword scan (works with and without structured markers).
    for line in output.lines() {
        let l = line.trim();

        if l.contains("No error has been found")
            || l.contains("Model checking completed. No error")
            || l.contains("Finished in")
        {
            // keep outcome = Clean unless a violation was already flagged
            continue;
        }

        if l.contains("is violated") || l.contains("Invariant") && l.contains("violat") {
            outcome = TlaOutcome::Violated;
            let msg = extract_violation_name(l);
            diagnostics.push(format!("{}:0:0: error: {}", tla_file, msg));
            continue;
        }

        if l.contains("Deadlock reached") {
            outcome = TlaOutcome::Violated;
            diagnostics.push(format!(
                "{}:0:0: error: Deadlock reached (no enabled actions)",
                tla_file
            ));
            continue;
        }

        if l.starts_with("Error:") || l.starts_with("TLC threw an unexpected exception") {
            outcome = TlaOutcome::Violated;
            diagnostics.push(format!("{}:0:0: error: {}", tla_file, l));
            continue;
        }

        if l.contains("Parsing error") || l.contains("Was expecting") {
            outcome = TlaOutcome::ParseError;
            diagnostics.push(format!("{}:0:0: error: {}", tla_file, l));
            continue;
        }

        // Counterexample state lines: "State N:" headers.
        if let Some(rest) = l.strip_prefix("State ") {
            if rest.ends_with(':') || rest.contains(": <") {
                diagnostics.push(format!("{}:0:0: note: {}", tla_file, l));
                continue;
            }
        }
    }

    if diagnostics.is_empty() && outcome == TlaOutcome::Clean {
        diagnostics.push(format!(
            "{}:0:0: info: Model checking completed — no errors found.",
            tla_file
        ));
    }

    (outcome, diagnostics)
}

fn extract_violation_name(line: &str) -> &str {
    // Common prefix: "Invariant X is violated." — return the whole sentence.
    line.trim()
}

// ── Driver ───────────────────────────────────────────────────────────────────

/// Run TLC on `tla_path` and return structured results.
///
/// `tlc_jar_override` is the value of `--tlc-jar`; pass `None` to use
/// auto-discovery.
pub fn check_tla_file(tla_path: &Path, tlc_jar_override: Option<&str>) -> TlaCheckResult {
    let tla_name = tla_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("spec.tla");

    let jar = match find_tlc_jar(tlc_jar_override) {
        Some(j) => j,
        None => {
            return TlaCheckResult {
                outcome: TlaOutcome::ParseError,
                diagnostics: vec![
                    "tla:0:0: error: tla2tools.jar not found. \
                     Set RESILIENT_TLC_JAR or pass --tlc-jar <path>."
                        .into(),
                ],
                raw_output: String::new(),
            };
        }
    };

    if !java_available() {
        return TlaCheckResult {
            outcome: TlaOutcome::ParseError,
            diagnostics: vec![
                "tla:0:0: error: `java` not found on PATH. \
                 TLC requires a JVM to run."
                    .into(),
            ],
            raw_output: String::new(),
        };
    }

    let output = Command::new("java")
        .args([
            "-cp",
            jar.to_str().unwrap_or("tla2tools.jar"),
            "tlc2.TLC",
            "-noGenerateSpecTE",
            "-workers",
            "auto",
            tla_path.to_str().unwrap_or(tla_name),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    match output {
        Err(e) => TlaCheckResult {
            outcome: TlaOutcome::ParseError,
            diagnostics: vec![format!("tla:0:0: error: failed to launch TLC: {}", e)],
            raw_output: String::new(),
        },
        Ok(out) => {
            let combined = format!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            let (outcome, diagnostics) = parse_tlc_output(&combined, tla_name);
            TlaCheckResult {
                outcome,
                diagnostics,
                raw_output: combined,
            }
        }
    }
}

// ── CLI subcommand dispatcher ────────────────────────────────────────────────

/// Handles `rz tla <verb> [args...]`.
///
/// Returns `Some(exit_code)` when the `tla` subcommand was recognised,
/// `None` to fall through to the normal compiler driver.
pub fn dispatch_tla_subcommand(args: &[String]) -> Option<i32> {
    // Must start with `tla`.
    let first = args.first()?;
    if first != "tla" {
        return None;
    }

    let verb = args.get(1).map(String::as_str).unwrap_or("--help");

    match verb {
        "check" => Some(run_tla_check(&args[2..])),
        "--help" | "-h" | "help" => {
            print_tla_help();
            Some(0)
        }
        other => {
            eprintln!(
                "Error: unknown `tla` subcommand `{}`. Try `rz tla --help`.",
                other
            );
            Some(1)
        }
    }
}

fn run_tla_check(args: &[String]) -> i32 {
    // Trivial flag parser: --tlc-jar PATH, --verbose, positional = file.
    let mut tla_file: Option<&str> = None;
    let mut tlc_jar: Option<&str> = None;
    let mut verbose = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--tlc-jar" => {
                i += 1;
                tlc_jar = args.get(i).map(String::as_str);
            }
            "--verbose" => verbose = true,
            f if !f.starts_with('-') => tla_file = Some(f),
            flag => {
                eprintln!("Error: unknown flag `{}`. Try `rz tla check --help`.", flag);
                return 1;
            }
        }
        i += 1;
    }

    let file = match tla_file {
        Some(f) => f,
        None => {
            eprintln!("Error: `rz tla check` requires a .tla file path.");
            eprintln!("Usage: rz tla check [--tlc-jar PATH] [--verbose] <file.tla>");
            return 1;
        }
    };

    let path = Path::new(file);
    if !path.exists() {
        eprintln!("Error: file not found: {}", file);
        return 1;
    }

    let result = check_tla_file(path, tlc_jar);

    if verbose && !result.raw_output.is_empty() {
        println!("=== TLC raw output ===");
        println!("{}", result.raw_output.trim());
        println!("=== end TLC output ===");
    }

    for diag in &result.diagnostics {
        println!("{}", diag);
    }

    match result.outcome {
        TlaOutcome::Clean => 0,
        TlaOutcome::Violated | TlaOutcome::ParseError => 1,
    }
}

fn print_tla_help() {
    println!(
        "rz tla — TLA+ model checking integration

USAGE:
    rz tla check [OPTIONS] <file.tla>

OPTIONS:
    --tlc-jar PATH   Path to tla2tools.jar (overrides RESILIENT_TLC_JAR env var)
    --verbose        Print raw TLC output before diagnostics

DISCOVERY ORDER for tla2tools.jar:
    1. --tlc-jar PATH
    2. RESILIENT_TLC_JAR environment variable
    3. tlc.jar / tla2tools.jar anywhere on PATH

EXAMPLES:
    rz tla check MySpec.tla
    RESILIENT_TLC_JAR=/opt/tla2tools.jar rz tla check MySpec.tla
    rz tla check --tlc-jar /opt/tla2tools.jar --verbose MySpec.tla

OUTPUT FORMAT:
    file.tla:line:col: error: <message>
    file.tla:0:0:     info:  Model checking completed — no errors found.

EXIT CODES:
    0 — no violations found
    1 — violation found, parse error, or TLC unavailable"
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_tlc_output ─────────────────────────────────────────────────────

    #[test]
    fn clean_run_emits_info_diagnostic() {
        let output = "TLC2 Version 2.16\n\
                      Model checking completed. No error has been found.\n\
                      Finished in 00s at (2024-01-01)";
        let (outcome, diags) = parse_tlc_output(output, "Spec.tla");
        assert_eq!(outcome, TlaOutcome::Clean);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].contains("no errors found"), "got: {}", diags[0]);
    }

    #[test]
    fn violated_invariant_is_detected() {
        let output = "Invariant Inv is violated.\nState 1: <Init>";
        let (outcome, diags) = parse_tlc_output(output, "Spec.tla");
        assert_eq!(outcome, TlaOutcome::Violated);
        assert!(diags.iter().any(|d| d.contains("error")));
    }

    #[test]
    fn deadlock_is_detected() {
        let output = "Deadlock reached";
        let (outcome, diags) = parse_tlc_output(output, "Spec.tla");
        assert_eq!(outcome, TlaOutcome::Violated);
        assert!(diags.iter().any(|d| d.contains("Deadlock")));
    }

    #[test]
    fn parsing_error_is_detected() {
        let output = "Parsing error in Spec.tla\nWas expecting 'END'";
        let (outcome, _diags) = parse_tlc_output(output, "Spec.tla");
        assert_eq!(outcome, TlaOutcome::ParseError);
    }

    #[test]
    fn generic_error_lines_are_captured() {
        let output = "Error: The invariant expression is not well-formed";
        let (outcome, diags) = parse_tlc_output(output, "Spec.tla");
        assert_eq!(outcome, TlaOutcome::Violated);
        assert!(diags.iter().any(|d| d.contains("error")));
    }

    #[test]
    fn diagnostics_include_filename() {
        let output = "Invariant Safety is violated.";
        let (_, diags) = parse_tlc_output(output, "MySpec.tla");
        assert!(
            diags.iter().all(|d| d.starts_with("MySpec.tla:")),
            "expected filename prefix, got: {:?}",
            diags
        );
    }

    // ── dispatch_tla_subcommand ──────────────────────────────────────────────

    #[test]
    fn non_tla_arg_returns_none() {
        let args: Vec<String> = vec!["check".into(), "foo.rz".into()];
        assert!(dispatch_tla_subcommand(&args).is_none());
    }

    #[test]
    fn tla_help_returns_zero() {
        let args: Vec<String> = vec!["tla".into(), "--help".into()];
        assert_eq!(dispatch_tla_subcommand(&args), Some(0));
    }

    #[test]
    fn tla_unknown_verb_returns_one() {
        let args: Vec<String> = vec!["tla".into(), "frobnicate".into()];
        assert_eq!(dispatch_tla_subcommand(&args), Some(1));
    }

    #[test]
    fn tla_check_missing_file_returns_one() {
        let args: Vec<String> = vec!["tla".into(), "check".into()];
        assert_eq!(dispatch_tla_subcommand(&args), Some(1));
    }

    #[test]
    fn tla_check_nonexistent_file_returns_one() {
        let args: Vec<String> = vec!["tla".into(), "check".into(), "/nonexistent/Spec.tla".into()];
        assert_eq!(dispatch_tla_subcommand(&args), Some(1));
    }

    // ── find_tlc_jar ─────────────────────────────────────────────────────────

    #[test]
    fn explicit_nonexistent_jar_returns_none() {
        assert!(find_tlc_jar(Some("/nonexistent/tla2tools.jar")).is_none());
    }

    #[test]
    fn explicit_existing_jar_is_returned() {
        // Create a temp file that acts as a stand-in jar.
        let tmp = std::env::temp_dir().join("__resilient_test_tla2tools.jar");
        std::fs::write(&tmp, b"fake jar").unwrap();
        let found = find_tlc_jar(Some(tmp.to_str().unwrap()));
        assert!(found.is_some());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn check_tla_file_without_jar_returns_error_result() {
        // With no jar available (controlled env), outcome should be ParseError.
        // We can't guarantee the CI box lacks tla2tools.jar, but we can
        // call with an explicit nonexistent jar and verify the error.
        let tmp = std::env::temp_dir().join("__resilient_test_Spec.tla");
        std::fs::write(&tmp, "---- MODULE Spec ----\nINIT TRUE\nNEXT TRUE\n====\n").unwrap();
        let result = check_tla_file(&tmp, Some("/nonexistent/tla2tools.jar"));
        assert_eq!(result.outcome, TlaOutcome::ParseError);
        assert!(result.diagnostics[0].contains("error"));
        let _ = std::fs::remove_file(&tmp);
    }
}
