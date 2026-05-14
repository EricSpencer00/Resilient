//! Feature 11/50 — Anti-Regression Contracts.
//!
//! `#[stable(since = "1.0", behavior = "<digest>")]` marks a function
//! whose observable behavior is locked-in across versions. The
//! `behavior` argument is a fingerprint digest produced by
//! `crate::behavioral_fingerprint`; if the current digest of the
//! function diverges from the recorded one, the build fails.
//!
//! This is the direct CI-enforceable answer to "make sure my
//! vibe-coded app doesn't break after refactoring": once a function
//! has a `#[stable]` tag, every subsequent edit must preserve its
//! behavior or explicitly bump the digest (with a follow-up that
//! confirms the change is intentional and acceptable).

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;

#[derive(Debug, Clone)]
pub struct StableSpec {
    pub item_name: String,
    pub since: Option<String>,
    pub locked_digest: Option<u64>,
}

pub fn collect_stable_specs() -> Vec<StableSpec> {
    let attrs = crate::feature_attrs::find_kind("stable");
    let mut out = Vec::new();
    for (item, rec) in attrs {
        let mut s = StableSpec {
            item_name: item,
            since: None,
            locked_digest: None,
        };
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "since" => s.since = Some(v.to_string()),
                    "behavior" => s.locked_digest = v.parse().ok(),
                    _ => {}
                }
            }
        }
        out.push(s);
    }
    out
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let specs = collect_stable_specs();
    if specs.is_empty() {
        return Ok(());
    }
    // RES-1529: skip the expensive fingerprint walk when no spec has a
    // locked digest — the walk is only useful when at least one `#[stable]`
    // attribute carries a `behavior = "..."` argument.
    if !specs.iter().any(|s| s.locked_digest.is_some()) {
        return Ok(());
    }
    let fps = crate::behavioral_fingerprint::fingerprint_program(program);
    for s in &specs {
        if let Some(locked) = s.locked_digest {
            if let Some(current) = fps.get(&s.item_name) {
                if current.digest != locked {
                    return Err(format!(
                        "{}:0:0: error: `{}` is `#[stable]` (since={:?}) but its behavior digest changed: {} → {}. Either revert the change or update the digest.",
                        source_path, s.item_name, s.since, locked, current.digest
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn matching_digest_passes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = r#"fn f(int x) -> int ensures result > 0 { return x; }"#;
        let (prog, _) = parse(src);
        let fps = crate::behavioral_fingerprint::fingerprint_program(&prog);
        let digest = fps["f"].digest;
        crate::feature_attrs::record(
            "f",
            crate::feature_attrs::AttrRecord {
                name: "stable".into(),
                args: format!(r#"since = "1.0", behavior = "{digest}""#),
                line: 0,
            },
        );
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn mismatched_digest_fails() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = r#"fn f(int x) -> int ensures result > 0 { return x; }"#;
        let (prog, _) = parse(src);
        crate::feature_attrs::record(
            "f",
            crate::feature_attrs::AttrRecord {
                name: "stable".into(),
                args: r#"since = "1.0", behavior = "999999""#.into(),
                line: 0,
            },
        );
        assert!(check(&prog, "test").is_err());
        crate::feature_attrs::reset();
    }
}
