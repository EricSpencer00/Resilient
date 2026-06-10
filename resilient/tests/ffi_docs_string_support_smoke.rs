//! RES-3309: FFI docs describe current String support precisely.

#[test]
fn ffi_docs_describe_current_string_trampoline_scope() {
    let syntax = include_str!("../../SYNTAX.md");
    let ffi_doc = include_str!("../../docs/ffi.md");
    let ffi_source = include_str!("../src/ffi.rs");
    let variadic_smoke = include_str!("ffi_variadic_integration.rs");

    for doc in [syntax, ffi_doc] {
        for expected in [
            "variadic `printf`-style format strings",
            "fixed-arity string ABI arms remain limited to implemented trampoline shapes",
            "fn c_printf(fmt: String, ...) -> Int",
        ] {
            assert!(
                doc.contains(expected),
                "FFI docs should describe current String trampoline scope; missing {expected:?}"
            );
        }
        assert!(
            !doc.contains("| `String`  | not yet supported |")
                && !doc.contains("| `String`    | not yet supported"),
            "FFI docs should not claim String is wholly unsupported"
        );
    }

    assert!(
        ffi_source.contains("\"String\" => Some(FfiType::Str)"),
        "FFI signature resolver should still recognize String as FfiType::Str"
    );
    assert!(
        variadic_smoke.contains("fn c_printf(fmt: String, ...) -> Int"),
        "variadic FFI smoke should still cover the documented String format-parameter shape"
    );
}
