//! RES-1162: deterministic hash builtins.
//!
//! Four pure leaf builtins providing a stable, deterministic hash
//! surface for custom hash-tables / bloom filters / fingerprinting /
//! consistent hashing. Output is **stable across runs and machines** —
//! unlike Rust's `DefaultHasher` which uses a randomized seed.
//!
//! | Builtin | Algorithm | Purpose |
//! |---|---|---|
//! | `hash_int(n)`         | SplitMix64 | Avalanche-mix an i64 |
//! | `hash_string(s)`      | FNV-1a 64  | Hash a UTF-8 string |
//! | `hash_bytes(b)`       | FNV-1a 64  | Hash a Bytes (same alg) |
//! | `hash_combine(h1, h2)`| boost::hash_combine | Fold two hashes |
//!
//! All four return Int (i64). The underlying u64 is reinterpreted via
//! bit-cast, so half the outputs are negative — same convention as
//! `float_to_bits` (RES-1130).
//!
//! **Not cryptographically secure** — use the FFI ed25519 pathway for
//! security-sensitive hashing.

use crate::{RResult, Value};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// `hash_int(n) -> Int` — SplitMix64 avalanche mix. Same input always
/// returns the same output across runs / machines.
pub(crate) fn builtin_hash_int(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(n)] => Ok(Value::Int(splitmix64(*n as u64) as i64)),
        [other] => Err(format!("hash_int: expected Int, got {}", other)),
        _ => Err(format!("hash_int: expected 1 argument, got {}", args.len())),
    }
}

