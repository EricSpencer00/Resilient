//! RES-3371: philosophy docs describe the JIT diagnostic boundary accurately.

#[test]
fn philosophy_docs_describe_static_jit_unsupported_diagnostics() {
    let doc = include_str!("../../docs/philosophy.md");

    for expected in [
        "**Unsupported JIT diagnostics still use static labels.**",
        "`JitError::Unsupported(&'static str)` covers shapes the JIT",
        "include the actual name yet. A follow-up should let that variant",
        "carry owned context without changing the already string-backed",
        "JIT errors.",
    ] {
        assert!(
            doc.contains(expected),
            "philosophy docs should describe the JIT diagnostic boundary: {expected:?}"
        );
    }

    for stale in [
        "**The error type carries `&'static str`.**",
        "A future ticket will widen `JitError` to",
        "carry owned strings.",
    ] {
        assert!(
            !doc.contains(stale),
            "philosophy docs should not imply every JitError is static-only: {stale:?}"
        );
    }
}
