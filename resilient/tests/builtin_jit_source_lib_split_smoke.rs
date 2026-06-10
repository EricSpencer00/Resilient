//! RES-3321: VM/JIT comments point builtin and exit hooks at lib.rs.

#[test]
fn builtin_and_jit_comments_use_lib_rs_sources() {
    let bytecode = include_str!("../src/bytecode.rs");
    let jit_backend = include_str!("../src/jit_backend.rs");

    for expected in [
        "canonical `BUILTINS` slice in `lib.rs`",
        "handler from `lib.rs` at program exit",
        "see `BUILTINS` in lib.rs",
        "Int builtins in lib.rs's BUILTINS table",
        "`Value::Int` case in lib.rs",
        "two-Int case in lib.rs",
        "from `lib.rs` at exit",
    ] {
        assert!(
            bytecode.contains(expected) || jit_backend.contains(expected),
            "VM/JIT comments should include current lib.rs wording: {expected:?}"
        );
    }

    for stale in [
        "canonical `BUILTINS` slice in `main.rs`",
        "handler from `main.rs` at program exit",
        "see `BUILTINS` in main.rs",
        "Int builtins in main.rs's BUILTINS table",
        "`Value::Int` case in main.rs",
        "two-Int case in main.rs",
        "from `main.rs` at exit",
    ] {
        assert!(
            !bytecode.contains(stale) && !jit_backend.contains(stale),
            "VM/JIT comments should not retain stale main.rs wording: {stale:?}"
        );
    }
}
