//! RES-3313: agent playbook treats lib.rs as the primary extension-block file.

#[test]
fn agent_playbook_uses_lib_rs_for_primary_append_only_guidance() {
    let playbook = include_str!("../../docs/AGENT_PLAYBOOK.md");
    let auto_resolve = include_str!("../../agent-scripts/auto-resolve-extensions.sh");
    let sync_integration = include_str!("../../agent-scripts/sync-integration.sh");

    for expected in [
        "`resilient/src/{lib.rs,typechecker.rs,lexer_logos.rs}` are shared by",
        "`lib.rs` is the large core file; `main.rs` remains in",
        "the resolver allowlist only for legacy compatibility",
        "append-only allowlist (lib.rs, typechecker.rs, lexer_logos.rs,\n   file-claims.json; main.rs is legacy-compatible)",
    ] {
        assert!(
            playbook.contains(expected),
            "agent playbook should make lib.rs primary in append-only guidance; missing {expected:?}"
        );
    }

    for script in [auto_resolve, sync_integration] {
        assert!(
            script.contains("resilient/src/lib.rs")
                && script.contains("resilient/src/main.rs")
                && script.contains("legacy"),
            "agent scripts should keep lib.rs primary while retaining legacy main.rs compatibility"
        );
    }

    for stale in [
        "`resilient/src/{main.rs,typechecker.rs,lexer_logos.rs}` are shared by",
        "append-only allowlist (main.rs, typechecker.rs, lexer_logos.rs,\n   file-claims.json)",
    ] {
        assert!(
            !playbook.contains(stale),
            "agent playbook should not retain stale main.rs-primary guidance: {stale:?}"
        );
    }
}
