//! RES-3227: SYNTAX.md uses a checkout-valid hello example path.

use std::path::Path;

#[test]
fn syntax_docs_use_checkout_valid_hello_path() {
    let doc = include_str!("../../SYNTAX.md");
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("resilient crate has repo parent");

    let command = "rz resilient/examples/hello.rz";
    assert!(
        doc.contains(command),
        "SYNTAX.md should show checkout-valid hello command {command:?}"
    );
    assert!(
        repo_root.join("resilient/examples/hello.rz").is_file(),
        "documented hello example should exist"
    );
    assert!(
        !doc.contains("rz examples/hello.rz"),
        "SYNTAX.md should not show a non-checkout-root hello path"
    );
}
