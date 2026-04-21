//! Golden-file tests for example programs.
//!
//! For every `examples/<name>.res` that has a sibling
//! `examples/<name>.expected.txt`, this test runs the compiled
//! `resilient` binary against it and asserts that combined stdout
//! (plus the CLI's trailing "Program executed successfully" line)
//! matches the expected file byte-for-byte after trimming trailing
//! whitespace.
//!
//! Examples without a sibling expected-file are skipped and named in
//! the failure output of `missing_expected_files_are_intentional` —
//! that test is itself ignored, so missing files don't break CI; they
//! simply show up as a line under `cargo test -- --ignored` for the
//! manager to triage.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
}

fn list_examples() -> Vec<PathBuf> {
    let dir = examples_dir();
    let entries: Vec<_> = fs::read_dir(&dir)
        .expect("reading examples dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();

    // Top-level `<name>.res` files.
    let mut out: Vec<PathBuf> = entries
        .iter()
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("res"))
        .cloned()
        .collect();

    // RES-272: also include `<subdir>/main.res` for each subdirectory that
    // has a sibling `<subdir>.expected.txt` at the top level. This covers
    // multi-file examples (e.g. `imports_demo/`) that `list_examples` would
    // otherwise silently skip because directories have no `.res` extension.
    for entry in &entries {
        if entry.is_dir() {
            let main_res = entry.join("main.res");
            if main_res.exists() {
                let dir_name = entry.file_name().and_then(|s| s.to_str()).unwrap_or("");
                let expected = dir.join(format!("{dir_name}.expected.txt"));
                if expected.exists() {
                    out.push(main_res);
                }
            }
        }
    }

    out.sort();
    out
}

fn expected_path(example: &Path) -> PathBuf {
    // RES-272: for multi-file examples (`<dir>/main.res`), the golden sidecar
    // lives at `<parent>/<dir>.expected.txt` rather than inside the subdir.
    let file_name = example.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if file_name == "main.res"
        && let Some(parent) = example.parent()
        && let Some(dir_name) = parent.file_name().and_then(|s| s.to_str())
        && let Some(grandparent) = parent.parent()
    {
        return grandparent.join(format!("{dir_name}.expected.txt"));
    }
    let stem = example.file_stem().and_then(|s| s.to_str()).unwrap();
    example.with_file_name(format!("{stem}.expected.txt"))
}

/// RES-144: a sibling `<stem>.interactive` file marks an example as
/// "don't run in CI" — typically because it reads from real stdin and
/// would block forever, or it's a demo whose behaviour depends on
/// runtime input. The golden harness skips such examples; the
/// missing-expected-file audit also treats them as intentional.
fn is_interactive(example: &Path) -> bool {
    // RES-272: for multi-file examples (`<dir>/main.res`), the interactive
    // marker lives at `<parent>/<dir>.interactive` alongside the golden sidecar.
    let file_name = example.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if file_name == "main.res"
        && let Some(parent) = example.parent()
        && let Some(dir_name) = parent.file_name().and_then(|s| s.to_str())
        && let Some(grandparent) = parent.parent()
    {
        return grandparent.join(format!("{dir_name}.interactive")).exists();
    }
    let stem = example.file_stem().and_then(|s| s.to_str()).unwrap();
    example
        .with_file_name(format!("{stem}.interactive"))
        .exists()
}

fn run(example: &Path) -> String {
    let output = Command::new(bin())
        .arg(example)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("failed to spawn resilient binary");
    // Most examples are expected to succeed, in which case the binary
    // prints stdout then appends its own "Program executed successfully".
    // The expected file captures exactly that combined output so it stays
    // truthful to what a user sees.
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn normalize(s: &str) -> String {
    s.trim_end_matches(&['\n', '\r'][..])
        .lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn golden_outputs_match() {
    let mut checked = 0usize;
    let mut failures = Vec::new();

    for example in list_examples() {
        // RES-144: skip examples tagged as interactive (stdin-driven
        // demos or live-clock experiments) — they have no reproducible
        // stdout to match against.
        if is_interactive(&example) {
            continue;
        }
        let expected_file = expected_path(&example);
        if !expected_file.exists() {
            continue;
        }
        checked += 1;

        let expected = fs::read_to_string(&expected_file)
            .unwrap_or_else(|e| panic!("reading {}: {}", expected_file.display(), e));
        let actual = run(&example);

        let (e, a) = (normalize(&expected), normalize(&actual));
        if e != a {
            failures.push(format!(
                "--- {}\n  expected:\n{}\n  actual:\n{}",
                example.display(),
                e,
                a
            ));
        }
    }

    assert!(
        checked > 0,
        "no examples had .expected.txt sidecars — at least hello/minimal should"
    );
    assert!(
        failures.is_empty(),
        "{} of {} golden files mismatched:\n{}",
        failures.len(),
        checked,
        failures.join("\n\n")
    );
}

/// Report which examples lack an `.expected.txt` sibling. Ignored by
/// default so CI stays green, but surfaces work for the manager to
/// triage:
///
///     cargo test -- --ignored missing_expected_files
#[test]
#[ignore]
fn missing_expected_files_are_intentional() {
    let missing: Vec<_> = list_examples()
        .into_iter()
        // RES-144: interactive examples intentionally have no
        // `.expected.txt` — they're exempt from the audit.
        .filter(|p| !is_interactive(p) && !expected_path(p).exists())
        .collect();
    if !missing.is_empty() {
        let names: Vec<_> = missing
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        panic!(
            "{} example(s) have no .expected.txt sidecar:\n  {}",
            names.len(),
            names.join("\n  ")
        );
    }
}
