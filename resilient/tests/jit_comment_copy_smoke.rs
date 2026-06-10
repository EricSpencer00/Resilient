//! RES-3345: JIT comments should describe the current backend path.

#[test]
fn jit_comments_use_current_backend_wording() {
    let cargo_toml = include_str!("../Cargo.toml");
    let lib = include_str!("../src/lib.rs");

    for expected in [
        "The JIT lowers the supported tree-walker subset to native code",
        "reports unsupported AST shapes cleanly so callers can fall back",
        "RES-072 / RES-096: route through the Cranelift JIT",
        "backend for the supported tree-walker subset",
    ] {
        assert!(
            cargo_toml.contains(expected) || lib.contains(expected),
            "JIT comments should describe current backend behavior: {expected:?}"
        );
    }

    for stale in [
        "Phase A only ships the scaffolding",
        "Phase A is a stub",
        "RES-096+ adds AST lowering",
        "RES-096+ adds real lowering",
    ] {
        assert!(
            !cargo_toml.contains(stale) && !lib.contains(stale),
            "JIT comments should not retain stale Phase A wording: {stale:?}"
        );
    }
}
