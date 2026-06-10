//! RES-3361: landing page copy should not imply placeholder runtime code.

#[test]
fn landing_copy_describes_supervisor_code_not_stub() {
    let landing = include_str!("../../docs/_layouts/landing.html");

    assert!(
        landing.contains("Live blocks compile away to ~80 bytes of supervisor code per site."),
        "landing page should describe live-block overhead with shipped-code wording"
    );

    assert!(
        !landing.contains("supervisor stub"),
        "landing page should not use placeholder-sounding supervisor stub wording"
    );
}
