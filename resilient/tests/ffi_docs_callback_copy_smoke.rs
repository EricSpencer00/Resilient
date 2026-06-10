//! RES-3329: FFI callback docs use user-facing unsupported wording.

#[test]
fn ffi_docs_describe_callback_as_unsupported_not_stubbed() {
    let ffi_doc = include_str!("../../docs/ffi.md");

    for expected in [
        "C function pointer (recognised in declarations; calls unsupported in Phase 1)",
        "### `Callback` — declaration-only in Phase 1",
        "Passing a Resilient function as `Callback` is not supported in Phase 1",
        "returns a clean error:",
        "callbacks\nrequire the trampoline feature (planned for Phase 2)",
    ] {
        assert!(
            ffi_doc.contains(expected),
            "FFI callback docs should use current user-facing wording: {expected:?}"
        );
    }

    for stale in [
        "Phase 1 stub",
        "Callback` is stubbed in Phase 1",
        "Callback)  | C function pointer (Phase 1 stub",
    ] {
        assert!(
            !ffi_doc.contains(stale),
            "FFI callback docs should not expose internal stub wording: {stale:?}"
        );
    }
}
