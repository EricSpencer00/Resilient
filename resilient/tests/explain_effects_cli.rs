//! RES-347: `--explain-effects` prints the inferred effect
//! (@pure / @io) for every user fn after typechecking.
//!
//! The pipeline under test is `pure_leaf -> pure_middle -> io_caller`:
//! the two callees never touch IO and must be reported `@pure`; the
//! outer fn calls `println` and must be reported `@io`. The
//! inference is a fixpoint over the call graph, so this also
//! exercises transitive propagation: an intermediate fn that only
//! relays a pure callee stays pure.
//!
//! We assert on the deterministic block printed by
//! `print_effect_explanation` — sorted by fn name, no ANSI
//! escapes — so the format is stable enough to grep from tooling.
use std::io::Write;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

fn write_temp(stem: &str, src: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "res347_{}_{}_{}.rz",
        stem,
        std::process::id(),
        // Using the nanoseconds-since-UNIX-epoch defeats parallel test
        // collisions without pulling in a uuid crate.
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let mut f = std::fs::File::create(&path).expect("create temp source");
    f.write_all(src.as_bytes()).expect("write source");
    path
}

/// Strip ANSI CSI sequences so the assertion ignores the
/// colorization `--audit` uses elsewhere. `--explain-effects` itself
/// does not emit colors, but `Type check passed` upstream does.
fn strip_ansi(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            i += 2;
            while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Extract the `--- Effects ---` … `--- End effects ---` block
/// from the driver's stdout so the assertion isn't fragile to
/// unrelated prefix/suffix lines (seed banner, "Program executed
/// successfully", etc.).
fn extract_effects_block(stdout: &str) -> String {
    let plain = strip_ansi(stdout);
    let mut inside = false;
    let mut lines = Vec::new();
    for line in plain.lines() {
        if line == "--- Effects ---" {
            inside = true;
        }
        if inside {
            lines.push(line.to_string());
            if line == "--- End effects ---" {
                break;
            }
        }
    }
    lines.join("\n")
}

#[test]
fn explain_effects_reports_pure_pure_io_pipeline() {
    let src = "\
fn pure_leaf(int x) {\n\
    return x * x;\n\
}\n\
\n\
fn pure_middle(int x) {\n\
    return pure_leaf(x) + 1;\n\
}\n\
\n\
fn io_caller(int x) {\n\
    println(\"calling\");\n\
    return pure_middle(x);\n\
}\n\
\n\
io_caller(3);\n\
";
    let path = write_temp("pipeline", src);

    let output = Command::new(bin())
        .arg("--explain-effects")
        .arg(&path)
        .output()
        .expect("spawn resilient");

    let _ = std::fs::remove_file(&path);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let block = extract_effects_block(&stdout);

    let expected = "\
--- Effects ---\n  \
io_caller: @io\n  \
pure_leaf: @pure\n  \
pure_middle: @pure\n\
--- End effects ---";

    assert_eq!(block, expected, "full stdout=\n{stdout}");
}

#[test]
fn explain_effects_without_user_fns_emits_empty_block() {
    // A one-liner script has no user fns; the flag should still
    // emit the header/footer so tooling never has to special-case
    // the empty case.
    let src = "let x = 1 + 2;\n";
    let path = write_temp("empty", src);

    let output = Command::new(bin())
        .arg("--explain-effects")
        .arg(&path)
        .output()
        .expect("spawn resilient");

    let _ = std::fs::remove_file(&path);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let block = extract_effects_block(&stdout);

    let expected = "--- Effects ---\n--- End effects ---";
    assert_eq!(block, expected, "full stdout=\n{stdout}");
}

#[test]
fn explain_effects_rejects_pure_annotation_on_io_body() {
    // Manual `@pure` on a function that reaches IO is still a
    // type error — the inference pass runs after the purity check,
    // so the existing diagnostic is preserved.
    let src = "\
@pure fn bad(int x) {\n\
    println(\"oops\");\n\
    return x;\n\
}\n\
\n\
bad(1);\n\
";
    let path = write_temp("bad", src);

    let output = Command::new(bin())
        .arg("--explain-effects")
        .arg(&path)
        .output()
        .expect("spawn resilient");

    let _ = std::fs::remove_file(&path);

    assert_ne!(
        output.status.code(),
        Some(0),
        "expected non-zero exit; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("@pure fn `bad`"),
        "expected @pure violation diagnostic; combined output=\n{combined}"
    );
}
