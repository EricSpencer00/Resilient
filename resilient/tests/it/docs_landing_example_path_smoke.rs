//! RES-4955: landing installation is one curl command for the current release.

use std::path::Path;

#[test]
fn landing_page_install_command_uses_the_current_release() {
    let html = include_str!("../../../docs/_layouts/landing.html");
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("resilient crate has repo parent");

    let command = html
        .split("<pre class=\"rl-cta__cmd\">")
        .nth(1)
        .and_then(|rest| rest.split("</pre>").next())
        .expect("landing page has an install command");
    assert_eq!(
        command,
        format!(
            "curl -fsSL https://github.com/EricSpencer00/Resilient/releases/download/v{}/install.sh | bash",
            env!("CARGO_PKG_VERSION")
        ),
        "the requested install flow is one versioned curl command"
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
