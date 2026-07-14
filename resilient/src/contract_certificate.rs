//! RES-3859 (#3854 Tier 3): contract proof certificates.
//!
//! `rz <file> --emit-contract-certificate <path>` writes a portable,
//! offline JSON document attesting — per function, per clause — what
//! the Z3 verifier actually established: `pass` (with the replayable
//! SMT-LIB2 certificate when the prover produced one), `fail` (with a
//! counterexample when Z3 gave a model), or `unknown` (out of the
//! supported subset, solver timeout, or a build without `--features
//! z3`; runtime checks remain).
//!
//! The certificate attests the *proof*, not the provenance. Whether a
//! function is tagged `@ai_generated` / `#[generated]` appears only as
//! an informational `provenance` array and changes nothing else in the
//! document — verification is a property of the code, not of who wrote
//! it (#3854).
//!
//! Output is deterministic: functions in program order, clauses in
//! `contract_verify::verify_program` order (declared `requires`, then
//! declared `ensures`, then inferred clauses), and every JSON object
//! writes its keys in a fixed order. Suitable for golden tests and
//! byte-for-byte audit diffs.
//!
//! ## Trust model (C-E5, #3933)
//!
//! The document by itself carries no cryptographic material — trusting
//! it means trusting the `rz` binary that produced it. Two layers hold
//! a consumer to something stronger:
//!
//! - **`"schema_version"`** (this module, every build): a numeric tag
//!   a consumer checks before parsing anything else. [`verify_schema_version`]
//!   never panics on malformed input — missing or unrecognized versions
//!   come back as a typed [`VerifyError`], so a future schema change
//!   fails closed instead of silently misparsing.
//! - **Ed25519 signature** (`--features z3` only, reusing the RES-194
//!   primitives in `cert_sign.rs`): [`sign_bytes`] / [`verify_signed`]
//!   bind the document's exact bytes to a keypair, so any tampering —
//!   even a single flipped bit — is detected. Gated on `z3` because
//!   `cert_sign` (and its `ed25519-dalek` / `rand_core` dependency
//!   chain) is only compiled under `--features z3` (RES-1202).

use std::fmt::Write as _;

use crate::Node;
use crate::contract_verify::{ClauseKind, ClauseVerdict, Verdict};

const SCHEMA: &str = "resilient-contract-certificate/v1";

/// C-E5: current certificate schema version. Bump this whenever a
/// field is added, removed, or reinterpreted in a way that changes how
/// a consumer must parse the document; [`verify_schema_version`]
/// rejects any other value it encounters.
pub const SCHEMA_VERSION: u64 = 1;

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

fn kind_label(kind: ClauseKind) -> &'static str {
    match kind {
        ClauseKind::Requires => "requires",
        ClauseKind::Ensures => "ensures",
        ClauseKind::InferredRequires => "inferred_requires",
        ClauseKind::InferredEnsures => "inferred_ensures",
    }
}

fn write_clause(out: &mut String, v: &ClauseVerdict) {
    out.push_str("      {\n");
    let _ = writeln!(out, "        \"clause\": \"{}\",", json_escape(&v.clause));
    let _ = writeln!(out, "        \"kind\": \"{}\",", kind_label(v.kind));
    // RES-3969: `basis` records whether an `ensures` verdict was
    // proven against the substituted function body (`implementation`)
    // or the free-variable clause text (`clause-only`). Emitted only
    // for `ensures`-family clauses — it is meaningless for `requires`
    // and consistency checks, which never substitute `result`.
    if matches!(v.kind, ClauseKind::Ensures | ClauseKind::InferredEnsures) {
        let _ = writeln!(out, "        \"basis\": \"{}\",", v.basis.label());
    }
    match &v.verdict {
        Verdict::Pass { certificate } => {
            if let Some(cert) = certificate {
                let _ = writeln!(out, "        \"smtlib2\": \"{}\",", json_escape(cert));
            }
            out.push_str("        \"verdict\": \"pass\"\n");
        }
        Verdict::Fail { counterexample } => {
            if let Some(cx) = counterexample {
                let _ = writeln!(out, "        \"counterexample\": \"{}\",", json_escape(cx));
            }
            out.push_str("        \"verdict\": \"fail\"\n");
        }
        Verdict::Unknown => {
            out.push_str("        \"verdict\": \"unknown\"\n");
        }
    }
    out.push_str("      }");
}

