//! RES-3317: strict CLI toggle comments point at the library dispatcher.

#[test]
fn strict_cli_toggle_comments_use_lib_rs_dispatcher() {
    let bounds_check = include_str!("../src/bounds_check.rs");
    let termination = include_str!("../src/termination.rs");

    for (name, source) in [("bounds_check", bounds_check), ("termination", termination)] {
        assert!(
            source.contains("Called from the `lib.rs` CLI\n/// dispatcher before `check_program_with_source` runs."),
            "{name} should point strict CLI toggle setup at the lib.rs dispatcher"
        );
        assert!(
            !source.contains(
                "Called from `main.rs` CLI\n/// parsing before `check_program_with_source` runs."
            ),
            "{name} should not retain stale main.rs CLI toggle wording"
        );
    }
}
