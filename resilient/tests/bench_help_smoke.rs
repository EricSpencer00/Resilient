//! RES-3184: focused help for the `rz bench` subcommand.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn assert_focused_bench_help(args: &[&str]) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn rz bench help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "bench help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "Usage: rz bench <file> [--baseline <git-ref>] [--summary-json <path>] [--warmup N] [--runs N]",
        "Discover and run `bench \"name\" { ... }` blocks.",
        "--summary-json <path>  Write a stable JSON summary artifact",
        "--filter <substr>  Only run benchmarks whose names contain <substr>",
    ] {
        assert!(
            stdout.contains(expected),
            "focused bench help missing {expected:?}; got:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("COMMON FLAGS:"),
        "bench help should not fall through to global help; got:\n{stdout}"
    );
}

#[test]
fn bench_long_help_is_focused() {
    assert_focused_bench_help(&["bench", "--help"]);
}

#[test]
fn bench_short_help_is_focused() {
    assert_focused_bench_help(&["bench", "-h"]);
}

#[test]
fn bench_help_word_is_focused() {
    assert_focused_bench_help(&["bench", "help"]);
}
