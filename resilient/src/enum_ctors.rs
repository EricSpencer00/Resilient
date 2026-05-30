//! RES-2603: Enum variant constructors as first-class function values.
//!
//! When a tuple-payload enum variant is referenced by name without being
//! immediately called, the runtime produces a `Value::EnumConstructor`
//! (bound during `EnumDecl` evaluation in `lib.rs`). Passing that value
//! to a higher-order function and calling it later produces the
//! corresponding `Value::EnumVariant`.
//!
//! # Example (Resilient surface)
//!
//! ```text
//! enum Option { Some(Int), None }
//!
//! fn apply(fn(Int) -> Option f, Int x) -> Option { return f(x); }
//!
//! let result = apply(Option::Some, 42);  // result == Option::Some(42)
//! ```
//!
//! # Design
//!
//! * **`Value::EnumConstructor`** (defined in `lib.rs`) — lightweight
//!   value carrying type name, variant name, and arity.
//! * **`apply_constructor`** — called from `apply_function` in `lib.rs`
//!   to convert a `Value::EnumConstructor` + args into a `Value::EnumVariant`.
//!
//! Named-payload variants (e.g. `Shape::Circle { r: Float }`) are not
//! supported as first-class constructors — the named-argument calling
//! convention differs from positional functions.

use crate::{EnumValuePayload, RResult, Value};

/// Convert a `Value::EnumConstructor` call into a `Value::EnumVariant`.
///
/// Called from `apply_function` in `lib.rs` when `func` is
/// `Value::EnumConstructor`. Checks arity then builds the tuple-payload
/// `EnumVariant`.
pub(crate) fn apply_constructor(
    type_name: &str,
    variant: &str,
    arity: usize,
    args: Vec<Value>,
) -> RResult<Value> {
    if args.len() != arity {
        return Err(format!(
            "Constructor {}::{}: expected {} argument(s), got {}",
            type_name,
            variant,
            arity,
            args.len()
        ));
    }
    Ok(Value::EnumVariant {
        type_name: type_name.to_string(),
        variant: variant.to_string(),
        payload: EnumValuePayload::Tuple(args),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_constructor_single_arg() {
        let result = apply_constructor("Option", "Some", 1, vec![Value::Int(42)]).unwrap();
        match result {
            Value::EnumVariant {
                type_name,
                variant,
                payload: EnumValuePayload::Tuple(items),
            } => {
                assert_eq!(type_name, "Option");
                assert_eq!(variant, "Some");
                assert_eq!(items.len(), 1);
                assert!(matches!(items[0], Value::Int(42)));
            }
            other => panic!("expected EnumVariant, got {:?}", other),
        }
    }

    #[test]
    fn apply_constructor_two_args() {
        let result =
            apply_constructor("Point", "Pair", 2, vec![Value::Int(1), Value::Int(2)]).unwrap();
        match result {
            Value::EnumVariant {
                payload: EnumValuePayload::Tuple(items),
                ..
            } => assert_eq!(items.len(), 2),
            other => panic!("expected EnumVariant, got {:?}", other),
        }
    }

    #[test]
    fn apply_constructor_too_few_args_errors() {
        let err = apply_constructor("Option", "Some", 1, vec![]).unwrap_err();
        assert!(err.contains("expected 1 argument"), "got: {err}");
    }

    #[test]
    fn apply_constructor_too_many_args_errors() {
        let err =
            apply_constructor("Option", "Some", 1, vec![Value::Int(1), Value::Int(2)]).unwrap_err();
        assert!(err.contains("expected 1 argument"), "got: {err}");
    }

    #[test]
    fn apply_constructor_zero_arity() {
        // Edge case: a tuple variant declared with no types, e.g. `Foo()`.
        let result = apply_constructor("Wrap", "Empty", 0, vec![]).unwrap();
        match result {
            Value::EnumVariant {
                payload: EnumValuePayload::Tuple(items),
                ..
            } => assert!(items.is_empty()),
            other => panic!("expected EnumVariant, got {:?}", other),
        }
    }
}
