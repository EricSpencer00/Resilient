//! RES-3214: certification docs keep Cargo feature flags out of `rz` commands.

#[test]
fn certification_docs_describe_z3_as_build_prerequisite() {
    let doc = include_str!("../../docs/certification.md");

    assert!(
        doc.contains("the `rz` binary was built with `--features z3`"),
        "certification docs should preserve the z3 build prerequisite"
    );
    assert!(
        doc.contains(
            "# One shot: typecheck + verify + audit + emit signed certificates.\nrz \\\n    --audit"
        ),
        "certification docs should show `rz --audit` without Cargo feature flags"
    );
    assert!(
        !doc.contains("rz \\\n    --features z3"),
        "certification docs should not pass Cargo feature flags to `rz`"
    );
}
