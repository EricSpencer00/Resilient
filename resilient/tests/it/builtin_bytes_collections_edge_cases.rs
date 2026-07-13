//! ~50 integration tests for bytes, ASCII predicates, hashmap, set, result/option builtins.
//! Covers: bytes_repeat/count_byte/replace_byte, bytes_xor/and/or/not/fill/reverse,
//! bytes_take/drop/take_last/drop_last, bytes_strip_prefix/suffix/to_string,
//! is_ascii_* predicates, map_entries/merge/is_empty, set_from_array/is_empty,
//! result_and, option_and.

fn run_ok(src: &str) -> String {
    let r = resilient::run_program(src);
    assert!(r.ok, "failed: {:?}", r.errors);
    r.stdout
}

// ─── bytes_repeat ───

#[test]
fn bytes_repeat_basic() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"ab";
    let r = bytes_repeat(b, 3);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("6"));
}

#[test]
fn bytes_repeat_zero() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"abc";
    let r = bytes_repeat(b, 0);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("0"));
}

#[test]
fn bytes_repeat_one() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"789";
    let r = bytes_repeat(b, 1);
    print(bytes_eq(r, b));
}
main();
"#,
    );
    assert!(out.contains("true"));
}

#[test]
fn bytes_repeat_empty() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"";
    let r = bytes_repeat(b, 5);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("0"));
}

// ─── bytes_count_byte ───

#[test]
fn bytes_count_byte_basic() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"\x01\x02\x01\x03\x01\x04";
    print(bytes_count_byte(b, 1));
}
main();
"#,
    );
    assert!(out.contains("3"));
}

#[test]
fn bytes_count_byte_zero_bytes() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"\x00\x01\x00\x02";
    print(bytes_count_byte(b, 0));
}
main();
"#,
    );
    assert!(out.contains("2"));
}

#[test]
fn bytes_count_byte_none_match() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"abc";
    print(bytes_count_byte(b, 255));
}
main();
"#,
    );
    assert!(out.contains("0"));
}

#[test]
fn bytes_count_byte_empty() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"";
    print(bytes_count_byte(b, 0));
}
main();
"#,
    );
    assert!(out.contains("0"));
}

// ─── bytes_replace_byte ───

#[test]
fn bytes_replace_byte_basic() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"\x01\x02\x01\x03";
    let r = bytes_replace_byte(b, 1, 99);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("4"));
}

#[test]
fn bytes_replace_byte_no_match() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"abc";
    let r = bytes_replace_byte(b, 255, 0);
    print(bytes_eq(r, b));
}
main();
"#,
    );
    assert!(out.contains("true"));
}

#[test]
fn bytes_replace_byte_identity() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"abc";
    let r = bytes_replace_byte(b, 98, 98);
    print(bytes_eq(r, b));
}
main();
"#,
    );
    assert!(out.contains("true"));
}

// ─── bytes_bitwise (xor, and, or, not) ───

