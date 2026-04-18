//! RES-194: Ed25519 signatures on RES-071 verification certificates.
//!
//! Layout on disk (one cert directory per `--emit-certificate` run):
//!
//!   <cert_dir>/
//!     foo__requires__0.smt2
//!     foo__ensures__0.smt2
//!     bar__invariant__2.smt2
//!     cert.sig              <- 64-byte Ed25519 signature, hex-encoded
//!
//! The signed payload is a byte-for-byte concatenation of the
//! `.smt2` files in lexicographic filename order, separated by a
//! single `\n` per file (so a directory with different ordering
//! can be round-tripped). See `compute_cert_payload`.
//!
//! PEM format in this module is intentionally minimal — we don't
//! need PKCS#8 / SPKI interop, just a convention that's easy to
//! read by eye and trivial to parse without pulling in `pem` /
//! `rustls-pki-types`:
//!
//!   -----BEGIN ED25519 PUBLIC KEY-----
//!   <64 hex chars for 32 bytes>
//!   -----END ED25519 PUBLIC KEY-----
//!
//! Private keys use `ED25519 PRIVATE KEY` markers. The payload
//! is raw-bytes hex (no ASN.1 wrapping).

use std::fs;
use std::path::Path;

use ed25519_dalek::{Signature, SigningKey, Verifier, VerifyingKey};
use ed25519_dalek::{PUBLIC_KEY_LENGTH, SECRET_KEY_LENGTH, SIGNATURE_LENGTH};
use ed25519_dalek::ed25519::signature::Signer;

/// RES-194: the public key committed alongside the binary. The
/// corresponding private key lives off-repo on the signing
/// pipeline's machine. `verify-cert` consults this for the
/// default verification path.
///
/// When swapping the key in production, update the file and bump
/// a follow-up ticket for the key-rotation story (noted in the
/// ticket's Notes — deliberately deferred).
pub const EMBEDDED_PUBLIC_KEY_PEM: &str = include_str!("cert_key.pem");

/// RES-194: PEM begin/end marker for the public key file.
const PEM_PUB_BEGIN: &str = "-----BEGIN ED25519 PUBLIC KEY-----";
const PEM_PUB_END: &str = "-----END ED25519 PUBLIC KEY-----";
/// RES-194: PEM begin/end marker for the private key file.
const PEM_PRIV_BEGIN: &str = "-----BEGIN ED25519 PRIVATE KEY-----";
const PEM_PRIV_END: &str = "-----END ED25519 PRIVATE KEY-----";

/// RES-194: build the signed payload from the cert directory.
///
/// We walk the directory once, sort entries by filename, and
/// concatenate their bytes with a single `\n` between files.
/// Non-`.smt2` files are ignored — that keeps an accidental
/// `.gitignore` or README in the directory from invalidating the
/// signature after the fact. `cert.sig` is specifically excluded
/// too (signing the signature would be a chicken-and-egg issue).
pub fn compute_cert_payload(dir: &Path) -> Result<Vec<u8>, String> {
    let rd = fs::read_dir(dir).map_err(|e| {
        format!("could not read cert directory {}: {}", dir.display(), e)
    })?;
    let mut entries: Vec<_> = rd
        .filter_map(|r| r.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .ends_with(".smt2")
        })
        .map(|e| e.path())
        .collect();
    entries.sort();

    let mut payload = Vec::new();
    for (i, p) in entries.iter().enumerate() {
        let body = fs::read(p).map_err(|e| {
            format!("could not read cert file {}: {}", p.display(), e)
        })?;
        if i > 0 {
            payload.push(b'\n');
        }
        payload.extend_from_slice(&body);
    }
    Ok(payload)
}

/// RES-194: sign `payload` with the 32-byte Ed25519 private key
/// and return the 64-byte raw signature. `priv_key_bytes` is the
/// SECRET_KEY part only — not the 64-byte SigningKey layout.
pub fn sign_payload(
    priv_key_bytes: &[u8; SECRET_KEY_LENGTH],
    payload: &[u8],
) -> [u8; SIGNATURE_LENGTH] {
    let signing_key = SigningKey::from_bytes(priv_key_bytes);
    let sig: Signature = signing_key.sign(payload);
    sig.to_bytes()
}

/// RES-194: verify `payload` against `sig` with `pub_key_bytes`.
/// Returns `true` on a valid signature, `false` otherwise.
/// Parse errors on the public key propagate as `Err`; tamper /
/// bad-signature simply returns `Ok(false)`.
pub fn verify_payload(
    pub_key_bytes: &[u8; PUBLIC_KEY_LENGTH],
    payload: &[u8],
    sig: &[u8; SIGNATURE_LENGTH],
) -> Result<bool, String> {
    let vk = VerifyingKey::from_bytes(pub_key_bytes)
        .map_err(|e| format!("invalid public key: {}", e))?;
    let signature = Signature::from_bytes(sig);
    Ok(vk.verify(payload, &signature).is_ok())
}

