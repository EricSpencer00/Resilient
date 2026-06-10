//! RES-3224: landing page copy uses a checkout-valid run command.

use std::path::Path;

#[test]
fn landing_page_run_command_points_at_existing_example() {
    let html = include_str!("../../docs/_layouts/landing.html");
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("resilient crate has repo parent");

    let command = "rz resilient/examples/sensor_monitor.rz";
    assert!(
        html.contains(command),
        "landing page should show checkout-valid run command {command:?}"
    );
    assert!(
        repo_root
            .join("resilient/examples/sensor_monitor.rz")
            .is_file(),
        "landing page run command should reference an existing example"
    );
    assert!(
        !html.contains("rz examples/altitude_controller.rz"),
        "landing page should not show a missing example path"
    );
}
