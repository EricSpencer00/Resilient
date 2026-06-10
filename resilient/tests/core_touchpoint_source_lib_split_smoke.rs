//! RES-3327: remaining core touchpoint comments point at lib.rs.

#[test]
fn core_touchpoint_comments_use_lib_rs_sources() {
    let lib = include_str!("../src/lib.rs");
    let vm = include_str!("../src/vm.rs");

    for expected in [
        "the core library only adds:",
        "`lib.rs` carries the Node variant",
        "`lib.rs` carries the Node / Value variants",
        "`lib.rs` just registers",
        "Node::Function::type_params field) lives in lib.rs",
        "lib.rs for NewtypeDecl (register)",
        "helpers in `tuples.rs`; `lib.rs`",
        "tree-walker interpreter in `lib.rs`",
    ] {
        assert!(
            lib.contains(expected) || vm.contains(expected),
            "core touchpoint comments should include lib.rs wording: {expected:?}"
        );
    }

    for stale in [
        "the main module only adds:",
        "All feature logic lives here; main.rs only adds",
        "`tuples.rs`; main.rs only adds",
        "thread-local handle registry live here; main.rs just registers",
        "Node::Function::type_params field) lives in main.rs",
        "main.rs for NewtypeDecl (register)",
        "helpers in `tuples.rs`; main.rs",
        "The interpreter in `main.rs`",
    ] {
        assert!(
            !lib.contains(stale) && !vm.contains(stale),
            "core touchpoint comments should not retain stale wording: {stale:?}"
        );
    }
}
