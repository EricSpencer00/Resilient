//! RES-4227: keep the optional hstcore example honest without requiring the
//! optional shared library or a licensed operator artifact in CI.

#[test]
fn hstcore_example_declares_the_guarded_abi_and_reference_probe() {
    let source = include_str!("../../examples/ffi_hstcore.rz");
    for expected in [
        "fn hst_open(",
        "artifact_path: CStr",
        "errbuf: Buffer<Int>",
        "fn hst_apply_delta(",
        "vals: Array<Float>",
        "y_out: Buffer<Float>",
        "fn hst_recompute_full(",
        "ensures result == true",
        "hst_close(ctx)",
    ] {
        assert!(
            source.contains(expected),
            "hstcore example lost required binding or oracle clause: {expected:?}"
        );
    }
}

#[test]
fn hstcore_example_is_marked_optional_and_not_golden_run() {
    let marker = include_str!("../../examples/ffi_hstcore.interactive");
    assert!(marker.contains("optional HST-core library"));
    assert!(marker.contains("exempt from the golden-file audit"));
}