/// Provenance tags recorded for `item` — informational only.
fn provenance_tags(item: &str) -> Vec<&'static str> {
    let mut tags = Vec::new();
    if crate::feature_attrs::find_kind("ai_generated")
        .iter()
        .any(|(name, _)| name == item)
    {
        tags.push("ai_generated");
    }
    if crate::feature_attrs::find_kind("generated")
        .iter()
        .any(|(name, _)| name == item)
    {
        tags.push("generated");
    }
    tags
}

/// Build the certificate JSON document for `program`.
///
/// Every top-level function appears, in program order, with its
/// enrolment status ([`crate::contract_policy::is_enrolled`]), its
/// informational provenance tags, and one entry per contract clause
/// routed through the prover.
pub(crate) fn emit(program: &Node, source_path: &str) -> String {
    let verdicts = crate::contract_verify::verify_program(program);

    let mut out = String::new();
    out.push_str("{\n");
    let _ = writeln!(out, "  \"schema\": \"{}\",", json_escape(SCHEMA));
    let _ = writeln!(out, "  \"schema_version\": {},", SCHEMA_VERSION);
    let _ = writeln!(out, "  \"source\": \"{}\",", json_escape(source_path));
    out.push_str("  \"functions\": [\n");

    let mut first_fn = true;
    if let Node::Program(stmts) = program {
        for stmt in stmts {
            let Node::Function { name, .. } = &stmt.node else {
                continue;
            };
            if !first_fn {
                out.push_str(",\n");
            }
            first_fn = false;

            out.push_str("    {\n");
            let _ = writeln!(out, "      \"name\": \"{}\",", json_escape(name));
            let _ = writeln!(
                out,
                "      \"enrolled\": {},",
                crate::contract_policy::is_enrolled(name)
            );
            let tags = provenance_tags(name);
            let tags_json = tags
                .iter()
                .map(|t| format!("\"{t}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "      \"provenance\": [{tags_json}],");
            out.push_str("      \"clauses\": [\n");
            let mut first_clause = true;
            for v in verdicts.iter().filter(|v| &v.function_name == name) {
                if !first_clause {
                    out.push_str(",\n");
                }
                first_clause = false;
                write_clause(&mut out, v);
            }
            if !first_clause {
                out.push('\n');
            }
            out.push_str("      ]\n    }");
        }
    }
    if !first_fn {
        out.push('\n');
    }
    out.push_str("  ]\n}\n");
    out
}

/// C-E5: typed failure modes for certificate verification. Every
/// path through [`verify_schema_version`] / [`verify_signed`] returns
/// one of these instead of panicking — a malformed or tampered
/// certificate is untrusted input, not an invariant violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// The document is not valid JSON at all.
    InvalidJson(String),
    /// The document has no `"schema_version"` field.
    MissingSchemaVersion,
    /// `"schema_version"` is present but this build doesn't know how
    /// to interpret it.
    UnsupportedSchemaVersion(u64),
    /// The supplied Ed25519 public key bytes don't decode to a valid
    /// curve point.
    #[cfg(feature = "z3")]
    InvalidPublicKey(String),
    /// The Ed25519 signature does not verify against the document
    /// bytes and public key — the expected outcome for any tampered
    /// certificate.
    #[cfg(feature = "z3")]
    SignatureMismatch,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::InvalidJson(e) => write!(f, "certificate is not valid JSON: {e}"),
            VerifyError::MissingSchemaVersion => {
                write!(f, "certificate is missing the \"schema_version\" field")
            }
            VerifyError::UnsupportedSchemaVersion(v) => write!(
                f,
                "certificate schema_version {v} is not supported by this build (expected {SCHEMA_VERSION})"
            ),
            #[cfg(feature = "z3")]
            VerifyError::InvalidPublicKey(e) => write!(f, "invalid certificate public key: {e}"),
            #[cfg(feature = "z3")]
            VerifyError::SignatureMismatch => {
                write!(f, "certificate signature does not match its contents")
            }
        }
    }
}

