//! RES-jit: Runtime support functions for the extended Cranelift JIT backend.
//!
//! This module provides a tagged-value representation and heap-allocated
//! types (string, struct, enum, map) that JIT-compiled code calls into
//! via `extern "C-unwind"` shims.  The tag scheme uses 4 bits in
//! positions 63-60 of an i64, with TAG_INT=0 so existing raw-integer
//! JIT code works without modification.
//!
//! All public functions follow the `res_jit_*` naming convention and
//! are registered as absolute-address symbols in the JITBuilder via
//! [`register_jit_runtime_symbols`].

#![allow(dead_code)]

use std::collections::HashMap;

// ============================================================
// Tag constants (4 bits, positions 63-60)
// ============================================================

pub(crate) const TAG_INT: i64 = 0;
pub(crate) const TAG_BOOL: i64 = 1;
pub(crate) const TAG_FLOAT: i64 = 2;
pub(crate) const TAG_STRING: i64 = 3;
pub(crate) const TAG_STRUCT: i64 = 4;
pub(crate) const TAG_ENUM: i64 = 5;
pub(crate) const TAG_CLOSURE: i64 = 6;
pub(crate) const TAG_MAP: i64 = 7;
pub(crate) const TAG_ARRAY: i64 = 8;
pub(crate) const TAG_NONE: i64 = 0xF;

const TAG_SHIFT: i64 = 60;
const TAG_MASK: i64 = 0xF;
const PAYLOAD_MASK: i64 = (1i64 << TAG_SHIFT) - 1;

// ============================================================
// Tagging / untagging helpers
// ============================================================

#[inline]
pub(crate) fn tag_of(v: i64) -> i64 {
    (v >> TAG_SHIFT) & TAG_MASK
}

#[inline]
pub(crate) fn payload_of(v: i64) -> i64 {
    v & PAYLOAD_MASK
}

#[inline]
pub(crate) fn make_tagged(tag: i64, ptr: usize) -> i64 {
    (tag << TAG_SHIFT) | (ptr as i64 & PAYLOAD_MASK)
}

// TAG_INT = 0 means raw i64 values pass through unchanged.
#[inline]
pub(crate) fn tag_int(v: i64) -> i64 {
    v
}

#[inline]
pub(crate) fn tag_bool(b: bool) -> i64 {
    make_tagged(TAG_BOOL, if b { 1 } else { 0 })
}

#[inline]
pub(crate) fn tag_float(ptr: *mut f64) -> i64 {
    make_tagged(TAG_FLOAT, ptr as usize)
}

#[inline]
pub(crate) fn tag_string(ptr: *mut String) -> i64 {
    make_tagged(TAG_STRING, ptr as usize)
}

// ============================================================
// Heap types
// ============================================================

pub(crate) struct JitStruct {
    pub name: String,
    pub fields: HashMap<String, i64>,
}

pub(crate) struct JitEnum {
    pub variant: String,
    pub payload: i64,
}

pub(crate) struct JitClosure {
    pub func_ptr: i64,
    pub env: Vec<i64>,
}

pub(crate) type JitMap = Vec<(i64, i64)>;

// ============================================================
// Runtime shims — extern "C-unwind"
// ============================================================

// --- Float ---

pub(crate) extern "C-unwind" fn res_jit_alloc_float(bits: i64) -> i64 {
    let f = f64::from_bits(bits as u64);
    let boxed = Box::new(f);
    tag_float(Box::into_raw(boxed))
}

pub(crate) extern "C-unwind" fn res_jit_float_add(a: i64, b: i64) -> i64 {
    let fa = read_float(a);
    let fb = read_float(b);
    res_jit_alloc_float((fa + fb).to_bits() as i64)
}

pub(crate) extern "C-unwind" fn res_jit_float_sub(a: i64, b: i64) -> i64 {
    let fa = read_float(a);
    let fb = read_float(b);
    res_jit_alloc_float((fa - fb).to_bits() as i64)
}

