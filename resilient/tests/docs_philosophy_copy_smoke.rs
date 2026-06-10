//! RES-3243: philosophy docs use Resilient fences and root-safe commands.

#[test]
fn philosophy_docs_use_resilient_fences_and_root_safe_cargo() {
    let doc = include_str!("../../docs/philosophy.md");

    assert!(
        doc.contains("```resilient\nlive {"),
        "live-block example should be fenced as Resilient"
    );
    assert!(
        doc.contains("```resilient\nfn safe_div"),
        "contract example should be fenced as Resilient"
    );
    assert!(
        doc.contains("cargo run --manifest-path resilient/Cargo.toml --features z3 -- --emit-certificate ./certs prog.rz"),
        "certificate command should be runnable from the repository root"
    );
    assert!(
        !doc.contains("cargo run --features z3 -- --emit-certificate"),
        "docs should not assume the reader is already inside resilient/"
    );
}