impl std::error::Error for VerifyError {}

/// C-E5: parse and validate the `"schema_version"` field of a
/// certificate document. Feature-independent — every build, z3 or
/// not, can run this cheap structural check before deciding whether
/// to trust anything else in the document. Never panics: malformed
/// JSON, a missing field, and an unrecognized version all come back
/// as a specific [`VerifyError`] variant.
///
/// Public library API for external verifiers (and the future
/// `rz verify-contract-cert` CLI wiring tracked under #3933 · C-E5);
/// the production binary doesn't call it yet, so mark it
/// `#[allow(dead_code)]` like the sibling `cert_sign::format_*_pem`
/// helpers rather than hiding it behind `#[cfg(test)]`.
#[allow(dead_code)]
pub fn verify_schema_version(json: &str) -> Result<u64, VerifyError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| VerifyError::InvalidJson(e.to_string()))?;
    let version = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or(VerifyError::MissingSchemaVersion)?;
    if version != SCHEMA_VERSION {
        return Err(VerifyError::UnsupportedSchemaVersion(version));
    }
    Ok(version)
}

/// C-E5: sign a certificate document's raw bytes with an Ed25519
/// private key. Thin wrapper over the RES-194 primitive in
/// `cert_sign.rs` — gated on `z3` because that module (and its
/// `ed25519-dalek` / `rand_core` dependency chain) is only compiled
/// under `--features z3` (RES-1202). Public library API; no CLI
/// caller yet (see `verify_schema_version` doc comment above).
#[cfg(feature = "z3")]
#[allow(dead_code)]
pub fn sign_bytes(
    priv_key: &[u8; ed25519_dalek::SECRET_KEY_LENGTH],
    json_bytes: &[u8],
) -> [u8; ed25519_dalek::SIGNATURE_LENGTH] {
    crate::cert_sign::sign_payload(priv_key, json_bytes)
}

