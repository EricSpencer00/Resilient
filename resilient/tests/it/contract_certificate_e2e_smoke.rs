//! RES-3780: end-to-end pipeline smoke test tying together
//! `@require_contracts(strict)`, `#[loop_bound(N)]`, and
//! `--emit-contract-certificate` — the full "provably correct AI code"
//! workflow documented in `docs/HOWTO_PROVABLY_CORRECT_AI_CODE.md`.
//!
//! Must pass in the default (non-z3) build: without `--features z3`
//! every clause verdict degrades to `"unknown"` (RES-3859), so this
//! test asserts the certificate's *structure* — schema, per-function
//! enrolment, per-clause verdict-enum membership — and never that any
//! specific clause proves `"pass"`. That's what keeps it green with or
//! without `--features z3`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "res_3780_cert_e2e_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

const SOURCE: &str = r#"
@require_contracts(strict)

#[loop_bound(100)]
@ai_generated
fn count_up(int n) -> int requires n >= 0 requires n <= 100 ensures result >= 0 {
    let i = 0;
    while (i < n) {
        i = i + 1;
    }
    return i;
}

fn main() {
    println(count_up(5));
}

main();
"#;

#[test]
fn require_contracts_strict_loop_bound_certificate_pipeline() {
    let dir = tmp_dir("pipeline");
    let src_path = dir.join("count_up.rz");
    std::fs::write(&src_path, SOURCE).expect("write source");
    let cert_path = dir.join("cert.json");

    let out = Command::new(bin())
        .arg(&src_path)
        .arg("--emit-contract-certificate")
        .arg(&cert_path)
        .output()
        .expect("spawn rz --emit-contract-certificate");

    assert_eq!(
        out.status.code(),
        Some(0),
        "expected successful compile+run; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains('5'),
        "expected count_up(5) to actually run and print 5; stdout={stdout}"
    );

    let cert_raw = std::fs::read_to_string(&cert_path).expect("read emitted certificate");
    let cert: serde_json::Value =
        serde_json::from_str(&cert_raw).expect("certificate must be valid JSON");

    assert_eq!(
        cert.get("schema").and_then(|v| v.as_str()),
        Some("resilient-contract-certificate/v1"),
        "unexpected certificate: {cert}"
    );
    assert_eq!(
        cert.get("source").and_then(|v| v.as_str()),
        Some(src_path.to_string_lossy().as_ref())
    );

    let functions = cert
        .get("functions")
        .and_then(|v| v.as_array())
        .expect("certificate must have a functions array");
    assert!(
        !functions.is_empty(),
        "expected at least one function in the certificate: {cert}"
    );

    let count_up = functions
        .iter()
        .find(|f| f.get("name").and_then(|v| v.as_str()) == Some("count_up"))
        .unwrap_or_else(|| panic!("count_up must appear in the certificate: {cert}"));

    assert_eq!(
        count_up.get("enrolled").and_then(|v| v.as_bool()),
        Some(true),
        "count_up is declared under @require_contracts(strict) and must be enrolled"
    );

    // RES-3858: provenance is informational only — it must appear in
    // the certificate, but it must not be what decided enrolment above.
    let provenance = count_up
        .get("provenance")
        .and_then(|v| v.as_array())
        .expect("provenance must be an array");
    assert!(
        provenance
            .iter()
            .any(|v| v.as_str() == Some("ai_generated")),
        "expected the ai_generated provenance tag on count_up, got {provenance:?}"
    );

    let clauses = count_up
        .get("clauses")
        .and_then(|v| v.as_array())
        .expect("count_up must have a clauses array");
    assert!(
        clauses.len() >= 3,
        "expected the 2 requires + 1 ensures clauses declared in source, got {clauses:?}"
    );

    let valid_kinds = [
        "requires",
        "ensures",
        "inferred_requires",
        "inferred_ensures",
    ];
    let valid_verdicts = ["pass", "fail", "unknown"];
    for clause in clauses {
        let kind = clause
            .get("kind")
            .and_then(|v| v.as_str())
            .expect("clause missing kind");
        assert!(
            valid_kinds.contains(&kind),
            "unexpected clause kind {kind:?} in {clause}"
        );

        let verdict = clause
            .get("verdict")
            .and_then(|v| v.as_str())
            .expect("clause missing verdict");
        assert!(
            valid_verdicts.contains(&verdict),
            "unexpected verdict {verdict:?} — must be one of {valid_verdicts:?}; clause={clause}"
        );

        // A `smtlib2` replay dump or `counterexample` may only appear
        // alongside their matching verdict; this holds in every build
        // configuration, unlike the verdict value itself.
        assert!(
            !clause.as_object().unwrap().contains_key("smtlib2") || verdict == "pass",
            "smtlib2 certificate must only appear on a pass verdict: {clause}"
        );
        assert!(
            !clause.as_object().unwrap().contains_key("counterexample") || verdict == "fail",
            "counterexample must only appear on a fail verdict: {clause}"
        );
    }

    // `main` is exempt from strict presence-of-contracts (RES-3854)
    // but is still enrolled by the module-level directive.
    let main_fn = functions
        .iter()
        .find(|f| f.get("name").and_then(|v| v.as_str()) == Some("main"))
        .unwrap_or_else(|| panic!("main must appear in the certificate: {cert}"));
    assert_eq!(
        main_fn.get("enrolled").and_then(|v| v.as_bool()),
        Some(true),
        "main is enrolled by the module-level @require_contracts(strict) directive"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
