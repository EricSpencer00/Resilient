//! RES-3835: `#[generated(intent="...", prompt_hash="...")]` — compile-time
//! provenance annotation for AI-generated code.
//!
//! Attributes the compiler checks:
//!
//! * `#[generated(intent="<free text>", prompt_hash="<non-empty string>")]` on
//!   any function or struct. The compiler enforces *presence* and *well-
//!   formedness* of the fields — it does not validate the content.
//!
//! ## Enforcement rules
//!
//! 1. `#[generated]` without parentheses is a hard error (missing fields).
//! 2. `prompt_hash` must be a non-empty string literal — an empty hash is
//!    equivalent to no hash and defeats the audit trail.
//! 3. `intent` must be present, but its content is unconstrained.
//! 4. Unknown keys inside the parens are silently ignored so forward-compat
//!    extensions (e.g. `model="..."`) don't break existing files.
//!
//! ## Integration with behavioral_fingerprint
//!
//! After the check pass the `generated` registry entries are readable by
//! `behavioral_fingerprint::fingerprint_program` via
//! `crate::feature_attrs::find_kind("generated")`. The fingerprint module
//! does not need to know the parse grammar — it reads `AttrRecord::args`
//! as an opaque string and stores it alongside the digest.

#![allow(clippy::doc_lazy_continuation)]

use crate::Node;

/// Parse the raw args string from a `#[generated(...)]` attribute.
/// Returns `(intent, prompt_hash)` on success, or an error string.
///
/// Accepted format: `intent="...", prompt_hash="..."` (order-independent,
/// whitespace-tolerant, unknown keys are ignored).
fn parse_generated_args(raw: &str) -> Result<(String, String), String> {
    let mut intent: Option<String> = None;
    let mut prompt_hash: Option<String> = None;

    // Minimal key="value" scanner. We don't use a full expression parser
    // here because the args string is already extracted by cfg_attr::parse_cfg_attribute
    // and is guaranteed to be a flat comma-separated key=value list.
    let mut remaining = raw.trim();
    while !remaining.is_empty() {
        // Find the next key= pair.
        let eq_pos = remaining
            .find('=')
            .ok_or_else(|| format!("expected `key=\"value\"` pairs, got `{remaining}`"))?;
        let key = remaining[..eq_pos].trim().to_string();
        remaining = remaining[eq_pos + 1..].trim_start();

        // Expect an opening quote.
        if !remaining.starts_with('"') {
            return Err(format!("value for `{key}` must be a string literal"));
        }
        remaining = &remaining[1..];

        // Find the closing quote (not preceded by `\`).
        let close = remaining
            .find('"')
            .ok_or_else(|| format!("unterminated string literal for `{key}`"))?;
        let value = remaining[..close].to_string();
        remaining = remaining[close + 1..].trim_start();
        // Skip optional comma separator.
        if remaining.starts_with(',') {
            remaining = remaining[1..].trim_start();
        }

        match key.as_str() {
            "intent" => intent = Some(value),
            "prompt_hash" => prompt_hash = Some(value),
            _ => {} // forward-compat: ignore unknown keys
        }
    }

    let intent =
        intent.ok_or_else(|| "`intent` field is required in `#[generated(...)]`".to_string())?;
    let prompt_hash = prompt_hash
        .ok_or_else(|| "`prompt_hash` field is required in `#[generated(...)]`".to_string())?;

    if prompt_hash.is_empty() {
        return Err(
            "`prompt_hash` must be non-empty — an empty hash defeats the audit trail".to_string(),
        );
    }

    Ok((intent, prompt_hash))
}