/// C-E5: verify a certificate document against a detached Ed25519
/// signature and public key. Checks `schema_version` first — a cheap,
/// non-cryptographic sanity check — so an unsupported-schema document
/// is rejected with a specific error rather than folded into a
/// generic signature failure. Any corruption of `json_bytes` (even a
/// single flipped bit) is expected to fail the signature check; see
/// `signature_rejects_every_single_byte_corruption` below.
#[cfg(feature = "z3")]
#[allow(dead_code)]
pub fn verify_signed(
    json_bytes: &[u8],
    sig: &[u8; ed25519_dalek::SIGNATURE_LENGTH],
    pub_key: &[u8; ed25519_dalek::PUBLIC_KEY_LENGTH],
) -> Result<(), VerifyError> {
    let text =
        std::str::from_utf8(json_bytes).map_err(|e| VerifyError::InvalidJson(e.to_string()))?;
    verify_schema_version(text)?;
    let ok = crate::cert_sign::verify_payload(pub_key, json_bytes, sig)
        .map_err(VerifyError::InvalidPublicKey)?;
    if ok {
        Ok(())
    } else {
        Err(VerifyError::SignatureMismatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emit_for(src: &str) -> String {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let json = emit(&prog, "<test>");
        crate::feature_attrs::reset();
        json
    }

    #[test]
    fn certificate_is_valid_shaped_json_with_fixed_keys() {
        let json = emit_for(
            "fn div(int a, int b) requires b != 0 ensures result == a / b { return a / b; }",
        );
        assert!(json.starts_with(
            "{\n  \"schema\": \"resilient-contract-certificate/v1\",\n  \"schema_version\": 1,\n"
        ));
        assert!(json.contains("\"source\": \"<test>\""));
        assert!(json.contains("\"name\": \"div\""));
        assert!(json.contains("\"kind\": \"requires\""));
        assert!(json.contains("\"kind\": \"ensures\""));
        assert!(
            json.contains("\"clause\": \"(b != 0)\"") || json.contains("\"clause\": \"b != 0\"")
        );
        assert!(json.ends_with("}\n"));
    }

    #[test]
    fn certificate_is_deterministic() {
        let src = "fn f(int x) requires x >= 0 ensures result >= 0 { return x; }\nfn g(int y) requires y > 0 { return y; }";
        assert_eq!(emit_for(src), emit_for(src));
    }

    #[test]
    fn provenance_is_informational_only() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) requires x >= 0 ensures result >= 0 { return x; }";
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let untagged = emit(&prog, "<test>");

        crate::feature_attrs::record(
            "f",
            crate::feature_attrs::AttrRecord {
                name: "ai_generated".into(),
                args: String::new(),
                line: 1,
            },
        );
        let tagged = emit(&prog, "<test>");
        crate::feature_attrs::reset();

        assert!(untagged.contains("\"provenance\": []"));
        assert!(tagged.contains("\"provenance\": [\"ai_generated\"]"));
        // The tag must change nothing beyond the informational fields:
        // strip the provenance and enrolment lines and the documents
        // are identical.
        let strip = |s: &str| {
            s.lines()
                .filter(|l| !l.contains("\"provenance\"") && !l.contains("\"enrolled\""))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(strip(&untagged), strip(&tagged));
    }

    #[test]
    fn functions_without_contracts_have_empty_clauses() {
        let json = emit_for("fn plain(int x) { return x; }");
        assert!(json.contains("\"name\": \"plain\""));
        assert!(json.contains("\"enrolled\": false"));
    }

    #[cfg(not(feature = "z3"))]
    #[test]
    fn non_z3_build_reports_unknown_golden() {
        let json = emit_for("fn f(int x) requires x >= 0 ensures result >= 0 { return x; }");
        let expected = "{\n  \"schema\": \"resilient-contract-certificate/v1\",\n  \"schema_version\": 1,\n  \"source\": \"<test>\",\n  \"functions\": [\n    {\n      \"name\": \"f\",\n      \"enrolled\": false,\n      \"provenance\": [],\n      \"clauses\": [\n";
        assert!(json.starts_with(expected), "unexpected prefix:\n{json}");
        // Without z3 every clause is Unknown and carries no SMT dump.
        assert!(json.contains("\"verdict\": \"unknown\""));
        assert!(!json.contains("\"verdict\": \"pass\""));
        assert!(!json.contains("\"smtlib2\""));
    }

    #[cfg(feature = "z3")]
    #[test]
    fn z3_build_attests_discharged_tautology() {
        // `x >= 0 || x < 0` is a tautology — Z3 discharges it and the
        // certificate must attest the proof with a replayable dump.
        let json =
            emit_for("fn f(int x) requires x >= 0 || x < 0 ensures result == x { return x; }");
        assert!(
            json.contains("\"verdict\": \"pass\""),
            "expected a pass verdict:\n{json}"
        );
        assert!(
            json.contains("\"smtlib2\": \""),
            "pass verdict should carry the SMT-LIB2 certificate:\n{json}"
        );
    }

    // ---- C-E5: trust-model hardening ----

    #[test]
    fn emitted_certificate_carries_current_schema_version() {
        let json = emit_for("fn plain(int x) { return x; }");
        assert_eq!(verify_schema_version(&json), Ok(SCHEMA_VERSION));
    }

    #[test]
    fn verify_schema_version_rejects_missing_field() {
        assert_eq!(
            verify_schema_version(r#"{"schema": "resilient-contract-certificate/v1"}"#),
            Err(VerifyError::MissingSchemaVersion)
        );
    }

    #[test]
    fn verify_schema_version_rejects_unsupported_version() {
        assert_eq!(
            verify_schema_version(r#"{"schema_version": 99}"#),
            Err(VerifyError::UnsupportedSchemaVersion(99))
        );
    }

    #[test]
    fn verify_schema_version_rejects_malformed_json_without_panicking() {
        assert!(matches!(
            verify_schema_version("not json at all"),
            Err(VerifyError::InvalidJson(_))
        ));
        assert!(matches!(
            verify_schema_version(""),
            Err(VerifyError::InvalidJson(_))
        ));
    }

    #[test]
    fn verify_error_messages_are_non_empty() {
        // Cheap smoke check that Display never panics and says
        // something for every variant a consumer can hit without z3.
        assert!(!VerifyError::InvalidJson("x".into()).to_string().is_empty());
        assert!(!VerifyError::MissingSchemaVersion.to_string().is_empty());
        assert!(
            !VerifyError::UnsupportedSchemaVersion(7)
                .to_string()
                .is_empty()
        );
    }

    #[cfg(feature = "z3")]
    mod z3_signing {
        use super::*;
        use ed25519_dalek::SigningKey;
        use rand_core::OsRng;

        fn fresh_keypair() -> (
            [u8; ed25519_dalek::SECRET_KEY_LENGTH],
            [u8; ed25519_dalek::PUBLIC_KEY_LENGTH],
        ) {
            let sk = SigningKey::generate(&mut OsRng);
            let pk = sk.verifying_key();
            (sk.to_bytes(), pk.to_bytes())
        }

        #[test]
        fn sign_then_verify_round_trip() {
            let json = emit_for(
                "fn div(int a, int b) requires b != 0 ensures result == a / b { return a / b; }",
            );
            let (priv_b, pub_b) = fresh_keypair();
            let sig = sign_bytes(&priv_b, json.as_bytes());
            assert_eq!(verify_signed(json.as_bytes(), &sig, &pub_b), Ok(()));
        }

        #[test]
        fn verify_signed_rejects_wrong_public_key() {
            let json = emit_for("fn f(int x) requires x >= 0 { return x; }");
            let (priv_b, _pub_b) = fresh_keypair();
            let (_other_priv, other_pub) = fresh_keypair();
            let sig = sign_bytes(&priv_b, json.as_bytes());
            assert_eq!(
                verify_signed(json.as_bytes(), &sig, &other_pub),
                Err(VerifyError::SignatureMismatch)
            );
        }

        #[test]
        fn verify_signed_rejects_unsupported_schema_version_before_checking_signature() {
            // A document with a bad schema_version is rejected on that
            // basis even if it happens to carry a validly-formed
            // signature over its (unsupported) bytes.
            let json = r#"{"schema_version": 2, "functions": []}"#;
            let (priv_b, pub_b) = fresh_keypair();
            let sig = sign_bytes(&priv_b, json.as_bytes());
            assert_eq!(
                verify_signed(json.as_bytes(), &sig, &pub_b),
                Err(VerifyError::UnsupportedSchemaVersion(2))
            );
        }

        /// C-E5 key deliverable: a certificate that verifies, tampered
        /// at ANY single byte position (every bit of every byte, not a
        /// sample), must NEVER verify again. This is what makes the
        /// certificate tamper-evident rather than merely
        /// tamper-detectable-by-luck.
        #[test]
        fn signature_rejects_every_single_byte_corruption() {
            let json = emit_for(
                "fn max(int a, int b) requires true ensures result >= a && result >= b { \
                 if (a >= b) { return a; } return b; }",
            );
            let bytes = json.as_bytes();
            assert!(!bytes.is_empty(), "sanity: certificate must be non-empty");

            let (priv_b, pub_b) = fresh_keypair();
            let sig = sign_bytes(&priv_b, bytes);
            assert_eq!(
                verify_signed(bytes, &sig, &pub_b),
                Ok(()),
                "untampered certificate must verify"
            );

            for i in 0..bytes.len() {
                for bit in 0..8u8 {
                    let mut tampered = bytes.to_vec();
                    tampered[i] ^= 1 << bit;
                    assert_ne!(tampered, bytes, "flip must actually change the byte");
                    assert!(
                        verify_signed(&tampered, &sig, &pub_b).is_err(),
                        "byte {i} bit {bit} corruption must be rejected, but verified \
                         successfully"
                    );
                }
            }
        }

        #[test]
        fn signature_rejects_truncation_and_extension() {
            let json = emit_for("fn f(int x) requires x >= 0 { return x; }");
            let bytes = json.as_bytes();
            let (priv_b, pub_b) = fresh_keypair();
            let sig = sign_bytes(&priv_b, bytes);

            let truncated = &bytes[..bytes.len() - 1];
            assert!(verify_signed(truncated, &sig, &pub_b).is_err());

            let mut extended = bytes.to_vec();
            extended.push(b'\n');
            assert!(verify_signed(&extended, &sig, &pub_b).is_err());
        }
    }
}
