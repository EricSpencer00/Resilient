//! Comprehensive edge-case tests for integer width-cast builtins.
//! Tests cover: as_int8, as_int16, as_int32, as_int64,
//!              as_uint8, as_uint16, as_uint32, as_uint64
//!
//! Semantics: wrapping truncation with sign/zero extension back to i64.
//! E.g., as_int8(300) = (300 as i8 as i64) = 44
//!       as_uint8(-1) = ((-1 as u8) as i64) = 255

fn run_ok(src: &str) -> String {
    let result = resilient::run_program(src);
    assert!(
        result.ok,
        "program failed: {:?}\nCode:\n{}",
        result.errors, src
    );
    result.stdout
}

// ============================================================================
// as_int8 tests (signed 8-bit: -128 to 127)
// ============================================================================

#[test]
fn as_int8_zero() {
    assert_eq!(run_ok("fn main() { println(as_int8(0)); } main();"), "0\n");
}

#[test]
fn as_int8_positive_one() {
    assert_eq!(run_ok("fn main() { println(as_int8(1)); } main();"), "1\n");
}

#[test]
fn as_int8_negative_one() {
    assert_eq!(
        run_ok("fn main() { println(as_int8(-1)); } main();"),
        "-1\n"
    );
}

#[test]
fn as_int8_max_boundary() {
    assert_eq!(
        run_ok("fn main() { println(as_int8(127)); } main();"),
        "127\n"
    );
}

#[test]
fn as_int8_max_plus_one_wraps() {
    assert_eq!(
        run_ok("fn main() { println(as_int8(128)); } main();"),
        "-128\n"
    );
}

#[test]
fn as_int8_min_boundary() {
    assert_eq!(
        run_ok("fn main() { println(as_int8(-128)); } main();"),
        "-128\n"
    );
}

#[test]
fn as_int8_min_minus_one_wraps() {
    assert_eq!(
        run_ok("fn main() { println(as_int8(-129)); } main();"),
        "127\n"
    );
}

#[test]
fn as_int8_truncates_300() {
    // 300 = 0x12c, lower 8 bits = 0x2c = 44
    assert_eq!(
        run_ok("fn main() { println(as_int8(300)); } main();"),
        "44\n"
    );
}

#[test]
fn as_int8_truncates_255() {
    // 255 = 0xff, as i8 = -1
    assert_eq!(
        run_ok("fn main() { println(as_int8(255)); } main();"),
        "-1\n"
    );
}

#[test]
fn as_int8_truncates_256() {
    // 256 = 0x100, lower 8 bits = 0x00 = 0
    assert_eq!(
        run_ok("fn main() { println(as_int8(256)); } main();"),
        "0\n"
    );
}

#[test]
fn as_int8_truncates_257() {
    // 257 = 0x101, lower 8 bits = 0x01 = 1
    assert_eq!(
        run_ok("fn main() { println(as_int8(257)); } main();"),
        "1\n"
    );
}

#[test]
fn as_int8_large_positive() {
    // 1000 = 0x3e8, lower 8 bits = 0xe8 = -24 as i8
    assert_eq!(
        run_ok("fn main() { println(as_int8(1000)); } main();"),
        "-24\n"
    );
}

#[test]
fn as_int8_idempotence() {
    assert_eq!(
        run_ok("fn main() { let x = 50; println(as_int8(as_int8(x))); } main();"),
        "50\n"
    );
}

#[test]
fn as_int8_double_negative() {
    // -1 truncated is -1, then -1 again is -1
    assert_eq!(
        run_ok("fn main() { println(as_int8(as_int8(-1))); } main();"),
        "-1\n"
    );
}

// ============================================================================
// as_int16 tests (signed 16-bit: -32768 to 32767)
// ============================================================================

#[test]
fn as_int16_zero() {
    assert_eq!(run_ok("fn main() { println(as_int16(0)); } main();"), "0\n");
}

#[test]
fn as_int16_positive_one() {
    assert_eq!(run_ok("fn main() { println(as_int16(1)); } main();"), "1\n");
}

#[test]
fn as_int16_negative_one() {
    assert_eq!(
        run_ok("fn main() { println(as_int16(-1)); } main();"),
        "-1\n"
    );
}

#[test]
fn as_int16_max_boundary() {
    assert_eq!(
        run_ok("fn main() { println(as_int16(32767)); } main();"),
        "32767\n"
    );
}

#[test]
fn as_int16_max_plus_one_wraps() {
    assert_eq!(
        run_ok("fn main() { println(as_int16(32768)); } main();"),
        "-32768\n"
    );
}

#[test]
fn as_int16_min_boundary() {
    assert_eq!(
        run_ok("fn main() { println(as_int16(-32768)); } main();"),
        "-32768\n"
    );
}

#[test]
fn as_int16_min_minus_one_wraps() {
    assert_eq!(
        run_ok("fn main() { println(as_int16(-32769)); } main();"),
        "32767\n"
    );
}

#[test]
fn as_int16_truncates_65535() {
    // 65535 = 0xffff, as i16 = -1
    assert_eq!(
        run_ok("fn main() { println(as_int16(65535)); } main();"),
        "-1\n"
    );
}