/// RES-194: parse our mini-PEM for a public key. Accepts an
/// ASCII file with the `BEGIN ED25519 PUBLIC KEY` markers and a
/// hex-encoded 32-byte body. Whitespace is ignored.
pub fn parse_public_key_pem(pem: &str) -> Result<[u8; PUBLIC_KEY_LENGTH], String> {
    parse_hex_pem(pem, PEM_PUB_BEGIN, PEM_PUB_END, PUBLIC_KEY_LENGTH)
        .map(|v| {
            let mut out = [0u8; PUBLIC_KEY_LENGTH];
            out.copy_from_slice(&v);
            out
        })
}

/// RES-194: parse our mini-PEM for a private key.
pub fn parse_private_key_pem(pem: &str) -> Result<[u8; SECRET_KEY_LENGTH], String> {
    parse_hex_pem(pem, PEM_PRIV_BEGIN, PEM_PRIV_END, SECRET_KEY_LENGTH)
        .map(|v| {
            let mut out = [0u8; SECRET_KEY_LENGTH];
            out.copy_from_slice(&v);
            out
        })
}

/// Internal: shared mini-PEM parser for public + private keys.
/// `expected_len` is the decoded raw-bytes length (32 for
/// Ed25519).
fn parse_hex_pem(
    pem: &str,
    begin: &str,
    end: &str,
    expected_len: usize,
) -> Result<Vec<u8>, String> {
    let begin_idx = pem
        .find(begin)
        .ok_or_else(|| format!("missing `{}` marker", begin))?;
    let after_begin = begin_idx + begin.len();
    let end_idx = pem[after_begin..]
        .find(end)
        .ok_or_else(|| format!("missing `{}` marker", end))?;
    let body = &pem[after_begin..after_begin + end_idx];
    // Strip whitespace.
    let hex: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    if hex.len() != expected_len * 2 {
        return Err(format!(
            "expected {} hex chars ({}-byte key), got {}",
            expected_len * 2, expected_len, hex.len()
        ));
    }
    hex_decode(&hex)
}

/// RES-194: write the 32-byte public key out in our mini-PEM.
/// Used by tests + external tooling that wants to export the
/// public half of a keypair; the production binary only ever
/// reads `cert_key.pem` (via `include_str!`) — it doesn't write.
#[allow(dead_code)]
pub fn format_public_key_pem(pub_key: &[u8; PUBLIC_KEY_LENGTH]) -> String {
    format!(
        "{}\n{}\n{}\n",
        PEM_PUB_BEGIN,
        hex_encode(pub_key),
        PEM_PUB_END,
    )
}

/// RES-194: write the 32-byte private key out in our mini-PEM.
/// Used by the test-helpers + future key-rotation story — the
/// production binary itself never emits a private key.
#[allow(dead_code)]
pub fn format_private_key_pem(priv_key: &[u8; SECRET_KEY_LENGTH]) -> String {
    format!(
        "{}\n{}\n{}\n",
        PEM_PRIV_BEGIN,
        hex_encode(priv_key),
        PEM_PRIV_END,
    )
}

/// RES-194: write the 64-byte signature as a hex string — the
/// `cert.sig` file's only contents.
pub fn format_signature_hex(sig: &[u8; SIGNATURE_LENGTH]) -> String {
    hex_encode(sig)
}

/// RES-194: parse a `cert.sig` file's contents — a hex string of
/// exactly 128 chars, ignoring leading/trailing whitespace.
pub fn parse_signature_hex(s: &str) -> Result<[u8; SIGNATURE_LENGTH], String> {
    let hex: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if hex.len() != SIGNATURE_LENGTH * 2 {
        return Err(format!(
            "expected {} hex chars (64-byte signature), got {}",
            SIGNATURE_LENGTH * 2, hex.len()
        ));
    }
    let v = hex_decode(&hex)?;
    let mut out = [0u8; SIGNATURE_LENGTH];
    out.copy_from_slice(&v);
    Ok(out)
}

