//! RES-3223: tooling docs cfg examples point at checkout-valid files.

use std::path::Path;

#[test]
fn tooling_docs_use_existing_cfg_example_paths() {
    let doc = include_str!("../../docs/tooling.md");
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("resilient crate has repo parent");

    for expected in [
        "rz --feature verbose resilient/examples/cfg_feature.rz",
        "rz --target thumbv7em resilient/examples/cfg_target.rz",
        "rz --cfg mode=demo resilient/examples/cfg_kv_demo.rz",
    ] {
        assert!(
            doc.contains(expected),
            "tooling docs missing checkout-valid cfg command {expected:?}"
        );
    }
    for path in [
        "resilient/examples/cfg_feature.rz",
        "resilient/examples/cfg_target.rz",
        "resilient/examples/cfg_kv_demo.rz",
    ] {
        assert!(
            repo_root.join(path).is_file(),
            "documented cfg example should exist: {path}"
        );
    }
    for retired in [
        "rz --feature verbose examples/cfg_feature.rz",
        "rz --target thumbv7em examples/cfg_target.rz",
        "rz --cfg mode=demo examples/cfg_kv_demo.rz",
    ] {
        assert!(
            !doc.contains(retired),
            "tooling docs should not use non-checkout-root cfg path {retired:?}"
        );
    }
}