pub(crate) extern "C-unwind" fn res_jit_float_mul(a: i64, b: i64) -> i64 {
    let fa = read_float(a);
    let fb = read_float(b);
    res_jit_alloc_float((fa * fb).to_bits() as i64)
}

pub(crate) extern "C-unwind" fn res_jit_float_div(a: i64, b: i64) -> i64 {
    let fa = read_float(a);
    let fb = read_float(b);
    res_jit_alloc_float((fa / fb).to_bits() as i64)
}

pub(crate) extern "C-unwind" fn res_jit_float_rem(a: i64, b: i64) -> i64 {
    let fa = read_float(a);
    let fb = read_float(b);
    res_jit_alloc_float((fa % fb).to_bits() as i64)
}

pub(crate) extern "C-unwind" fn res_jit_float_neg(a: i64) -> i64 {
    let fa = read_float(a);
    res_jit_alloc_float((-fa).to_bits() as i64)
}

pub(crate) extern "C-unwind" fn res_jit_float_cmp(a: i64, b: i64) -> i64 {
    let fa = read_float(a);
    let fb = read_float(b);
    if fa < fb {
        -1
    } else if fa > fb {
        1
    } else {
        0
    }
}

fn read_float(v: i64) -> f64 {
    if tag_of(v) == TAG_FLOAT {
        let ptr = payload_of(v) as *const f64;
        unsafe { *ptr }
    } else {
        // Untagged int — promote to float
        v as f64
    }
}

// --- String ---

pub(crate) extern "C-unwind" fn res_jit_alloc_string(ptr: i64, len: i64) -> i64 {
    let s = if ptr == 0 || len == 0 {
        String::new()
    } else {
        let slice = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
        String::from_utf8_lossy(slice).into_owned()
    };
    let boxed = Box::new(s);
    tag_string(Box::into_raw(boxed))
}

pub(crate) extern "C-unwind" fn res_jit_string_concat(a: i64, b: i64) -> i64 {
    let sa = read_string(a);
    let sb = read_string(b);
    let result = format!("{sa}{sb}");
    let boxed = Box::new(result);
    tag_string(Box::into_raw(boxed))
}

pub(crate) extern "C-unwind" fn res_jit_string_len(v: i64) -> i64 {
    let s = read_string(v);
    s.len() as i64
}

pub(crate) extern "C-unwind" fn res_jit_value_to_string(v: i64) -> i64 {
    let s = jit_value_display(v);
    let boxed = Box::new(s);
    tag_string(Box::into_raw(boxed))
}

fn read_string(v: i64) -> String {
    if tag_of(v) == TAG_STRING {
        let ptr = payload_of(v) as *const String;
        unsafe { (*ptr).clone() }
    } else {
        jit_value_display(v)
    }
}

// --- Struct ---

pub(crate) extern "C-unwind" fn res_jit_alloc_struct(
    name_ptr: i64,
    name_len: i64,
    _field_count: i64,
) -> i64 {
    let name = read_raw_str(name_ptr, name_len);
    let s = JitStruct {
        name,
        fields: HashMap::new(),
    };
    let boxed = Box::new(s);
    let ptr = Box::into_raw(boxed);
    make_tagged(TAG_STRUCT, ptr as usize)
}

pub(crate) extern "C-unwind" fn res_jit_struct_set_field(
    struct_v: i64,
    field_ptr: i64,
    field_len: i64,
    value: i64,
) -> i64 {
    let ptr = payload_of(struct_v) as *mut JitStruct;
    let field_name = read_raw_str(field_ptr, field_len);
    unsafe {
        (*ptr).fields.insert(field_name, value);
    }
    struct_v
}

pub(crate) extern "C-unwind" fn res_jit_struct_get_field(
    struct_v: i64,
    field_ptr: i64,
    field_len: i64,
) -> i64 {
    let ptr = payload_of(struct_v) as *const JitStruct;
    let field_name = read_raw_str(field_ptr, field_len);
    unsafe { (*ptr).fields.get(&field_name).copied().unwrap_or(0) }
}

