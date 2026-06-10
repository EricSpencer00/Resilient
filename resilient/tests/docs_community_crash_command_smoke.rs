//! RES-3213: community docs use the current crash-report command.

#[test]
fn community_docs_use_current_crash_report_command() {
    let doc = include_str!("../../docs/community.md");

    assert!(
        doc.contains("RUST_BACKTRACE=1 rz your_file.rz"),
        "community docs should show the current `rz` crash-report command"
    );
    assert!(
        !doc.contains("RUST_BACKTRACE=1 resilient your_file.rs"),
        "community docs should not show the retired binary name or .rs placeholder"
    );
}