#[test]
fn bytes_xor_equal_length() {
    let out = run_ok(
        r#"
fn main() {
    let a = b"\xFF\x0F";
    let b = b"\xAA\xAA";
    let r = bytes_xor(a, b);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("2"));
}

#[test]
fn bytes_xor_self_zero() {
    let out = run_ok(
        r#"
fn main() {
    let a = b"abc";
    let r = bytes_xor(a, a);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("3"));
}

#[test]
fn bytes_and_mask() {
    let out = run_ok(
        r#"
fn main() {
    let a = b"\xFF\xFF";
    let b = b"\x00\x0F";
    let r = bytes_and(a, b);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("2"));
}

#[test]
fn bytes_or_set_bits() {
    let out = run_ok(
        r#"
fn main() {
    let a = b"\x10\x20";
    let b = b"\x0F\x0F";
    let r = bytes_or(a, b);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("2"));
}

#[test]
fn bytes_not_involution() {
    let out = run_ok(
        r#"
fn main() {
    let a = b"\x00\xFF\xAA";
    let once = bytes_not(a);
    let twice = bytes_not(once);
    print(bytes_eq(twice, a));
}
main();
"#,
    );
    assert!(out.contains("true"));
}

#[test]
fn bytes_not_empty() {
    let out = run_ok(
        r#"
fn main() {
    let a = b"";
    let r = bytes_not(a);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("0"));
}

// ─── bytes_fill ───

#[test]
fn bytes_fill_basic() {
    let out = run_ok(
        r#"
fn main() {
    let b = bytes_fill(4, 66);
    print(bytes_len(b));
}
main();
"#,
    );
    assert!(out.contains("4"));
}

#[test]
fn bytes_fill_zero_length() {
    let out = run_ok(
        r#"
fn main() {
    let b = bytes_fill(0, 0);
    print(bytes_len(b));
}
main();
"#,
    );
    assert!(out.contains("0"));
}

#[test]
fn bytes_fill_boundary_bytes() {
    let out = run_ok(
        r#"
fn main() {
    let b0 = bytes_fill(2, 0);
    let b255 = bytes_fill(2, 255);
    print(bytes_len(b0) + bytes_len(b255));
}
main();
"#,
    );
    assert!(out.contains("4"));
}

// ─── bytes_reverse ───

#[test]
fn bytes_reverse_basic() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"12345";
    let r = bytes_reverse(b);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("5"));
}

#[test]
fn bytes_reverse_involution() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"\xDE\xAD\xBE\xEF";
    let once = bytes_reverse(b);
    let twice = bytes_reverse(once);
    print(bytes_eq(twice, b));
}
main();
"#,
    );
    assert!(out.contains("true"));
}

#[test]
fn bytes_reverse_palindrome() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"\x01\x02\x03\x02\x01";
    let r = bytes_reverse(b);
    print(bytes_eq(r, b));
}
main();
"#,
    );
    assert!(out.contains("true"));
}

// ─── bytes_take / bytes_drop ───

#[test]
fn bytes_take_basic() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"12345";
    let r = bytes_take(b, 3);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("3"));
}

#[test]
fn bytes_take_zero() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"abc";
    let r = bytes_take(b, 0);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("0"));
}

#[test]
fn bytes_take_clamps() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"abc";
    let r = bytes_take(b, 99);
    print(bytes_eq(r, b));
}
main();
"#,
    );
    assert!(out.contains("true"));
}

#[test]
fn bytes_drop_basic() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"12345";
    let r = bytes_drop(b, 2);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("3"));
}

#[test]
fn bytes_drop_zero() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"abc";
    let r = bytes_drop(b, 0);
    print(bytes_eq(r, b));
}
main();
"#,
    );
    assert!(out.contains("true"));
}

#[test]
fn bytes_drop_all() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"abc";
    let r = bytes_drop(b, 99);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("0"));
}

#[test]
fn bytes_take_last_basic() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"12345";
    let r = bytes_take_last(b, 2);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("2"));
}

#[test]
fn bytes_drop_last_basic() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"12345";
    let r = bytes_drop_last(b, 2);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("3"));
}

// ─── bytes_strip_prefix / bytes_strip_suffix ───

#[test]
fn bytes_strip_prefix_match() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"12345";
    let prefix = b"12";
    let r = bytes_strip_prefix(b, prefix);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("3"));
}

#[test]
fn bytes_strip_prefix_no_match() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"abc";
    let prefix = b"99";
    let r = bytes_strip_prefix(b, prefix);
    print(bytes_eq(r, b));
}
main();
"#,
    );
    assert!(out.contains("true"));
}

#[test]
fn bytes_strip_suffix_match() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"12345";
    let suffix = b"45";
    let r = bytes_strip_suffix(b, suffix);
    print(bytes_len(r));
}
main();
"#,
    );
    assert!(out.contains("3"));
}

#[test]
fn bytes_strip_suffix_no_match() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"abc";
    let suffix = b"99";
    let r = bytes_strip_suffix(b, suffix);
    print(bytes_eq(r, b));
}
main();
"#,
    );
    assert!(out.contains("true"));
}

// ─── bytes_to_string ───

#[test]
fn bytes_to_string_ascii() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"hello";
    match bytes_to_string(b) {
        Ok(s) => print(s),
        Err(e) => print("error"),
    }
}
main();
"#,
    );
    assert!(out.contains("hello"));
}

#[test]
fn bytes_to_string_empty() {
    let out = run_ok(
        r#"
fn main() {
    let b = b"";
    match bytes_to_string(b) {
        Ok(s) => print("ok"),
        Err(e) => print("error"),
    }
}
main();
"#,
    );
    assert!(out.contains("ok"));
}