pub(crate) extern "C-unwind" fn res_jit_struct_get_name(struct_v: i64) -> i64 {
    let ptr = payload_of(struct_v) as *const JitStruct;
    let name = unsafe { (*ptr).name.clone() };
    let boxed = Box::new(name);
    tag_string(Box::into_raw(boxed))
}

// --- Enum ---

pub(crate) extern "C-unwind" fn res_jit_alloc_enum(
    variant_ptr: i64,
    variant_len: i64,
    payload: i64,
) -> i64 {
    let variant = read_raw_str(variant_ptr, variant_len);
    let e = JitEnum { variant, payload };
    let boxed = Box::new(e);
    let ptr = Box::into_raw(boxed);
    make_tagged(TAG_ENUM, ptr as usize)
}

pub(crate) extern "C-unwind" fn res_jit_enum_is_variant(
    enum_v: i64,
    variant_ptr: i64,
    variant_len: i64,
) -> i64 {
    if tag_of(enum_v) != TAG_ENUM {
        return 0;
    }
    let ptr = payload_of(enum_v) as *const JitEnum;
    let target = read_raw_str(variant_ptr, variant_len);
    let matches = unsafe { (*ptr).variant == target };
    if matches { 1 } else { 0 }
}

pub(crate) extern "C-unwind" fn res_jit_enum_payload(enum_v: i64) -> i64 {
    if tag_of(enum_v) != TAG_ENUM {
        return 0;
    }
    let ptr = payload_of(enum_v) as *const JitEnum;
    unsafe { (*ptr).payload }
}

pub(crate) extern "C-unwind" fn res_jit_enum_variant_name(enum_v: i64) -> i64 {
    if tag_of(enum_v) != TAG_ENUM {
        return res_jit_alloc_string(0, 0);
    }
    let ptr = payload_of(enum_v) as *const JitEnum;
    let name = unsafe { (*ptr).variant.clone() };
    let boxed = Box::new(name);
    tag_string(Box::into_raw(boxed))
}

// --- Map ---

pub(crate) extern "C-unwind" fn res_jit_alloc_map() -> i64 {
    let m: JitMap = Vec::new();
    let boxed = Box::new(m);
    let ptr = Box::into_raw(boxed);
    make_tagged(TAG_MAP, ptr as usize)
}

pub(crate) extern "C-unwind" fn res_jit_map_set(map_v: i64, key: i64, value: i64) -> i64 {
    let ptr = payload_of(map_v) as *mut JitMap;
    unsafe {
        // Linear scan — maps are small in typical JIT use
        if let Some(entry) = (*ptr).iter_mut().find(|(k, _)| jit_values_equal(*k, key)) {
            entry.1 = value;
        } else {
            (*ptr).push((key, value));
        }
    }
    map_v
}

pub(crate) extern "C-unwind" fn res_jit_map_get(map_v: i64, key: i64) -> i64 {
    let ptr = payload_of(map_v) as *const JitMap;
    unsafe {
        (*ptr)
            .iter()
            .find(|(k, _)| jit_values_equal(*k, key))
            .map(|(_, v)| *v)
            .unwrap_or(0)
    }
}

pub(crate) extern "C-unwind" fn res_jit_map_len(map_v: i64) -> i64 {
    let ptr = payload_of(map_v) as *const JitMap;
    unsafe { (*ptr).len() as i64 }
}

// --- Println / Print ---

pub(crate) extern "C-unwind" fn res_jit_println(v: i64) -> i64 {
    println!("{}", jit_value_display(v));
    0
}

pub(crate) extern "C-unwind" fn res_jit_print(v: i64) -> i64 {
    print!("{}", jit_value_display(v));
    0
}

// --- Equality ---

pub(crate) extern "C-unwind" fn res_jit_value_eq(a: i64, b: i64) -> i64 {
    if jit_values_equal(a, b) { 1 } else { 0 }
}