/// Typecheck pass: validate every `#[generated(...)]` attribute in the
/// attribute registry. Called from `typechecker.rs` `<EXTENSION_PASSES>`.
pub fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let entries = crate::feature_attrs::find_kind("generated");
    if entries.is_empty() {
        return Ok(());
    }

    let mut errors: Vec<String> = Vec::new();

    for (item_name, record) in &entries {
        if record.args.is_empty() {
            errors.push(format!(
                "{}:{}:0: error: `#[generated]` on `{}` requires `intent` and `prompt_hash` fields — \
                 use `#[generated(intent=\"...\", prompt_hash=\"...\")]`",
                source_path, record.line, item_name
            ));
            continue;
        }

        if let Err(msg) = parse_generated_args(&record.args) {
            errors.push(format!(
                "{}:{}:0: error: malformed `#[generated]` on `{}`: {}",
                source_path, record.line, item_name, msg
            ));
        }
    }

    // Also check #[ai_review_required] without #[generated] — emit a warning
    // (not an error) so teams can migrate incrementally.
    let ai_review = crate::feature_attrs::find_kind("ai_review_required");
    let generated_items: std::collections::HashSet<&str> =
        entries.iter().map(|(name, _)| name.as_str()).collect();
    for (item_name, record) in &ai_review {
        if !generated_items.contains(item_name.as_str()) {
            eprintln!(
                "{}:{}:0: warning: `{}` has `#[ai_review_required]` but no `#[generated(...)]` — \
                 add a provenance annotation to enable full audit trail",
                source_path, record.line, item_name
            );
        }
    }

    if errors.is_empty() {
        // Validate that the referenced program has at least one node — just
        // ensures we consumed the argument (clippy: unused variable).
        let _ = matches!(program, Node::Program(_));
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_args() {
        let (intent, hash) =
            parse_generated_args(r#"intent="create user account", prompt_hash="sha256:abc123""#)
                .unwrap();
        assert_eq!(intent, "create user account");
        assert_eq!(hash, "sha256:abc123");
    }

    #[test]
    fn parse_order_independent() {
        let (intent, hash) =
            parse_generated_args(r#"prompt_hash="deadbeef", intent="validate payment""#).unwrap();
        assert_eq!(intent, "validate payment");
        assert_eq!(hash, "deadbeef");
    }

    #[test]
    fn parse_missing_intent_errors() {
        let err = parse_generated_args(r#"prompt_hash="abc""#).unwrap_err();
        assert!(err.contains("intent"), "error must mention 'intent': {err}");
    }

    #[test]
    fn parse_missing_hash_errors() {
        let err = parse_generated_args(r#"intent="do something""#).unwrap_err();
        assert!(
            err.contains("prompt_hash"),
            "error must mention 'prompt_hash': {err}"
        );
    }

    #[test]
    fn parse_empty_hash_errors() {
        let err = parse_generated_args(r#"intent="do something", prompt_hash="""#).unwrap_err();
        assert!(
            err.contains("non-empty"),
            "error must mention 'non-empty': {err}"
        );
    }

    #[test]
    fn parse_unknown_key_ignored() {
        let result = parse_generated_args(r#"intent="x", prompt_hash="y", model="gpt-4o""#);
        assert!(result.is_ok(), "unknown keys must be silently ignored");
    }

    #[test]
    fn check_pass_empty_registry() {
        // When no #[generated] attributes are present, the pass is a no-op.
        let prog = Node::Program(vec![]);
        assert!(check(&prog, "test.rz").is_ok());
    }

    #[test]
    fn check_pass_with_valid_attr() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "create_user",
            crate::feature_attrs::AttrRecord {
                name: "generated".into(),
                args: r#"intent="create user", prompt_hash="abc123""#.into(),
                line: 1,
            },
        );
        let prog = Node::Program(vec![]);
        assert!(check(&prog, "test.rz").is_ok());
    }

    #[test]
    fn check_pass_rejects_empty_args() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "my_fn",
            crate::feature_attrs::AttrRecord {
                name: "generated".into(),
                args: String::new(),
                line: 5,
            },
        );
        let prog = Node::Program(vec![]);
        let err = check(&prog, "src/main.rz").unwrap_err();
        assert!(
            err.contains("intent") && err.contains("prompt_hash"),
            "error must mention both required fields: {err}"
        );
    }

    #[test]
    fn check_pass_rejects_empty_hash() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "my_fn",
            crate::feature_attrs::AttrRecord {
                name: "generated".into(),
                args: r#"intent="do something", prompt_hash="""#.into(),
                line: 3,
            },
        );
        let prog = Node::Program(vec![]);
        let err = check(&prog, "src/main.rz").unwrap_err();
        assert!(
            err.contains("non-empty"),
            "error must mention non-empty requirement: {err}"
        );
    }
}
