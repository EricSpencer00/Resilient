//! RES-173: smoke test for the `--dump-chunks` driver flag.
//!
//! Spawns the compiled `resilient` binary against a tiny two-
//! function example and asserts the output carries the
//! disassembly's stable shape — section headers, offset/line/op
//! columns, constants block, and peephole-folded ops where
//! applicable. External tools parse this output; the assertions
//! here pin down what they're allowed to depend on.

use std::io::Write;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn write_temp(contents: &str, tag: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("res_173_{}_{}_{}.rs", tag, std::process::id(), n));
    let mut f = std::fs::File::create(&path).expect("create temp");
    f.write_all(contents.as_bytes()).expect("write temp");
    path
}

#[test]
fn dump_chunks_prints_sections_for_main_and_each_function() {
    let src = "\
        fn add_one(int x) { return x + 1; }\n\
        fn triple(int x) { return x * 3; }\n\
        return triple(add_one(4));\n\
    ";
    let path = write_temp(src, "two_fn");
    let output = Command::new(bin())
        .arg("--dump-chunks")
        .arg(&path)
        .output()
        .expect("spawn resilient --dump-chunks");
    let _ = std::fs::remove_file(&path);
    assert_eq!(
        output.status.code(),
        Some(0),
        "--dump-chunks must exit 0; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    // Section headers
    assert!(
        stdout.contains("=== main ==="),
        "missing main section header:\n{}",
        stdout
    );
    assert!(
        stdout.contains("=== fn add_one (arity=1, locals=1) ==="),
        "missing add_one header:\n{}",
        stdout
    );
    assert!(
        stdout.contains("=== fn triple (arity=1, locals=1) ==="),
        "missing triple header:\n{}",
        stdout
    );
    // Constants block shape — both functions have an Int constant
    // (1 and 3 respectively). Render as `const[0] = 1` / `const[0] = 3`.
    assert!(
        stdout.contains("const[0] = 1"),
        "expected `const[0] = 1` from add_one's `+ 1`:\n{}",
        stdout
    );
    assert!(
        stdout.contains("const[0] = 3"),
        "expected `const[0] = 3` from triple's `* 3`:\n{}",
        stdout
    );
    // Call-site annotation: main calls triple and add_one by name.
    assert!(
        stdout.contains("Call") && stdout.contains("-> triple"),
        "expected `Call ... -> triple`:\n{}",
        stdout
    );
    assert!(
        stdout.contains("-> add_one"),
        "expected `-> add_one`:\n{}",
        stdout
    );
}

#[test]
fn dump_chunks_format_columns_match_spec() {
    // Per the ticket: `<offset:04x>  <line>   <OpName> <operands>`.
    // We verify the four-hex offset column and an `L<n>` line
    // column show up on at least one code line. The exact column
    // widths aren't pinned — tools should tolerate 2+ spaces
    // between columns.
    let src = "return 42;\n";
    let path = write_temp(src, "cols");
    let output = Command::new(bin())
        .arg("--dump-chunks")
        .arg(&path)
        .output()
        .expect("spawn resilient --dump-chunks");
    let _ = std::fs::remove_file(&path);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    // At least one line matches the `  XXXX  L<n>  ...` shape.
    let has_offset_line = stdout.lines().any(|l| {
        let trimmed = l.trim_start();
        trimmed
            .split_whitespace()
            .next()
            .and_then(|w| usize::from_str_radix(w, 16).ok())
            .is_some()
            && trimmed.contains(" L")
    });
    assert!(
        has_offset_line,
        "expected at least one code line with hex offset + L<n> column:\n{}",
        stdout
    );
}

#[test]
fn dump_chunks_reflects_peephole_inc_local_fold() {
    // Ticket Note: "reflect the optimized bytecode — that's the
    // version that runs." A while-loop counter is the canonical
    // shape the RES-172 peephole folds into `IncLocal`.
    let src = "\
        fn count_up() {\n\
            let i = 0;\n\
            while i < 3 {\n\
                i = i + 1;\n\
            }\n\
            return i;\n\
        }\n\
        return count_up();\n\
    ";
    let path = write_temp(src, "inc_local");
    let output = Command::new(bin())
        .arg("--dump-chunks")
        .arg(&path)
        .output()
        .expect("spawn resilient --dump-chunks");
    let _ = std::fs::remove_file(&path);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    // The fold replaces LoadLocal + Const + Add + StoreLocal with
    // a single IncLocal. Verify the compiled output shows the
    // fold, not the raw four-op sequence.
    assert!(
        stdout.contains("IncLocal 0"),
        "expected IncLocal fold in disassembly:\n{}",
        stdout
    );
}

#[test]
fn dump_chunks_requires_path_argument() {
    let output = Command::new(bin())
        .arg("--dump-chunks")
        .output()
        .expect("spawn resilient --dump-chunks (no path)");
    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("requires a path argument"),
        "expected missing-path diagnostic, got stderr:\n{}",
        stderr
    );
}