pub(crate) extern "C-unwind" fn res_jit_value_ne(a: i64, b: i64) -> i64 {
    if jit_values_equal(a, b) { 0 } else { 1 }
}

// ============================================================
// Display / equality helpers
// ============================================================

/// Convert a tagged JIT value to its display string.
///
/// TAG_INT = 0 means raw i64 values with zero in their top 4 bits display
/// as integers directly.  For negative integers (whose top 4 bits are
/// nonzero), `tag_of` may return a "fake" tag.  We guard against this by
/// checking that heap-tag payloads look like plausible aligned pointers
/// before dereferencing them.  If the pointer check fails, the value is
/// treated as a raw integer.
pub(crate) fn jit_value_display(v: i64) -> String {
    let tag = tag_of(v);

    // Fast path: tag 0 is always a raw integer.
    if tag == TAG_INT {
        return format!("{v}");
    }

    // TAG_BOOL: payload is 0 or 1 — both are small, not a pointer.
    if tag == TAG_BOOL {
        return if payload_of(v) != 0 {
            "true".to_string()
        } else {
            "false".to_string()
        };
    }

    // For heap-backed tags we need a plausible pointer in the payload.
    let ptr_val = payload_of(v) as usize;
    let is_plausible_ptr =
        ptr_val > 0x1000 && ptr_val.is_multiple_of(std::mem::align_of::<usize>());

    if !is_plausible_ptr {
        // Top bits are nonzero but the payload is not a valid pointer —
        // this is a negative raw integer.
        return format!("{v}");
    }

    match tag {
        TAG_FLOAT => {
            let ptr = ptr_val as *const f64;
            let f = unsafe { *ptr };
            format!("{f}")
        }
        TAG_STRING => {
            let ptr = ptr_val as *const String;
            unsafe { (*ptr).clone() }
        }
        TAG_STRUCT => {
            let ptr = ptr_val as *const JitStruct;
            unsafe {
                let s = &*ptr;
                let mut out = format!("{} {{ ", s.name);
                let mut first = true;
                for (k, val) in &s.fields {
                    if !first {
                        out.push_str(", ");
                    }
                    out.push_str(&format!("{k}: {}", jit_value_display(*val)));
                    first = false;
                }
                out.push_str(" }");
                out
            }
        }
        TAG_ENUM => {
            let ptr = ptr_val as *const JitEnum;
            unsafe {
                let e = &*ptr;
                if e.payload == 0 && tag_of(e.payload) == TAG_INT {
                    e.variant.clone()
                } else {
                    format!("{}({})", e.variant, jit_value_display(e.payload))
                }
            }
        }
        TAG_MAP => {
            let ptr = ptr_val as *const JitMap;
            unsafe {
                let m = &*ptr;
                let mut out = String::from("{");
                let mut first = true;
                for (k, val) in m {
                    if !first {
                        out.push_str(", ");
                    }
                    out.push_str(&format!(
                        "{} -> {}",
                        jit_value_display(*k),
                        jit_value_display(*val)
                    ));
                    first = false;
                }
                out.push('}');
                out
            }
        }
        _ => format!("{v}"),
    }
}

/// Check whether a tagged value is effectively a raw integer.
/// TAG_INT = 0 covers non-negative values; negative integers have
/// nonzero top bits that look like a tag but don't point to a valid
/// heap object.
fn is_raw_int(v: i64) -> bool {
    let tag = tag_of(v);
    if tag == TAG_INT {
        return true;
    }
    if tag == TAG_BOOL {
        return false;
    }
    // Heap tags require a plausible aligned pointer in the payload.
    let ptr_val = payload_of(v) as usize;
    !(ptr_val > 0x1000 && ptr_val.is_multiple_of(std::mem::align_of::<usize>()))
}