// ─── ASCII predicates ───

#[test]
fn is_ascii_basic() {
    let out = run_ok(
        r#"
fn main() {
    print(is_ascii("hello"));
    print(is_ascii(""));
}
main();
"#,
    );
    assert!(out.contains("true"));
}

#[test]
fn is_ascii_hexdigit() {
    let out = run_ok(
        r#"
fn main() {
    print(is_ascii_hexdigit("0123456789abcdef"));
    print(is_ascii_hexdigit("DEADBEEF"));
    print(is_ascii_hexdigit("g"));
}
main();
"#,
    );
    assert!(out.contains("true"));
    assert!(out.contains("false"));
}

#[test]
fn is_ascii_uppercase() {
    let out = run_ok(
        r#"
fn main() {
    print(is_ascii_uppercase("ABC"));
    print(is_ascii_uppercase(""));
    print(is_ascii_uppercase("Abc"));
}
main();
"#,
    );
    assert!(out.contains("true"));
    assert!(out.contains("false"));
}

#[test]
fn is_ascii_lowercase() {
    let out = run_ok(
        r#"
fn main() {
    print(is_ascii_lowercase("abc"));
    print(is_ascii_lowercase(""));
    print(is_ascii_lowercase("aBc"));
}
main();
"#,
    );
    assert!(out.contains("true"));
    assert!(out.contains("false"));
}

#[test]
fn is_ascii_whitespace() {
    let out = run_ok(
        r#"
fn main() {
    print(is_ascii_whitespace(" \t\n"));
    print(is_ascii_whitespace(""));
    print(is_ascii_whitespace("a"));
}
main();
"#,
    );
    assert!(out.contains("true"));
    assert!(out.contains("false"));
}

#[test]
fn is_ascii_punctuation() {
    let out = run_ok(
        r#"
fn main() {
    print(is_ascii_punctuation("!@#"));
    print(is_ascii_punctuation(""));
    print(is_ascii_punctuation("a"));
}
main();
"#,
    );
    assert!(out.contains("true"));
    assert!(out.contains("false"));
}

#[test]
fn is_ascii_control() {
    let out = run_ok(
        r#"
fn main() {
    print(is_ascii_control(""));
    print(is_ascii_control("a"));
}
main();
"#,
    );
    assert!(out.contains("true"));
    assert!(out.contains("false"));
}

// ─── Map builtins ───

#[test]
fn map_is_empty() {
    let out = run_ok(
        r#"
fn main() {
    let m1: Map<Int, Int> = map_new();
    print(map_is_empty(m1));
    let m2 = map_insert(m1, 1, 10);
    print(map_is_empty(m2));
}
main();
"#,
    );
    assert!(out.contains("true"));
    assert!(out.contains("false"));
}

#[test]
fn map_entries_empty() {
    let out = run_ok(
        r#"
fn main() {
    let m: Map<Int, Int> = map_new();
    let entries = map_entries(m);
    print(entries.len());
}
main();
"#,
    );
    assert!(out.contains("0"));
}

#[test]
fn map_entries_has_elements() {
    let out = run_ok(
        r#"
fn main() {
    let m: Map<Int, Int> = map_new();
    let m2 = map_insert(m, 1, 10);
    let entries = map_entries(m2);
    print(entries.len());
}
main();
"#,
    );
    assert!(out.contains("1"));
}

#[test]
fn map_merge_basic() {
    let out = run_ok(
        r#"
fn main() {
    let m1: Map<Int, Int> = map_new();
    let m1 = map_insert(m1, 1, 10);
    let m2: Map<Int, Int> = map_new();
    let m2 = map_insert(m2, 2, 20);
    let merged = map_merge(m1, m2);
    print(map_len(merged));
}
main();
"#,
    );
    assert!(out.contains("2"));
}

#[test]
fn map_merge_override() {
    let out = run_ok(
        r#"
fn main() {
    let m1: Map<Int, Int> = map_new();
    let m1 = map_insert(m1, 1, 10);
    let m2: Map<Int, Int> = map_new();
    let m2 = map_insert(m2, 1, 99);
    let merged = map_merge(m1, m2);
    print(map_len(merged));
}
main();
"#,
    );
    assert!(out.contains("1"));
}

