//! RES-3245: README runtime docs describe the current Cargo workspace.

#[test]
fn readme_runtime_section_mentions_current_workspace() {
    let readme = include_str!("../../README.md");

    assert!(
        readme.contains("The root Cargo workspace keeps `resilient/`, `resilient-runtime/`,"),
        "README should describe the existing workspace root"
    );
    assert!(
        readme.contains("and `resilient-span/` as separate packages"),
        "README should name the current workspace member crates"
    );
    assert!(
        readme.contains("`resilient-runtime/` remains"),
        "README should still identify resilient-runtime as the no-std runtime package"
    );
    assert!(
        !readme.contains("A future ticket can promote both to a workspace"),
        "README should not describe the existing workspace as future work"
    );
}