/// `hash_string(s) -> Int` — FNV-1a 64-bit hash of the UTF-8 bytes.
pub(crate) fn builtin_hash_string(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => Ok(Value::Int(fnv1a(s.as_bytes()) as i64)),
        [other] => Err(format!("hash_string: expected String, got {}", other)),
        _ => Err(format!(
            "hash_string: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `hash_bytes(b) -> Int` — FNV-1a 64-bit hash of the raw bytes.
/// Same hash as `hash_string` on a UTF-8 encoded string with the same
/// byte content.
pub(crate) fn builtin_hash_bytes(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b)] => Ok(Value::Int(fnv1a(b) as i64)),
        [other] => Err(format!("hash_bytes: expected Bytes, got {}", other)),
        _ => Err(format!(
            "hash_bytes: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `hash_combine(h1, h2) -> Int` — fold two hashes into a single
/// combined value using the boost::hash_combine recipe:
/// `h1 ^ (h2 + 0x9E3779B9 + (h1 << 6) + (h1 >> 2))`.
pub(crate) fn builtin_hash_combine(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(h1), Value::Int(h2)] => {
            let a = *h1 as u64;
            let b = *h2 as u64;
            let combined = a ^ b
                .wrapping_add(0x9E37_79B9)
                .wrapping_add(a << 6)
                .wrapping_add(a >> 2);
            Ok(Value::Int(combined as i64))
        }
        [a, b] => Err(format!(
            "hash_combine: expected (Int, Int), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "hash_combine: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn as_int(v: Value) -> i64 {
        match v {
            Value::Int(n) => n,
            other => panic!("expected Int, got {:?}", other),
        }
    }

    // --- hash_int ---

    #[test]
    fn hash_int_deterministic() {
        // Same input → same output.
        for n in [0i64, 1, -1, 42, i64::MAX, i64::MIN] {
            let h1 = as_int(builtin_hash_int(&[Value::Int(n)]).unwrap());
            let h2 = as_int(builtin_hash_int(&[Value::Int(n)]).unwrap());
            assert_eq!(h1, h2, "hash_int({}) is non-deterministic", n);
        }
    }

    #[test]
    fn hash_int_distinguishes_inputs() {
        // Sequential inputs must produce distinct hashes (with extremely
        // high probability under SplitMix64 avalanche).
        let a = as_int(builtin_hash_int(&[Value::Int(0)]).unwrap());
        let b = as_int(builtin_hash_int(&[Value::Int(1)]).unwrap());
        let c = as_int(builtin_hash_int(&[Value::Int(2)]).unwrap());
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
    }

    #[test]
    fn hash_int_avalanche() {
        // 1-bit change in input should produce a very different hash.
        // We can't easily verify the strict avalanche criterion in a
        // unit test, but we can check non-equality on adjacent inputs.
        let zero = as_int(builtin_hash_int(&[Value::Int(0)]).unwrap());
        let one = as_int(builtin_hash_int(&[Value::Int(1)]).unwrap());
        // Hamming distance should be substantial — at least more than
        // one bit. We approximate by comparing absolute difference.
        assert_ne!(zero, one);
        // Differ by at least a high-order bit (not just LSB).
        assert_ne!(zero.wrapping_sub(one).abs(), 1);
    }

    #[test]
    fn hash_int_known_values() {
        // SplitMix64(0) is a well-known constant. Lock the value down to
        // catch any change in the underlying algorithm.
        let h = as_int(builtin_hash_int(&[Value::Int(0)]).unwrap());
        // SplitMix64(0) = 0xE220A8397B1DCDAF as u64, cast to i64.
        assert_eq!(h, 0xE220_A839_7B1D_CDAFu64 as i64);
    }

    // --- hash_string ---

    #[test]
    fn hash_string_deterministic() {
        for s in ["", "hello", "Hello, World!", "🌟"] {
            let h1 = as_int(builtin_hash_string(&[Value::String(s.to_string())]).unwrap());
            let h2 = as_int(builtin_hash_string(&[Value::String(s.to_string())]).unwrap());
            assert_eq!(h1, h2);
        }
    }

    #[test]
    fn hash_string_empty_is_fnv_offset() {
        // Empty input must produce the FNV-1a offset basis (cast to i64).
        let h = as_int(builtin_hash_string(&[Value::String("".to_string())]).unwrap());
        assert_eq!(h, FNV_OFFSET as i64);
    }

    #[test]
    fn hash_string_distinguishes_inputs() {
        let a = as_int(builtin_hash_string(&[Value::String("a".to_string())]).unwrap());
        let b = as_int(builtin_hash_string(&[Value::String("b".to_string())]).unwrap());
        let ab = as_int(builtin_hash_string(&[Value::String("ab".to_string())]).unwrap());
        assert_ne!(a, b);
        assert_ne!(a, ab);
        assert_ne!(b, ab);
    }

    #[test]
    fn hash_string_rejects_non_string() {
        let err = builtin_hash_string(&[Value::Int(0)]).unwrap_err();
        assert!(err.contains("expected String"));
    }

    // --- hash_bytes ---

    #[test]
    fn hash_bytes_matches_hash_string_on_same_bytes() {
        let s = "hello";
        let str_hash = as_int(builtin_hash_string(&[Value::String(s.to_string())]).unwrap());
        let bytes_hash =
            as_int(builtin_hash_bytes(&[Value::Bytes(s.as_bytes().to_vec())]).unwrap());
        assert_eq!(str_hash, bytes_hash);
    }

    #[test]
    fn hash_bytes_empty() {
        let h = as_int(builtin_hash_bytes(&[Value::Bytes(vec![])]).unwrap());
        assert_eq!(h, FNV_OFFSET as i64);
    }

    #[test]
    fn hash_bytes_single_byte_avalanches() {
        let a = as_int(builtin_hash_bytes(&[Value::Bytes(vec![0])]).unwrap());
        let b = as_int(builtin_hash_bytes(&[Value::Bytes(vec![1])]).unwrap());
        assert_ne!(a, b);
    }

    // --- hash_combine ---

    #[test]
    fn hash_combine_deterministic() {
        let h = as_int(builtin_hash_combine(&[Value::Int(7), Value::Int(13)]).unwrap());
        let h2 = as_int(builtin_hash_combine(&[Value::Int(7), Value::Int(13)]).unwrap());
        assert_eq!(h, h2);
    }

    #[test]
    fn hash_combine_is_not_commutative() {
        // h(a, b) != h(b, a) in general — boost::hash_combine is not
        // symmetric. This is a feature; ordered structures want order.
        let ab = as_int(builtin_hash_combine(&[Value::Int(7), Value::Int(13)]).unwrap());
        let ba = as_int(builtin_hash_combine(&[Value::Int(13), Value::Int(7)]).unwrap());
        assert_ne!(ab, ba);
    }

    #[test]
    fn hash_combine_distinguishes_pairs() {
        let h1 = as_int(builtin_hash_combine(&[Value::Int(1), Value::Int(2)]).unwrap());
        let h2 = as_int(builtin_hash_combine(&[Value::Int(1), Value::Int(3)]).unwrap());
        let h3 = as_int(builtin_hash_combine(&[Value::Int(2), Value::Int(2)]).unwrap());
        assert_ne!(h1, h2);
        assert_ne!(h1, h3);
    }

    // --- general ---

    #[test]
    fn arity_diagnostics_consistent() {
        let err = builtin_hash_int(&[]).unwrap_err();
        assert!(err.contains("expected 1"));
        let err = builtin_hash_string(&[]).unwrap_err();
        assert!(err.contains("expected 1"));
        let err = builtin_hash_bytes(&[]).unwrap_err();
        assert!(err.contains("expected 1"));
        let err = builtin_hash_combine(&[Value::Int(0)]).unwrap_err();
        assert!(err.contains("expected 2"));
    }

    #[test]
    fn type_diagnostics_consistent() {
        let err = builtin_hash_int(&[Value::String("x".to_string())]).unwrap_err();
        assert!(err.contains("expected Int"));
        let err = builtin_hash_string(&[Value::Int(0)]).unwrap_err();
        assert!(err.contains("expected String"));
        let err = builtin_hash_bytes(&[Value::Int(0)]).unwrap_err();
        assert!(err.contains("expected Bytes"));
        let err =
            builtin_hash_combine(&[Value::Int(0), Value::String("x".to_string())]).unwrap_err();
        assert!(err.contains("expected (Int, Int)"));
    }
}