#[test]
fn hashmap_is_empty_alias() {
    let out = run_ok(
        r#"
fn main() {
    let m: Map<Int, Int> = map_new();
    print(hashmap_is_empty(m));
}
main();
"#,
    );
    assert!(out.contains("true"));
}

#[test]
fn hashmap_entries_alias() {
    let out = run_ok(
        r#"
fn main() {
    let m: Map<Int, Int> = map_new();
    let m = map_insert(m, 1, 10);
    let entries = hashmap_entries(m);
    print(entries.len());
}
main();
"#,
    );
    assert!(out.contains("1"));
}

// ─── Set builtins ───

#[test]
fn set_from_array_basic() {
    let out = run_ok(
        r#"
fn main() {
    let arr = [1, 2, 3];
    let s = set_from_array(arr);
    print(set_len(s));
}
main();
"#,
    );
    assert!(out.contains("3"));
}

#[test]
fn set_from_array_dedup() {
    let out = run_ok(
        r#"
fn main() {
    let arr = [1, 2, 1, 2];
    let s = set_from_array(arr);
    print(set_len(s));
}
main();
"#,
    );
    assert!(out.contains("2"));
}

#[test]
fn set_from_array_empty() {
    let out = run_ok(
        r#"
fn main() {
    let arr: Array<Int> = [];
    let s = set_from_array(arr);
    print(set_len(s));
}
main();
"#,
    );
    assert!(out.contains("0"));
}

#[test]
fn set_is_empty() {
    let out = run_ok(
        r#"
fn main() {
    let s: Set<Int> = set_new();
    print(set_is_empty(s));
    let s2 = set_insert(s, 1);
    print(set_is_empty(s2));
}
main();
"#,
    );
    assert!(out.contains("true"));
    assert!(out.contains("false"));
}

// ─── Result / Option combinators ───

#[test]
fn result_and_ok_ok() {
    let out = run_ok(
        r#"
fn main() {
    let r1: Result<Int, String> = Ok(42);
    let r2: Result<Int, String> = Ok(99);
    let r = result_and(r1, r2);
    match r {
        Ok(v) => print("ok"),
        Err(_) => print("err"),
    }
}
main();
"#,
    );
    assert!(out.contains("ok"));
}

#[test]
fn result_and_err_ok() {
    let out = run_ok(
        r#"
fn main() {
    let r1: Result<Int, String> = Err("first");
    let r2: Result<Int, String> = Ok(99);
    let r = result_and(r1, r2);
    match r {
        Ok(_) => print("ok"),
        Err(_) => print("err"),
    }
}
main();
"#,
    );
    assert!(out.contains("err"));
}

#[test]
fn result_and_ok_err() {
    let out = run_ok(
        r#"
fn main() {
    let r1: Result<Int, String> = Ok(42);
    let r2: Result<Int, String> = Err("second");
    let r = result_and(r1, r2);
    match r {
        Ok(_) => print("ok"),
        Err(_) => print("err"),
    }
}
main();
"#,
    );
    assert!(out.contains("err"));
}

#[test]
fn option_and_some_some() {
    let out = run_ok(
        r#"
fn main() {
    let o1: Option<Int> = Some(42);
    let o2: Option<Int> = Some(99);
    let o = option_and(o1, o2);
    match o {
        Some(_) => print("some"),
        None => print("none"),
    }
}
main();
"#,
    );
    assert!(out.contains("some"));
}

#[test]
fn option_and_none_some() {
    let out = run_ok(
        r#"
fn main() {
    let o1: Option<Int> = None;
    let o2: Option<Int> = Some(99);
    let o = option_and(o1, o2);
    match o {
        Some(_) => print("some"),
        None => print("none"),
    }
}
main();
"#,
    );
    assert!(out.contains("none"));
}

#[test]
fn option_and_some_none() {
    let out = run_ok(
        r#"
fn main() {
    let o1: Option<Int> = Some(42);
    let o2: Option<Int> = None;
    let o = option_and(o1, o2);
    match o {
        Some(_) => print("some"),
        None => print("none"),
    }
}
main();
"#,
    );
    assert!(out.contains("none"));
}
