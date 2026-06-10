//! RES-3247: README project status points to live planning sources.

#[test]
fn readme_project_status_uses_live_planning_sources() {
    let readme = include_str!("../../README.md");

    assert!(
        readme.contains("Current priorities live in [ROADMAP.md](ROADMAP.md)")
            && readme.contains("GitHub issue queue"),
        "README should point readers to live planning sources"
    );
    assert!(
        readme.contains("Use the roadmap for the goalpost ladder"),
        "README should explain how to use the roadmap"
    );
    for stale_goalpost in ["G4 (", "G5 (", "G6 (", "G7 (", "G8\u{2013}G10", "G11+"] {
        assert!(
            !readme.contains(stale_goalpost),
            "README should not list stale static goalpost {stale_goalpost:?}"
        );
    }
}
