//! RES-4115 (E-E4 increment 4): CI lint for a codeless diagnostic
//! funnel.
//!
//! Resilient's error-message call sites don't (yet) all go through
//! `crate::diag::Diagnostic` directly — most of the typechecker and
//! parser still return a bare `String` built by a small, named set of
//! `render_*_error` funnel functions (see `typechecker.rs`,
//! `imports.rs`, `lib.rs`), each of which prefixes a `[E00NN]` bracket
//! when `RESILIENT_RICH_DIAG=1` is set. This test is the CI-enforced
//! guard that a *new* funnel function can't be added without wiring a
//! registered code into it.
//!
//! ## Mechanism
//!
//! Every source file in `SCOPED_FILES` is scanned for top-level `fn
//! render_..._error(...)` definitions (the funnel naming convention).
//! For each one found, the function body must contain both
//! `rich_diag_enabled()` (the gate) and a `[E` or `[W` bracket literal
//! (the code) — unless the function's name is in `LEGACY_CODELESS_ALLOWLIST`,
//! a fixed, individually-justified list of pre-existing funnels that
//! predate this lint and haven't been migrated yet.
//!
//! `LEGACY_CODELESS_ALLOWLIST` is meant to **shrink monotonically**: as
//! each legacy funnel gets a code (see the pattern in
//! `render_rich_arg_type_mismatch` / `render_missing_return_error` for
//! reference), remove its name from the list. Adding a *new* name to
//! the list is a regression this test can't catch mechanically — reviewers
//! (or the PR body, since this repo has no human review gate) must
//! justify any addition.
//!
//! This intentionally does not scan the entire codebase for bare
//! `format!`/`record_error` call sites: the long tail of unmigrated
//! one-off parser/interpreter error strings (e.g. `Duplicate named
//! argument ... in call`) is out of scope until it's promoted to a
//! named funnel function — scanning raw string literals would produce
//! hundreds of false positives unrelated to this ticket's scope.

/// Legacy funnels that exist today without a registered code. Empty:
/// every current `render_*_error` funnel in the scoped files already
/// carries a code. Keep this list here (rather than deleting the
/// mechanism) so the next funnel that's added *without* a code has an
/// obvious, documented escape hatch instead of reviewers reaching for
/// `#[allow]` or deleting the test.
const LEGACY_CODELESS_ALLOWLIST: &[&str] = &[];

struct ScopedFile {
    path: &'static str,
    source: &'static str,
}

const SCOPED_FILES: &[ScopedFile] = &[
    ScopedFile {
        path: "resilient/src/typechecker.rs",
        source: include_str!("../../src/typechecker.rs"),
    },
    ScopedFile {
        path: "resilient/src/imports.rs",
        source: include_str!("../../src/imports.rs"),
    },
    ScopedFile {
        path: "resilient/src/lib.rs",
        source: include_str!("../../src/lib.rs"),
    },
    ScopedFile {
        path: "resilient/src/immutability.rs",
        source: include_str!("../../src/immutability.rs"),
    },
    ScopedFile {
        path: "resilient/src/dyn_trait.rs",
        source: include_str!("../../src/dyn_trait.rs"),
    },
];

/// Extracts `(name, body)` for every `fn render_..._error(` definition
/// in `source`, where `body` is the brace-matched function body
/// (inclusive of the outer `{`/`}`).
fn find_render_error_funnels(source: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut search_from = 0usize;
    while let Some(rel) = source[search_from..].find("fn render_") {
        let start = search_from + rel;
        let after_fn = &source[start + 3..]; // skip "fn "
        let name_end = after_fn
            .find(|c: char| c == '(' || c.is_whitespace())
            .unwrap_or(after_fn.len());
        let name = &after_fn[..name_end];
        search_from = start + 10; // advance past "fn render_" to avoid rematching

        if !name.ends_with("_error") {
            continue;
        }

        let Some(brace_rel) = source[start..].find('{') else {
            continue;
        };
        let body_start = start + brace_rel;
        let mut depth = 0i32;
        let mut body_end = None;
        for (i, ch) in source[body_start..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        body_end = Some(body_start + i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(body_end) = body_end else { continue };
        out.push((name.to_string(), source[body_start..body_end].to_string()));
    }
    out
}

#[test]
fn every_render_error_funnel_carries_a_registered_code() {
    let mut checked = 0usize;
    for file in SCOPED_FILES {
        for (name, body) in find_render_error_funnels(file.source) {
            checked += 1;
            if LEGACY_CODELESS_ALLOWLIST.contains(&name.as_str()) {
                continue;
            }
            let has_gate = body.contains("rich_diag_enabled()");
            let has_bracket_code = body.contains("[E") || body.contains("[W");
            assert!(
                has_gate && has_bracket_code,
                "{}::{name} is a new codeless diagnostic funnel — every `render_*_error` \
                 function must gate a `[E00NN]`/`[W00NN]` prefix behind `rich_diag_enabled()` \
                 (see `render_missing_return_error` for the pattern), or be added to \
                 LEGACY_CODELESS_ALLOWLIST with a justification in the PR body if a code \
                 genuinely doesn't exist yet for this diagnostic class.",
                file.path
            );
        }
    }
    assert!(
        checked >= 10,
        "expected to find the known render_*_error funnels across the scoped files; \
         found {checked} — did a scoped file move or get renamed?"
    );
}

#[test]
fn allowlist_only_names_funnels_that_actually_exist() {
    // Guards against a stale allowlist entry surviving after its funnel
    // was migrated or deleted — an allowlist that references a
    // nonexistent function name gives a false sense that something is
    // still being grandfathered in.
    let all_names: Vec<String> = SCOPED_FILES
        .iter()
        .flat_map(|f| find_render_error_funnels(f.source))
        .map(|(name, _)| name)
        .collect();
    for allowed in LEGACY_CODELESS_ALLOWLIST {
        assert!(
            all_names.iter().any(|n| n == allowed),
            "LEGACY_CODELESS_ALLOWLIST names `{allowed}`, but no such render_*_error \
             function exists anymore in the scoped files — remove the stale entry"
        );
    }
}

#[test]
fn diagnostic_new_call_sites_outside_diag_rs_carry_a_code() {
    // `Diagnostic::new(...)` construction sites in non-test code
    // (infer.rs is the only current caller outside diag.rs itself)
    // must chain `.with_code(...)` within the same statement. This
    // complements the render_*_error funnel check above by covering
    // callers that build a `Diagnostic` value directly instead of a
    // bare `String`.
    let infer_src = include_str!("../../src/infer.rs");
    let mut search_from = 0usize;
    let mut checked = 0usize;
    while let Some(rel) = infer_src[search_from..].find("Diagnostic::new(") {
        let start = search_from + rel;
        search_from = start + "Diagnostic::new(".len();
        // Look at the next ~400 bytes for a chained `.with_code(` — every
        // real call site in this file closes the call and chains
        // `.with_code(...)` well within that window.
        let window_end = (start + 400).min(infer_src.len());
        let window = &infer_src[start..window_end];
        checked += 1;
        assert!(
            window.contains(".with_code("),
            "infer.rs: found a `Diagnostic::new(` call site (byte offset {start}) with no \
             `.with_code(...)` chained nearby — every non-test Diagnostic construction must \
             carry a registered (or T-prototype) code"
        );
    }
    assert!(
        checked >= 4,
        "expected to find infer.rs's known Diagnostic::new call sites; found {checked}"
    );
}
