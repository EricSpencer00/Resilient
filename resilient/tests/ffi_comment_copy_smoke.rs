//! RES-3355: FFI comments should describe backend boundaries, not stubs.

#[test]
fn ffi_comments_describe_backend_boundary() {
    let ffi_source = include_str!("../src/ffi.rs");
    let lib_source = include_str!("../src/lib.rs");

    for expected in [
        "FFI v1 ships as a shared API plus backend boundary",
        "`ffi` enables\n// dynamic loading",
        "default backend returns `FfiDisabled`",
        "disabled backend that returns `FfiError::FfiDisabled`",
    ] {
        assert!(
            ffi_source.contains(expected) || lib_source.contains(expected),
            "FFI comments should describe the enabled/disabled backend boundary: {expected:?}"
        );
    }

    for stale in [
        "Phase 1 skeleton",
        "build stays warning-clean as a stub",
        "`disabled` stub that returns `FfiError::FfiDisabled`",
    ] {
        assert!(
            !ffi_source.contains(stale) && !lib_source.contains(stale),
            "FFI comments should not retain skeleton/stub wording: {stale:?}"
        );
    }
}