#[test]
fn as_int16_truncates_65536() {
    // 65536 = 0x10000, lower 16 bits = 0x0000 = 0
    assert_eq!(
        run_ok("fn main() { println(as_int16(65536)); } main();"),
        "0\n"
    );
}

#[test]
fn as_int16_idempotence() {
    assert_eq!(
        run_ok("fn main() { let x = 1000; println(as_int16(as_int16(x))); } main();"),
        "1000\n"
    );
}

// ============================================================================
// as_int32 tests (signed 32-bit: -2147483648 to 2147483647)
// ============================================================================

#[test]
fn as_int32_zero() {
    assert_eq!(run_ok("fn main() { println(as_int32(0)); } main();"), "0\n");
}

#[test]
fn as_int32_positive_one() {
    assert_eq!(run_ok("fn main() { println(as_int32(1)); } main();"), "1\n");
}

#[test]
fn as_int32_negative_one() {
    assert_eq!(
        run_ok("fn main() { println(as_int32(-1)); } main();"),
        "-1\n"
    );
}

#[test]
fn as_int32_max_boundary() {
    assert_eq!(
        run_ok("fn main() { println(as_int32(2147483647)); } main();"),
        "2147483647\n"
    );
}

#[test]
fn as_int32_max_plus_one_wraps() {
    assert_eq!(
        run_ok("fn main() { println(as_int32(2147483648)); } main();"),
        "-2147483648\n"
    );
}

#[test]
fn as_int32_min_boundary() {
    assert_eq!(
        run_ok("fn main() { println(as_int32(-2147483648)); } main();"),
        "-2147483648\n"
    );
}

#[test]
fn as_int32_min_minus_one_wraps() {
    assert_eq!(
        run_ok("fn main() { println(as_int32(-2147483649)); } main();"),
        "2147483647\n"
    );
}

#[test]
fn as_int32_truncates_4294967295() {
    // 4294967295 = 0xffffffff, as i32 = -1
    assert_eq!(
        run_ok("fn main() { println(as_int32(4294967295)); } main();"),
        "-1\n"
    );
}

#[test]
fn as_int32_truncates_4294967296() {
    // 4294967296 = 0x100000000, lower 32 bits = 0x00000000 = 0
    assert_eq!(
        run_ok("fn main() { println(as_int32(4294967296)); } main();"),
        "0\n"
    );
}

#[test]
fn as_int32_idempotence() {
    assert_eq!(
        run_ok("fn main() { let x = 100000; println(as_int32(as_int32(x))); } main();"),
        "100000\n"
    );
}

// ============================================================================
// as_int64 tests (signed 64-bit: full i64 range)
// ============================================================================

#[test]
fn as_int64_zero() {
    assert_eq!(run_ok("fn main() { println(as_int64(0)); } main();"), "0\n");
}

#[test]
fn as_int64_positive_one() {
    assert_eq!(run_ok("fn main() { println(as_int64(1)); } main();"), "1\n");
}

#[test]
fn as_int64_negative_one() {
    assert_eq!(
        run_ok("fn main() { println(as_int64(-1)); } main();"),
        "-1\n"
    );
}

#[test]
fn as_int64_large_positive() {
    assert_eq!(
        run_ok("fn main() { println(as_int64(9223372036854775807)); } main();"),
        "9223372036854775807\n"
    );
}

#[test]
fn as_int64_idempotence() {
    assert_eq!(
        run_ok("fn main() { let x = 123456789; println(as_int64(as_int64(x))); } main();"),
        "123456789\n"
    );
}

// ============================================================================
// as_uint8 tests (unsigned 8-bit: 0 to 255)
// ============================================================================

#[test]
fn as_uint8_zero() {
    assert_eq!(run_ok("fn main() { println(as_uint8(0)); } main();"), "0\n");
}

#[test]
fn as_uint8_positive_one() {
    assert_eq!(run_ok("fn main() { println(as_uint8(1)); } main();"), "1\n");
}

#[test]
fn as_uint8_max_boundary() {
    assert_eq!(
        run_ok("fn main() { println(as_uint8(255)); } main();"),
        "255\n"
    );
}

#[test]
fn as_uint8_max_plus_one_wraps() {
    assert_eq!(
        run_ok("fn main() { println(as_uint8(256)); } main();"),
        "0\n"
    );
}

#[test]
fn as_uint8_truncates_300() {
    // 300 = 0x12c, lower 8 bits = 0x2c = 44
    assert_eq!(
        run_ok("fn main() { println(as_uint8(300)); } main();"),
        "44\n"
    );
}

#[test]
fn as_uint8_negative_one() {
    // -1i64 = 0xffffffffffffffff, lower 8 bits = 0xff = 255 as u8
    assert_eq!(
        run_ok("fn main() { println(as_uint8(-1)); } main();"),
        "255\n"
    );
}

#[test]
fn as_uint8_negative_128() {
    // -128i64 has lower 8 bits = 0x80 = 128 as u8
    assert_eq!(
        run_ok("fn main() { println(as_uint8(-128)); } main();"),
        "128\n"
    );
}

