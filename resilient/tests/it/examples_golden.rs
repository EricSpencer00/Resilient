//! Golden-file tests for example programs.
//!
//! For every `examples/<name>.rz` that has a sibling
//! `examples/<name>.expected.txt`, this test runs the compiled
//! `resilient` binary against it and asserts that combined stdout
//! (plus the CLI's trailing "Program executed successfully" line)
//! matches the expected file byte-for-byte after trimming trailing
//! whitespace.
//!
//! Examples without a sibling expected-file must be accounted for:
//! either a sibling `.interactive` marker (RES-144), or an entry in
//! [`INTENTIONALLY_UNGOLDENED`] with a one-line rationale. The
//! `missing_expected_files_are_intentional` test enforces that every
//! other example has a golden sidecar, so new examples cannot silently
//! drop out of CI.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Examples that deliberately lack a `.expected.txt` sidecar and are
/// not marked `.interactive`. Each entry needs a one-line comment
/// saying why; the audit test fails if a name here no longer exists on
/// disk or has gained a sidecar (stale allowlist entry).
const INTENTIONALLY_UNGOLDENED: &[&str] = &[
    // Documented in-file: expected to fail typechecking/compilation.
    // Asserted by a dedicated #[test] in tests/it/examples_smoke.rs
    // (non-zero exit + "Undefined variable 'hidden'" diagnostic), not
    // by the golden-success harness.
    "visibility_boundary_neg.rz",
];

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples")
}

fn list_examples() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(examples_dir())
        .expect("reading examples dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("rz"))
        .collect();
    out.sort();
    out
}

/// RES-1186: prefer a platform-specific sibling
/// `<stem>.expected.<os>.txt` (`os` = `std::env::consts::OS`, e.g.
/// `macos`, `linux`, `windows`) when present; fall back to the default
/// `<stem>.expected.txt`. Lets a single example carry an OS-specific
/// override for cases where the underlying libc rounds float printing
/// differently (Apple Silicon libsystem_m vs Linux glibc both compute
/// `f64::exp_m1(1.0)`'s shortest round-trip decimal at a different ULP,
/// for instance — see `precision_math.expected.macos.txt`).
fn expected_path(example: &Path) -> PathBuf {
    let stem = example.file_stem().and_then(|s| s.to_str()).unwrap();
    let platform_specific =
        example.with_file_name(format!("{stem}.expected.{}.txt", std::env::consts::OS));
    if platform_specific.exists() {
        return platform_specific;
    }
    example.with_file_name(format!("{stem}.expected.txt"))
}

/// RES-144: a sibling `<stem>.interactive` file marks an example as
/// "don't run in CI" — typically because it reads from real stdin and
/// would block forever, or it's a demo whose behaviour depends on
/// runtime input. The golden harness skips such examples; the
/// missing-expected-file audit also treats them as intentional.
fn is_interactive(example: &Path) -> bool {
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

/// Every non-interactive example must have a golden sidecar, or be
/// named in [`INTENTIONALLY_UNGOLDENED`] with a reason. Also asserts
/// the allowlist is not stale: every entry must still exist and still
/// lack a sidecar.
#[test]
fn missing_expected_files_are_intentional() {
    let allow: HashSet<&str> = INTENTIONALLY_UNGOLDENED.iter().copied().collect();
    let examples = list_examples();
    let on_disk: HashSet<String> = examples
        .iter()
        .filter_map(|p| p.file_name().and_then(|s| s.to_str()).map(str::to_owned))
        .collect();

    let mut stale = Vec::new();
    for name in INTENTIONALLY_UNGOLDENED {
        if !on_disk.contains(*name) {
            stale.push(format!(
                "{name}: listed in INTENTIONALLY_UNGOLDENED but not present under examples/"
            ));
            continue;
        }
        let path = examples_dir().join(name);
        if expected_path(&path).exists() {
            stale.push(format!(
                "{name}: has a .expected.txt sidecar now — remove it from INTENTIONALLY_UNGOLDENED"
            ));
        }
        if is_interactive(&path) {
            stale.push(format!(
                "{name}: marked .interactive — remove it from INTENTIONALLY_UNGOLDENED"
            ));
        }
    }
    assert!(
        stale.is_empty(),
        "stale INTENTIONALLY_UNGOLDENED entries:\n  {}",
        stale.join("\n  ")
    );

    let mut unaccounted = Vec::new();
    for example in &examples {
        if is_interactive(example) || expected_path(example).exists() {
            continue;
        }
        let name = example
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("<unknown>");
        if allow.contains(name) {
            continue;
        }
        unaccounted.push(name.to_owned());
    }

    assert!(
        unaccounted.is_empty(),
        "{} example(s) have no .expected.txt sidecar and are not allowlisted.\n\
         Resolve each by (1) adding a sibling .expected.txt golden file, \
         (2) adding a sibling .interactive marker (stdin-driven / non-CI demos), \
         or (3) adding the name to INTENTIONALLY_UNGOLDENED in examples_golden.rs \
         with a one-line comment explaining why:\n  {}",
        unaccounted.len(),
        unaccounted.join("\n  ")
    );
}
