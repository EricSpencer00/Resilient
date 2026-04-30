//! RES-200: snapshot tests for diagnostic rendering.
//!
//! Each test writes a small canary Resilient program to a scratch
//! file, runs the real `resilient` binary against it (no mocks —
//! the full driver including lex / parse / typecheck / runtime),
//! captures combined `stdout + stderr`, normalizes away run-to-run
//! noise (ANSI color codes, the scratch path), and snapshots the
//! result via `insta::assert_snapshot!`.
//!
//! The snapshot file lives under `tests/snapshots/<test-name>.snap`.
//! First run creates a `.snap.new` pending diff; `cargo insta review`
//! (requires `cargo install cargo-insta`) promotes it to the
//! committed `.snap`. In CI, a drift between committed `.snap` and
//! current output fails the test loudly — `assert_snapshot!` does
//! NOT auto-accept.
//!
//! Keep canary programs tiny (per ticket note: <20 lines each) so a
//! diff reviewer can inspect the snapshot at a glance.
//!
//! To add a new canary:
//! 1. Append a `#[test]` function calling `check_diagnostic(
//!    name, flags, source)`.
//! 2. Run `cargo test --test diagnostics_snapshots` once to create
//!    the `.snap.new` pending file.
//! 3. Run `cargo insta review` (or move the `.snap.new` to `.snap`
//!    manually after inspection).
//! 4. Commit the new `.snap` alongside the test.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

/// Build a unique scratch path inside the OS temp dir. We use a
/// fixed filename (not mktemp-random) so the stem appearing inside
/// diagnostics is predictable — combined with the normalizer below,
/// every run produces the same string.
fn scratch_path(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("res_snap_{}_{}_{}.rs", tag, std::process::id(), n))
}