#[test]
fn as_uint8_negative_127() {
    // -127i64 has lower 8 bits = 0x81 = 129 as u8
    assert_eq!(
        run_ok("fn main() { println(as_uint8(-127)); } main();"),
        "129\n"
    );
}

#[test]
fn as_uint8_idempotence() {
    assert_eq!(
        run_ok("fn main() { let x = 200; println(as_uint8(as_uint8(x))); } main();"),
        "200\n"
    );
}

// ============================================================================
// as_uint16 tests (unsigned 16-bit: 0 to 65535)
// ============================================================================

#[test]
fn as_uint16_zero() {
    assert_eq!(
        run_ok("fn main() { println(as_uint16(0)); } main();"),
        "0\n"
    );
}

#[test]
fn as_uint16_positive_one() {
    assert_eq!(
        run_ok("fn main() { println(as_uint16(1)); } main();"),
        "1\n"
    );
}

#[test]
fn as_uint16_max_boundary() {
    assert_eq!(
        run_ok("fn main() { println(as_uint16(65535)); } main();"),
        "65535\n"
    );
}

#[test]
fn as_uint16_max_plus_one_wraps() {
    assert_eq!(
        run_ok("fn main() { println(as_uint16(65536)); } main();"),
        "0\n"
    );
}

#[test]
fn as_uint16_negative_one() {
    // -1i64 has lower 16 bits = 0xffff = 65535 as u16
    assert_eq!(
        run_ok("fn main() { println(as_uint16(-1)); } main();"),
        "65535\n"
    );
}

#[test]
fn as_uint16_negative_128() {
    // -128i64 has lower 16 bits = 0xff80 = 65408 as u16
    assert_eq!(
        run_ok("fn main() { println(as_uint16(-128)); } main();"),
        "65408\n"
    );
}

#[test]
fn as_uint16_idempotence() {
    assert_eq!(
        run_ok("fn main() { let x = 50000; println(as_uint16(as_uint16(x))); } main();"),
        "50000\n"
    );
}

// ============================================================================
// as_uint32 tests (unsigned 32-bit: 0 to 4294967295)
// ============================================================================

#[test]
fn as_uint32_zero() {
    assert_eq!(
        run_ok("fn main() { println(as_uint32(0)); } main();"),
        "0\n"
    );
}

#[test]
fn as_uint32_positive_one() {
    assert_eq!(
        run_ok("fn main() { println(as_uint32(1)); } main();"),
        "1\n"
    );
}

#[test]
fn as_uint32_max_boundary() {
    assert_eq!(
        run_ok("fn main() { println(as_uint32(4294967295)); } main();"),
        "4294967295\n"
    );
}

#[test]
fn as_uint32_max_plus_one_wraps() {
    assert_eq!(
        run_ok("fn main() { println(as_uint32(4294967296)); } main();"),
        "0\n"
    );
}

#[test]
fn as_uint32_negative_one() {
    // -1i64 has lower 32 bits = 0xffffffff = 4294967295 as u32
    assert_eq!(
        run_ok("fn main() { println(as_uint32(-1)); } main();"),
        "4294967295\n"
    );
}

#[test]
fn as_uint32_negative_128() {
    // -128i64 has lower 32 bits = 0xffffff80 = 4294967168 as u32
    assert_eq!(
        run_ok("fn main() { println(as_uint32(-128)); } main();"),
        "4294967168\n"
    );
}

#[test]
fn as_uint32_idempotence() {
    assert_eq!(
        run_ok("fn main() { let x = 3000000000; println(as_uint32(as_uint32(x))); } main();"),
        "3000000000\n"
    );
}

// ============================================================================
// as_uint64 tests (unsigned 64-bit: 0 to 18446744073709551615 mod wrapping)
// ============================================================================

#[test]
fn as_uint64_zero() {
    assert_eq!(
        run_ok("fn main() { println(as_uint64(0)); } main();"),
        "0\n"
    );
}

#[test]
fn as_uint64_positive_one() {
    assert_eq!(
        run_ok("fn main() { println(as_uint64(1)); } main();"),
        "1\n"
    );
}

#[test]
fn as_uint64_large_positive() {
    // Values up to i64::MAX pass through unchanged
    assert_eq!(
        run_ok("fn main() { println(as_uint64(9223372036854775807)); } main();"),
        "9223372036854775807\n"
    );
}

#[test]
fn as_uint64_negative_one() {
    // -1i64 as u64 gives 0xffffffffffffffff, which wraps back to -1 when cast to i64
    assert_eq!(
        run_ok("fn main() { println(as_uint64(-1)); } main();"),
        "-1\n"
    );
}

#[test]
fn as_uint64_negative_128() {
    // -128i64 as u64 gives 0xffffffffffffff80, which wraps back to -128 as i64
    assert_eq!(
        run_ok("fn main() { println(as_uint64(-128)); } main();"),
        "-128\n"
    );
}

#[test]
fn as_uint64_idempotence() {
    assert_eq!(
        run_ok("fn main() { let x = 5000000000; println(as_uint64(as_uint64(x))); } main();"),
        "5000000000\n"
    );
}
