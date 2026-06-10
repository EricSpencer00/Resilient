//! RES-3167: focused help for certificate verification subcommands.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn assert_focused_help(args: &[&str], expected: &[&str]) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn rz verification help");

    assert_eq!(
        output.status.code(),
        Some(0),
        "verification help should exit 0; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for text in expected {
        assert!(
            stdout.contains(text),
            "focused verification help missing {text:?}; got:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("COMMON FLAGS:"),
        "verification help should not fall through to global help; got:\n{stdout}"
    );
}

fn assert_verify_cert_help(args: &[&str]) {
    assert_focused_help(
        args,
        &[
            "rz verify-cert — verify a signed certificate directory",
            "USAGE:\n    rz verify-cert <dir> [--pubkey <path>]",
            "backend-limited; requires --features z3",
            "Reads cert.sig and every .smt2 file in the directory.",
            "Verifies the directory signature with the embedded key or --pubkey.",
            "--pubkey PATH    Verify with a PEM public key instead of the embedded key",
            "rz verify-cert certs/",
            "Run `rz --help` for global flags and other subcommands.",
        ],
    );
}

fn assert_verify_all_help(args: &[&str]) {
    assert_focused_help(
        args,
        &[
            "rz verify-all — re-check every obligation in a manifest",
            "USAGE:\n    rz verify-all <dir> [--pubkey <path>] [--z3]",
            "backend-limited; requires --features z3",
            "Reads manifest.json, checks certificate hashes, and verifies signatures.",
            "With --z3, also runs `z3 -smt2` for solver re-verification when z3 is on PATH.",
            "--pubkey PATH    Verify signatures with a PEM public key override",
            "--z3             Re-run Z3 on each certificate when the z3 binary is available",
            "rz verify-all certs/ --z3",
            "Run `rz --help` for global flags and other subcommands.",
        ],
    );
}

#[test]
fn verify_cert_long_help_is_focused() {
    assert_verify_cert_help(&["verify-cert", "--help"]);
}

#[test]
fn verify_cert_short_help_is_focused() {
    assert_verify_cert_help(&["verify-cert", "-h"]);
}

#[test]
fn verify_cert_help_word_is_focused() {
    assert_verify_cert_help(&["verify-cert", "help"]);
}

#[test]
fn verify_all_long_help_is_focused() {
    assert_verify_all_help(&["verify-all", "--help"]);
}

#[test]
fn verify_all_short_help_is_focused() {
    assert_verify_all_help(&["verify-all", "-h"]);
}

#[test]
fn verify_all_help_word_is_focused() {
    assert_verify_all_help(&["verify-all", "help"]);
}