pub(crate) fn jit_values_equal(a: i64, b: i64) -> bool {
    let a_int = is_raw_int(a);
    let b_int = is_raw_int(b);

    if a_int && b_int {
        return a == b;
    }

    let ta = tag_of(a);
    let tb = tag_of(b);

    if ta == TAG_BOOL && tb == TAG_BOOL {
        return payload_of(a) == payload_of(b);
    }

    let a_float = !a_int && ta == TAG_FLOAT;
    let b_float = !b_int && tb == TAG_FLOAT;

    if a_float && b_float {
        return read_float(a) == read_float(b);
    }

    let a_str = !a_int && ta == TAG_STRING;
    let b_str = !b_int && tb == TAG_STRING;
    if a_str && b_str {
        let pa = payload_of(a) as *const String;
        let pb = payload_of(b) as *const String;
        return unsafe { *pa == *pb };
    }
    // Cross-type numeric: int vs float
    if a_int && b_float {
        return (a as f64) == read_float(b);
    }
    if a_float && b_int {
        return read_float(a) == (b as f64);
    }
    // Pointer equality for heap types
    a == b
}

// ============================================================
// Helpers
// ============================================================

fn read_raw_str(ptr: i64, len: i64) -> String {
    if ptr == 0 || len == 0 {
        return String::new();
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    String::from_utf8_lossy(slice).into_owned()
}

// ============================================================
// Symbol registration
// ============================================================

/// Register all JIT runtime shim symbols on the given JITBuilder
/// so Cranelift-compiled code can call them by name.
pub(crate) fn register_jit_runtime_symbols(builder: &mut cranelift_jit::JITBuilder) {
    macro_rules! reg {
        ($name:ident) => {
            builder.symbol(stringify!($name), $name as *const u8);
        };
    }
    reg!(res_jit_alloc_float);
    reg!(res_jit_float_add);
    reg!(res_jit_float_sub);
    reg!(res_jit_float_mul);
    reg!(res_jit_float_div);
    reg!(res_jit_float_rem);
    reg!(res_jit_float_neg);
    reg!(res_jit_float_cmp);
    reg!(res_jit_alloc_string);
    reg!(res_jit_string_concat);
    reg!(res_jit_string_len);
    reg!(res_jit_value_to_string);
    reg!(res_jit_alloc_struct);
    reg!(res_jit_struct_set_field);
    reg!(res_jit_struct_get_field);
    reg!(res_jit_struct_get_name);
    reg!(res_jit_alloc_enum);
    reg!(res_jit_enum_is_variant);
    reg!(res_jit_enum_payload);
    reg!(res_jit_enum_variant_name);
    reg!(res_jit_alloc_map);
    reg!(res_jit_map_set);
    reg!(res_jit_map_get);
    reg!(res_jit_map_len);
    reg!(res_jit_println);
    reg!(res_jit_print);
    reg!(res_jit_value_eq);
    reg!(res_jit_value_ne);
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_int_is_identity() {
        assert_eq!(tag_int(42), 42);
        assert_eq!(tag_of(42), TAG_INT);
    }

    #[test]
    fn tag_bool_roundtrip() {
        let t = tag_bool(true);
        assert_eq!(tag_of(t), TAG_BOOL);
        assert_eq!(payload_of(t), 1);
        let f = tag_bool(false);
        assert_eq!(tag_of(f), TAG_BOOL);
        assert_eq!(payload_of(f), 0);
    }

    #[test]
    fn float_alloc_and_read() {
        let v = res_jit_alloc_float(1.234f64.to_bits() as i64);
        assert_eq!(tag_of(v), TAG_FLOAT);
        let f = read_float(v);
        assert!((f - 1.234).abs() < 1e-10);
    }

    #[test]
    fn string_alloc_and_display() {
        let hello = "hello";
        let v = res_jit_alloc_string(hello.as_ptr() as i64, hello.len() as i64);
        assert_eq!(tag_of(v), TAG_STRING);
        assert_eq!(jit_value_display(v), "hello");
    }

    #[test]
    fn string_concat() {
        let a = res_jit_alloc_string("foo".as_ptr() as i64, 3);
        let b = res_jit_alloc_string("bar".as_ptr() as i64, 3);
        let c = res_jit_string_concat(a, b);
        assert_eq!(jit_value_display(c), "foobar");
    }

    #[test]
    fn struct_alloc_and_field_access() {
        let name = "Point";
        let s = res_jit_alloc_struct(name.as_ptr() as i64, name.len() as i64, 2);
        assert_eq!(tag_of(s), TAG_STRUCT);
        let field = "x";
        res_jit_struct_set_field(s, field.as_ptr() as i64, field.len() as i64, 42);
        let val = res_jit_struct_get_field(s, field.as_ptr() as i64, field.len() as i64);
        assert_eq!(val, 42);
    }

    #[test]
    fn enum_alloc_and_variant_check() {
        let variant = "Some";
        let e = res_jit_alloc_enum(variant.as_ptr() as i64, variant.len() as i64, 99);
        assert_eq!(tag_of(e), TAG_ENUM);
        assert_eq!(
            res_jit_enum_is_variant(e, variant.as_ptr() as i64, variant.len() as i64),
            1
        );
        let other = "None";
        assert_eq!(
            res_jit_enum_is_variant(e, other.as_ptr() as i64, other.len() as i64),
            0
        );
        assert_eq!(res_jit_enum_payload(e), 99);
    }

    #[test]
    fn map_alloc_set_get() {
        let m = res_jit_alloc_map();
        assert_eq!(tag_of(m), TAG_MAP);
        res_jit_map_set(m, 1, 100);
        res_jit_map_set(m, 2, 200);
        assert_eq!(res_jit_map_get(m, 1), 100);
        assert_eq!(res_jit_map_get(m, 2), 200);
        assert_eq!(res_jit_map_len(m), 2);
    }

    #[test]
    fn value_equality_int() {
        assert_eq!(res_jit_value_eq(42, 42), 1);
        assert_eq!(res_jit_value_eq(42, 43), 0);
        assert_eq!(res_jit_value_ne(42, 43), 1);
    }

    #[test]
    fn value_equality_string() {
        let a = res_jit_alloc_string("same".as_ptr() as i64, 4);
        let b = res_jit_alloc_string("same".as_ptr() as i64, 4);
        assert_eq!(res_jit_value_eq(a, b), 1);
    }

    #[test]
    fn display_int() {
        assert_eq!(jit_value_display(42), "42");
        assert_eq!(jit_value_display(-1), "-1");
    }

    #[test]
    fn display_bool() {
        assert_eq!(jit_value_display(tag_bool(true)), "true");
        assert_eq!(jit_value_display(tag_bool(false)), "false");
    }

    #[test]
    fn value_to_string_shim() {
        let v = res_jit_value_to_string(42);
        assert_eq!(tag_of(v), TAG_STRING);
        assert_eq!(jit_value_display(v), "42");
    }

    #[test]
    fn float_arithmetic() {
        let a = res_jit_alloc_float(2.0f64.to_bits() as i64);
        let b = res_jit_alloc_float(3.0f64.to_bits() as i64);
        let sum = res_jit_float_add(a, b);
        assert!((read_float(sum) - 5.0).abs() < 1e-10);
        let diff = res_jit_float_sub(a, b);
        assert!((read_float(diff) - (-1.0)).abs() < 1e-10);
        let prod = res_jit_float_mul(a, b);
        assert!((read_float(prod) - 6.0).abs() < 1e-10);
        let quot = res_jit_float_div(a, b);
        assert!((read_float(quot) - (2.0 / 3.0)).abs() < 1e-10);
    }

    #[test]
    fn empty_string_alloc() {
        let v = res_jit_alloc_string(0, 0);
        assert_eq!(tag_of(v), TAG_STRING);
        assert_eq!(jit_value_display(v), "");
    }
}