/// Strip ANSI color escapes (CSI `ESC [ ... m`) from `s`. The
/// driver unconditionally emits color — the snapshot harness strips
/// it so the committed `.snap` stays readable and terminal-agnostic.
///
/// UTF-8 aware: iterates chars, not bytes, so the em-dash (and other
/// multi-byte sequences) in diagnostic messages survive intact. The
/// ESC byte (0x1b) is ASCII-only so we can safely scan for it at
/// the char level.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['
            // Skip until we hit the terminator byte (a char in the
            // final-byte range for CSI: 0x40..=0x7e).
            for term in chars.by_ref() {
                if matches!(term, '\u{40}'..='\u{7e}') {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Run `resilient <flags> <source-file>` and return the normalized
/// output. Stdout and stderr are merged (in that order) so the
/// snapshot shows the whole user-visible transcript. The scratch
/// path is rewritten to `<tmp>.rs` so the snapshot is stable
/// across machines.
fn run_capture(source: &str, flags: &[&str], tag: &str) -> String {
    let path = scratch_path(tag);
    std::fs::write(&path, source).expect("write scratch source");

    let output = Command::new(bin())
        .args(flags)
        .arg(&path)
        .output()
        .expect("spawn resilient binary");

    // Clean up scratch file before snapshotting — if the assertion
    // panics we don't want to leak files.
    let _ = std::fs::remove_file(&path);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);

    // Normalize:
    // 1. Strip ANSI color escapes.
    // 2. Replace the scratch path with `<tmp>.rs` so the snapshot
    //    is machine-independent.
    let stripped = strip_ansi(&combined);
    let path_str = path.to_string_lossy().to_string();
    stripped.replace(&path_str, "<tmp>.rs")
}

/// Snapshot-test helper used by every canary. Pins a small set of
/// filters on top of whatever the caller's source happens to
/// contain:
/// - strips ANSI (handled above, but also defensive against stray
///   `\x1b` in source);
/// - normalizes any remaining absolute-path-to-scratch fragments;
/// - pins the `seed=<N>` stderr line from the RNG initializer.
fn check_diagnostic(name: &str, flags: &[&str], source: &str) {
    let output = run_capture(source, flags, name);
    // Use insta's settings to attach a `description` listing the
    // source so a reviewer sees WHAT produced the snapshot without
    // hunting for the test's source.
    let mut settings = insta::Settings::new();
    // Line-by-line filter: `seed=<N>` → `seed=<REDACTED>` (only
    // emitted when the user doesn't pass `--seed`; all our tests
    // pass `--seed 0`, but belt-and-suspenders).
    settings.add_filter(r"seed=\d+", "seed=<REDACTED>");
    // A filter for any leftover `/tmp/` absolute paths.
    settings.add_filter(r"/tmp/res_snap_[^ :\n]+\.rs", "<tmp>.rs");
    // And for the Windows-style case in case a reviewer runs on
    // Windows or a different tmp root.
    settings.add_filter(r"/var/folders/[^ :\n]+\.rs", "<tmp>.rs");
    settings.set_snapshot_suffix(name);
    settings.set_description(source);
    settings.bind(|| {
        insta::assert_snapshot!(name, output);
    });
}

// ---------- parser-side canaries ----------

#[test]
fn parser_missing_equals_in_let() {
    let src = "fn main(int _d) { let x 1; return 0; } main(0);\n";
    check_diagnostic("parser_missing_equals_in_let", &["--seed", "0"], src);
}

#[test]
fn parser_unexpected_after_fn_name() {
    // Identifier expected; an int literal is a parser recovery
    // point in Resilient's hand-rolled parser.
    let src = "fn 42() { return 0; } main(0);\n";
    check_diagnostic("parser_unexpected_after_fn_name", &["--seed", "0"], src);
}

#[test]
fn parser_match_missing_fat_arrow() {
    let src = "fn main(int _d) { return match 1 { 1 2 }; } main(0);\n";
    check_diagnostic("parser_match_missing_fat_arrow", &["--seed", "0"], src);
}

#[test]
fn parser_assert_missing_paren() {
    // `assert false;` without parens — parser expects `(`.
    let src = "fn main(int _d) {\n    assert false;\n    return 0;\n}\nmain(0);\n";
    check_diagnostic("parser_assert_missing_paren", &["--seed", "0"], src);
}

#[test]
fn parser_let_missing_identifier() {
    // `let = 1;` — no name between `let` and `=`.
    let src = "fn main(int _d) { let = 1; return 0; } main(0);\n";
    check_diagnostic("parser_let_missing_identifier", &["--seed", "0"], src);
}

// ---------- typechecker canaries (require `-t`) ----------

#[test]
fn typecheck_let_int_assigned_string() {
    let src = "fn main(int _d) {\n    let bad: int = \"hi\";\n    return 0;\n}\n";
    check_diagnostic(
        "typecheck_let_int_assigned_string",
        &["-t", "--seed", "0"],
        src,
    );
}

#[test]
fn typecheck_undefined_variable() {
    let src = "fn main(int _d) {\n    return x;\n}\nmain(0);\n";
    check_diagnostic("typecheck_undefined_variable", &["-t", "--seed", "0"], src);
}

#[test]
fn typecheck_call_arity_mismatch() {
    let src = "fn add(int a, int b) {\n    return a + b;\n}\nfn main(int _d) {\n    return add(1);\n}\nmain(0);\n";
    check_diagnostic("typecheck_call_arity_mismatch", &["-t", "--seed", "0"], src);
}

#[test]
fn typecheck_if_condition_nonbool() {
    let src = "fn main(int _d) {\n    if 1 { return 0; }\n    return 0;\n}\nmain(0);\n";
    check_diagnostic(
        "typecheck_if_condition_nonbool",
        &["-t", "--seed", "0"],
        src,
    );
}

#[test]
fn typecheck_binop_array_plus_int() {
    let src = "fn main(int _d) {\n    let xs = [1, 2, 3];\n    return xs + 1;\n}\nmain(0);\n";
    check_diagnostic(
        "typecheck_binop_array_plus_int",
        &["-t", "--seed", "0"],
        src,
    );
}

// ---------- runtime canaries (default run) ----------

#[test]
fn runtime_division_by_zero() {
    let src = "fn main(int _d) { return 10 / 0; } main(0);\n";
    check_diagnostic("runtime_division_by_zero", &["--seed", "0"], src);
}

#[test]
fn runtime_array_out_of_bounds() {
    let src = "fn main(int _d) {\n    let xs = [1, 2, 3];\n    return xs[10];\n}\nmain(0);\n";
    check_diagnostic("runtime_array_out_of_bounds", &["--seed", "0"], src);
}

#[test]
fn runtime_assert_false() {
    let src = "fn main(int _d) {\n    assert(false);\n    return 0;\n}\nmain(0);\n";
    check_diagnostic("runtime_assert_false", &["--seed", "0"], src);
}

#[test]
fn runtime_unwrap_on_err() {
    let src = "fn main(int _d) {\n    let x = unwrap(Err(\"boom\"));\n    return 0;\n}\nmain(0);\n";
    check_diagnostic("runtime_unwrap_on_err", &["--seed", "0"], src);
}

#[test]
fn runtime_contract_violation() {
    // RES-035 contract: `requires a > 0` fails when called with 0.
    let src = "fn add(int a, int b) requires a > 0 {\n    return a + b;\n}\nfn main(int _d) { return add(0, 1); } main(0);\n";
    check_diagnostic("runtime_contract_violation", &["--seed", "0"], src);
}

#[test]
fn runtime_unknown_identifier() {
    // Identifier not bound at all — separate from the typecheck
    // path because the interpreter has its own env-lookup error
    // wording.
    let src = "fn main(int _d) {\n    return frobnicate(1);\n}\nmain(0);\n";
    check_diagnostic("runtime_unknown_identifier", &["--seed", "0"], src);
}

#[test]
fn runtime_unknown_identifier_builtin_typo() {
    // RES-487: when the misspelled name is within Levenshtein
    // distance 2 of a known builtin, the runtime "Identifier not
    // found" diagnostic must append a "did you mean" hint —
    // mirroring the typechecker's variable-suggestion path
    // (RES-306). `array_revrese` is a 1-edit transposition of
    // `array_reverse`, so the suggester must propose it.
    let src =
        "fn main(int _d) {\n    let x = array_revrese([1, 2, 3]);\n    return 0;\n}\nmain(0);\n";
    check_diagnostic(
        "runtime_unknown_identifier_builtin_typo",
        &["--seed", "0"],
        src,
    );
}
