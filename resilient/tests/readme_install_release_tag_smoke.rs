//! RES-3239: README install examples point at the current release tag.

#[test]
fn readme_install_examples_use_current_release_tag() {
    let readme = include_str!("../../README.md");

    assert!(
        readme.contains("Pin a version with `RZ_VERSION=v0.2.0"),
        "README should show the current release in the one-liner pin example"
    );
    assert!(
        readme.contains("TAG=v0.2.0  # see the releases page for the latest"),
        "README should show the current release in the pre-built binary example"
    );
    assert!(
        !readme.contains("RZ_VERSION=v0.1.0"),
        "README should not suggest the stale v0.1.0 pin"
    );
    assert!(
        !readme.contains("TAG=v0.1.0"),
        "README should not suggest the stale v0.1.0 archive tag"
    );
}