// ---- Tiny hex codec (keep the dep tree small) ----

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(*b >> 4) as usize] as char);
        out.push(HEX[(*b & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err("odd-length hex string".to_string());
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let chars: Vec<char> = s.chars().collect();
    for i in (0..chars.len()).step_by(2) {
        let hi = hex_val(chars[i])?;
        let lo = hex_val(chars[i + 1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_val(c: char) -> Result<u8, String> {
    match c {
        '0'..='9' => Ok(c as u8 - b'0'),
        'a'..='f' => Ok(c as u8 - b'a' + 10),
        'A'..='F' => Ok(c as u8 - b'A' + 10),
        _ => Err(format!("invalid hex character `{}`", c)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand_core::OsRng;

    fn fresh_keypair() -> ([u8; SECRET_KEY_LENGTH], [u8; PUBLIC_KEY_LENGTH]) {
        let sk = SigningKey::generate(&mut OsRng);
        let pk = sk.verifying_key();
        (sk.to_bytes(), pk.to_bytes())
    }

    #[test]
    fn sign_then_verify_round_trip() {
        let (priv_b, pub_b) = fresh_keypair();
        let payload = b"(assert (> x 0))\n";
        let sig = sign_payload(&priv_b, payload);
        assert!(
            verify_payload(&pub_b, payload, &sig).unwrap(),
            "fresh signature must verify"
        );
    }

    #[test]
    fn tamper_detection_on_payload() {
        let (priv_b, pub_b) = fresh_keypair();
        let payload = b"(assert (> x 0))\n";
        let sig = sign_payload(&priv_b, payload);
        // Flip one byte in the payload.
        let mut tampered = payload.to_vec();
        tampered[3] ^= 0x01;
        assert!(
            !verify_payload(&pub_b, &tampered, &sig).unwrap(),
            "altered payload must NOT verify"
        );
    }

    #[test]
    fn tamper_detection_on_signature() {
        let (priv_b, pub_b) = fresh_keypair();
        let payload = b"(assert (> x 0))\n";
        let mut sig = sign_payload(&priv_b, payload);
        sig[0] ^= 0x01; // flip a bit in the sig
        assert!(
            !verify_payload(&pub_b, payload, &sig).unwrap(),
            "altered signature must NOT verify"
        );
    }

    #[test]
    fn tamper_detection_on_public_key() {
        let (priv_b, _pub_b) = fresh_keypair();
        let payload = b"data";
        let sig = sign_payload(&priv_b, payload);
        // Use a DIFFERENT public key — signature won't verify.
        let (_priv2, pub2) = fresh_keypair();
        assert!(
            !verify_payload(&pub2, payload, &sig).unwrap(),
            "wrong public key must NOT verify"
        );
    }

    #[test]
    fn pem_round_trip_public_key() {
        let (_priv, pub_b) = fresh_keypair();
        let pem = format_public_key_pem(&pub_b);
        let parsed = parse_public_key_pem(&pem).expect("parse round-trip");
        assert_eq!(parsed, pub_b);
    }

    #[test]
    fn pem_round_trip_private_key() {
        let (priv_b, _pub) = fresh_keypair();
        let pem = format_private_key_pem(&priv_b);
        let parsed = parse_private_key_pem(&pem).expect("parse round-trip");
        assert_eq!(parsed, priv_b);
    }

    #[test]
    fn pem_rejects_missing_begin_marker() {
        let err = parse_public_key_pem("nope").unwrap_err();
        assert!(err.contains("BEGIN"), "error was: {err}");
    }

    #[test]
    fn pem_rejects_wrong_length() {
        let bad = format!(
            "{}\n{}\n{}\n",
            PEM_PUB_BEGIN, "aabb", PEM_PUB_END
        );
        let err = parse_public_key_pem(&bad).unwrap_err();
        assert!(err.contains("hex chars"), "error was: {err}");
    }

    #[test]
    fn signature_hex_round_trip() {
        let (priv_b, _pub) = fresh_keypair();
        let sig = sign_payload(&priv_b, b"x");
        let hex = format_signature_hex(&sig);
        let parsed = parse_signature_hex(&hex).expect("parse");
        assert_eq!(parsed, sig);
    }

    #[test]
    fn signature_hex_rejects_odd_length() {
        assert!(parse_signature_hex("abc").is_err());
    }

    #[test]
    fn embedded_public_key_parses() {
        // The committed `cert_key.pem` must always be valid.
        parse_public_key_pem(EMBEDDED_PUBLIC_KEY_PEM)
            .expect("embedded key must parse");
    }

    #[test]
    fn compute_cert_payload_walks_dir_sorted() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "res_certsign_{}_{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        // Two files, written in reverse alphabetical order — the
        // payload must still sort them.
        fs::write(dir.join("b__requires__0.smt2"), b"BBBB").unwrap();
        fs::write(dir.join("a__ensures__0.smt2"), b"AAAA").unwrap();
        // A non-.smt2 file must be ignored.
        fs::write(dir.join("readme.txt"), b"ignore me").unwrap();
        let payload = compute_cert_payload(&dir).expect("payload");
        assert_eq!(payload, b"AAAA\nBBBB");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sign_and_verify_directory_payload() {
        // End-to-end at the API level: sign the concatenated payload
        // of a real cert directory, then verify with the
        // corresponding public key.
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "res_certsign_e2e_{}_{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("foo__requires__0.smt2"), b"(assert true)\n").unwrap();
        fs::write(dir.join("foo__ensures__0.smt2"), b"(assert false)\n").unwrap();

        let (priv_b, pub_b) = fresh_keypair();
        let payload = compute_cert_payload(&dir).expect("payload");
        let sig = sign_payload(&priv_b, &payload);
        assert!(verify_payload(&pub_b, &payload, &sig).unwrap());

        // Tamper: rewrite one file, recompute payload, verify
        // should now fail.
        fs::write(dir.join("foo__requires__0.smt2"), b"(assert maybe)\n").unwrap();
        let tampered = compute_cert_payload(&dir).expect("payload");
        assert!(!verify_payload(&pub_b, &tampered, &sig).unwrap());

        let _ = fs::remove_dir_all(&dir);
    }
}
