//! RES-2560: SHA-256 and SHA-512 cryptographic hash builtins.
//! RES-2561: CRC-32 and CRC-16 checksum builtins.
//!
//! SHA-256 and SHA-512 are implemented via the `sha2` crate (already in Cargo.toml).
//! CRC-32 and CRC-16 are implemented in-tree (no external dependency).
//!
//! ## SHA-256 / SHA-512
//!
//!   sha256(bytes)   → string  — hex digest of bytes value
//!   sha256_str(s)   → string  — hex digest of UTF-8 string
//!   sha512(bytes)   → string  — hex digest of bytes value
//!   sha512_str(s)   → string  — hex digest of UTF-8 string
//!
//! ## CRC checksums
//!
//!   crc32(bytes)    → int     — CRC-32/ISO-HDLC checksum
//!   crc32_str(s)    → int     — CRC-32 of UTF-8 string
//!   crc16(bytes)    → int     — CRC-16/CCITT-FALSE checksum
//!   crc16_str(s)    → int     — CRC-16 of UTF-8 string

use sha2::{Digest, Sha256, Sha512};

use crate::Value;

type RResult<T> = Result<T, String>;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_raw(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

fn sha512_raw(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha512::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

// ---------------------------------------------------------------------------
// SHA-256 builtins
// ---------------------------------------------------------------------------

pub(crate) fn builtin_sha256(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b)] => Ok(Value::String(hex_encode(&sha256_raw(b)))),
        [_] => Err("sha256: expected Bytes argument".to_string()),
        _ => Err(format!("sha256: expected 1 argument, got {}", args.len())),
    }
}

pub(crate) fn builtin_sha256_str(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => Ok(Value::String(hex_encode(&sha256_raw(s.as_bytes())))),
        [_] => Err("sha256_str: expected string argument".to_string()),
        _ => Err(format!(
            "sha256_str: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ---------------------------------------------------------------------------
// SHA-512 builtins
// ---------------------------------------------------------------------------

pub(crate) fn builtin_sha512(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b)] => Ok(Value::String(hex_encode(&sha512_raw(b)))),
        [_] => Err("sha512: expected Bytes argument".to_string()),
        _ => Err(format!("sha512: expected 1 argument, got {}", args.len())),
    }
}

pub(crate) fn builtin_sha512_str(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => Ok(Value::String(hex_encode(&sha512_raw(s.as_bytes())))),
        [_] => Err("sha512_str: expected string argument".to_string()),
        _ => Err(format!(
            "sha512_str: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ---------------------------------------------------------------------------
// CRC-32 (ISO-HDLC / Ethernet / ZIP polynomial 0xEDB88320)
// ---------------------------------------------------------------------------

fn make_crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            if c & 1 != 0 {
                c = 0xEDB88320 ^ (c >> 1);
            } else {
                c >>= 1;
            }
        }
        *entry = c;
    }
    table
}

fn crc32_raw(data: &[u8]) -> u32 {
    let table = make_crc32_table();
    let mut crc: u32 = 0xFFFFFFFF;
    for &byte in data {
        let idx = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = (crc >> 8) ^ table[idx];
    }
    crc ^ 0xFFFFFFFF
}

pub(crate) fn builtin_crc32(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b)] => Ok(Value::Int(crc32_raw(b) as i64)),
        [_] => Err("crc32: expected Bytes argument".to_string()),
        _ => Err(format!("crc32: expected 1 argument, got {}", args.len())),
    }
}

pub(crate) fn builtin_crc32_str(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => Ok(Value::Int(crc32_raw(s.as_bytes()) as i64)),
        [_] => Err("crc32_str: expected string argument".to_string()),
        _ => Err(format!(
            "crc32_str: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ---------------------------------------------------------------------------
// CRC-16 (CCITT-FALSE / initial value 0xFFFF, polynomial 0x1021)
// ---------------------------------------------------------------------------

fn crc16_raw(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

pub(crate) fn builtin_crc16(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b)] => Ok(Value::Int(crc16_raw(b) as i64)),
        [_] => Err("crc16: expected Bytes argument".to_string()),
        _ => Err(format!("crc16: expected 1 argument, got {}", args.len())),
    }
}

pub(crate) fn builtin_crc16_str(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => Ok(Value::Int(crc16_raw(s.as_bytes()) as i64)),
        [_] => Err("crc16_str: expected string argument".to_string()),
        _ => Err(format!(
            "crc16_str: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ---------------------------------------------------------------------------
// Advisory check pass (no-op)
// ---------------------------------------------------------------------------

pub(crate) fn check(_program: &crate::Node, _source_path: &str) -> Result<(), String> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sha256_of(s: &str) -> String {
        hex_encode(&sha256_raw(s.as_bytes()))
    }

    fn sha512_of(s: &str) -> String {
        hex_encode(&sha512_raw(s.as_bytes()))
    }

    #[test]
    fn sha256_empty_string() {
        assert_eq!(
            sha256_of(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_abc() {
        assert_eq!(
            sha256_of("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha512_empty_string() {
        assert_eq!(
            sha512_of(""),
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
        );
    }

    #[test]
    fn sha512_abc() {
        assert_eq!(
            sha512_of("abc"),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
    }

    #[test]
    fn crc32_empty() {
        assert_eq!(crc32_raw(b""), 0x00000000);
    }

    #[test]
    fn crc32_known() {
        assert_eq!(crc32_raw(b"123456789"), 0xCBF43926);
    }

    #[test]
    fn crc16_known() {
        assert_eq!(crc16_raw(b"123456789"), 0x29B1);
    }

    #[test]
    fn interpreter_sha256_str() {
        use crate::run_program;
        let r = run_program(r#"println(sha256_str("abc"));"#);
        assert!(r.ok, "{:?}", r.errors);
        assert!(
            r.stdout.contains("ba7816bf"),
            "expected sha256 of abc, got: {}",
            r.stdout
        );
    }

    #[test]
    fn interpreter_crc32_str() {
        use crate::run_program;
        let r = run_program(r#"println(to_string(crc32_str("123456789")));"#);
        assert!(r.ok, "{:?}", r.errors);
        assert!(!r.stdout.trim().is_empty(), "expected output");
    }
}
