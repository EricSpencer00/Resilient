//! Operator overloading for structs via trait method dispatch.
//!
//! When an arithmetic, comparison, or bitwise operator is applied to a
//! `Value::Struct` and the host environment contains a method with the
//! mangled name `<StructName>$<op_method>`, the operator dispatches to
//! that method as if the user had written `lhs.op_method(rhs)`. The
//! method is expected to receive `self` as the first parameter and the
//! right-hand operand as the second.
//!
//! The mapping from operator → method name follows Rust's `core::ops`
//! convention so a Resilient `impl Add for Vec2 { fn add(self, other) }`
//! plays nicely with `+`. No new tokens, no parser changes — this is a
//! pure runtime dispatch enrichment that fires before the legacy
//! `Type mismatch` error.
//!
//! ## Example
//!
//! ```text
//! struct Vec2 { float x, float y, }
//! impl Add for Vec2 {
//!     fn add(Vec2 self, Vec2 other) -> Vec2 {
//!         return new Vec2 { x: self.x + other.x, y: self.y + other.y };
//!     }
//! }
//! let c = a + b;  // dispatches to Vec2$add(a, b)
//! ```

use crate::{Interpreter, RResult, Value};

/// Map an infix operator string to the conventional trait-method name.
/// Returns `None` for operators that don't have an overloadable method
/// (logical `&&` / `||`, `??`, etc.).
pub(crate) fn op_method_name(op: &str) -> Option<&'static str> {
    Some(match op {
        "+" => "add",
        "-" => "sub",
        "*" => "mul",
        "/" => "div",
        "%" => "rem",
        "==" => "eq",
        "!=" => "ne",
        "<" => "lt",
        "<=" => "le",
        ">" => "gt",
        ">=" => "ge",
        "&" => "bitand",
        "|" => "bitor",
        "^" => "bitxor",
        "<<" => "shl",
        ">>" => "shr",
        _ => return None,
    })
}

/// Try to dispatch `left <op> right` to a struct method.
///
/// Returns `Ok(Some(value))` if dispatch succeeded, `Ok(None)` if no
/// matching method was found (caller should fall through to the
/// built-in type dispatch / type-mismatch error), and `Err` if dispatch
/// fired but the method itself raised a runtime error.
pub(crate) fn try_dispatch(
    interp: &mut Interpreter,
    operator: &str,
    left: &Value,
    right: &Value,
) -> RResult<Option<Value>> {
    let Some(method) = op_method_name(operator) else {
        return Ok(None);
    };

    let struct_name = match left {
        Value::Struct { name, .. } => name.clone(),
        _ => match right {
            Value::Struct { name, .. } => name.clone(),
            _ => return Ok(None),
        },
    };

    let mangled = format!("{}${}", struct_name, method);
    let Some(method_val) = interp.env.get(&mangled) else {
        return Ok(None);
    };

    let args = vec![left.clone(), right.clone()];
    interp.apply_function(&method_val, args).map(Some)
}

#[cfg(test)]
mod tests {
    use super::op_method_name;

    #[test]
    fn op_method_name_covers_arithmetic() {
        assert_eq!(op_method_name("+"), Some("add"));
        assert_eq!(op_method_name("-"), Some("sub"));
        assert_eq!(op_method_name("*"), Some("mul"));
        assert_eq!(op_method_name("/"), Some("div"));
        assert_eq!(op_method_name("%"), Some("rem"));
    }

    #[test]
    fn op_method_name_covers_comparisons() {
        assert_eq!(op_method_name("=="), Some("eq"));
        assert_eq!(op_method_name("!="), Some("ne"));
        assert_eq!(op_method_name("<"), Some("lt"));
        assert_eq!(op_method_name(">="), Some("ge"));
    }

    #[test]
    fn op_method_name_covers_bitwise() {
        assert_eq!(op_method_name("&"), Some("bitand"));
        assert_eq!(op_method_name("|"), Some("bitor"));
        assert_eq!(op_method_name("^"), Some("bitxor"));
        assert_eq!(op_method_name("<<"), Some("shl"));
        assert_eq!(op_method_name(">>"), Some("shr"));
    }

    #[test]
    fn op_method_name_returns_none_for_logical() {
        assert_eq!(op_method_name("&&"), None);
        assert_eq!(op_method_name("||"), None);
        assert_eq!(op_method_name("??"), None);
    }
}
