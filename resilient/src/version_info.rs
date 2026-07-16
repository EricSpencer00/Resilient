//! RES-2631: build-metadata version banner.
//!
//! Two-form `--version` output for the `rz` CLI driver:
//!
//! - **Short** (`rz --version`) — single line, version + pre-1.0
//!   stability notice, identical to the historical `--version` line
//!   so existing CI scrapers do not need to change.
//! - **Verbose** (`rz --version --verbose`) — appends the
//!   commit hash, build date, target triple, profile, rustc
//!   version, and enabled compile-time features. Useful for
//!   reproducible-build verification and bug-report attachment in
//!   safety-critical contexts where the exact compiler bytes matter.
//!
//! All metadata is read from `RESILIENT_BUILD_*` env vars set by
//! `build.rs`. Any value equal to the literal string `"unknown"`
//! (the fallback when the build environment cannot supply it — for
//! instance a release tarball without `.git`) is suppressed from the
//! verbose printout rather than echoed back as a useless line.

/// RES-2631: the short single-line banner.
///
/// RES-4102 (F-E6): the stability notice is derived from the version
/// rather than hardcoded to `pre-1.0`, so the same banner stays honest
/// as the crate crosses the 1.0 line:
///   - `0.x`            → `pre-1.0` (breaking changes at any time)
///   - `x.y.z-<pre>`    → `release candidate` (API stabilizing)
///   - `>= 1.0.0` final → `stable release`
pub fn short() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let notice = if version.starts_with("0.") {
        "pre-1.0 — breaking changes possible (see STABILITY.md)"
    } else if version.contains('-') {
        "release candidate — API stabilizing toward 1.0 (see STABILITY.md)"
    } else {
        "stable release (see STABILITY.md)"
    };
    format!("rz {}: {}\n", version, notice)
}

/// RES-2631: the full multi-line `--version --verbose` banner.
///
/// Lines whose metadata value is `"unknown"` are suppressed so a
/// build outside a git tree does not print useless `commit: unknown`
/// noise.
pub fn verbose() -> String {
    let mut out = short();
    let lines = [
        ("commit", env!("RESILIENT_BUILD_GIT_HASH")),
        ("built", env!("RESILIENT_BUILD_DATE")),
        ("target", env!("RESILIENT_BUILD_TARGET")),
        ("profile", env!("RESILIENT_BUILD_PROFILE")),
        ("rustc", env!("RESILIENT_BUILD_RUSTC_VERSION")),
        ("features", env!("RESILIENT_BUILD_FEATURES")),
    ];
    for (label, value) in lines {
        if value != "unknown" {
            out.push_str(&format!("  {}: {}\n", label, value));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_form_starts_with_rz_and_version() {
        let s = short();
        assert!(s.starts_with("rz "), "got: {:?}", s);
        assert!(s.contains(env!("CARGO_PKG_VERSION")));
        // RES-4102: the notice text is version-derived (pre-1.0 /
        // release candidate / stable), but always points at the policy.
        assert!(s.contains("STABILITY.md"));
        assert!(s.ends_with('\n'));
    }

    #[test]
    fn verbose_form_contains_short_prefix() {
        let v = verbose();
        let s = short();
        assert!(v.starts_with(&s), "verbose must start with short banner");
    }

    #[test]
    fn verbose_form_omits_unknown_values() {
        let v = verbose();
        // The verbose banner never prints `: unknown` because the
        // formatter is supposed to drop those entries entirely.
        assert!(
            !v.contains(": unknown"),
            "unknown-value line leaked into verbose output: {:?}",
            v
        );
    }

    #[test]
    fn verbose_form_includes_known_labels_when_available() {
        let v = verbose();
        // Target triple is always set by Cargo even without git, so
        // it must be present in any verbose build.
        let target = env!("RESILIENT_BUILD_TARGET");
        if target != "unknown" {
            assert!(
                v.contains(&format!("target: {}", target)),
                "target line missing: {:?}",
                v
            );
        }
        // Profile likewise — set by Cargo.
        let profile = env!("RESILIENT_BUILD_PROFILE");
        if profile != "unknown" {
            assert!(
                v.contains(&format!("profile: {}", profile)),
                "profile line missing: {:?}",
                v
            );
        }
    }
}
