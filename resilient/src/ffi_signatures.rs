//! RES-4246: bind `extern fn` declarations into the typechecker scope.
//!
//! The `Node::Extern` arm of the typechecker validates each declared
//! signature against the supported-ABI allowlist, but until this module
//! existed it never bound the declared names. Every call site therefore
//! failed name resolution — visibly as a spurious `Undefined variable`
//! diagnostic on stderr, and invisibly as a total absence of call-site
//! checking: arity and argument types were only caught at runtime, in
//! the trampoline, after the dynamic symbol had already been resolved.
//!
//! Mapping the FFI surface spellings onto Resilient types is not quite
//! the identity. `CStr` is a `const char*` on the C side but a `String`
//! in Resilient source, and `OpaquePtr` is deliberately opaque — the
//! language can only pass it between extern calls, so no Resilient type
//! describes it and it erases to `Any`.

use crate::ExternDecl;
use crate::typechecker::Type;

/// Resilient type for an FFI parameter or return spelling.
fn resilient_type(ffi_name: &str) -> Type {
    if let Some(Ok(_)) = crate::ffi_arrays::parse_array_type(ffi_name) {
        return array_type(ffi_name);
    }
    match ffi_name {
        "Int" => Type::Int,
        "Int32" => Type::Int32,
        "Float" => Type::Float,
        "Bool" => Type::Bool,
        // `CStr` marshals from a Resilient `String`; the NUL-terminated
        // representation is the trampoline's business, not the caller's.
        "String" | "CStr" => Type::String,
        "Void" => Type::Void,
        // Deliberately opaque (RES-215): Resilient can receive a handle
        // from one extern call and hand it to another, but no Resilient
        // type describes what it points at. `Callback` lands here too —
        // the `Node::Extern` arm rejects it outright, so the binding
        // only has to avoid inventing a type for it.
        "OpaquePtr" | "Callback" => Type::Any,
        // Anything else is a `@repr(C)` struct name (RES-317).
        other => Type::Struct(other.to_string()),
    }
}

/// Element-tracked array type for an `Array<T>` spelling.
///
/// `parse_array_type` has already confirmed the element type is one
/// with a contiguous C layout, so the element match cannot be reached
/// with anything else; `Any` keeps the array-ness without asserting an
/// element type we did not verify.
fn array_type(ffi_name: &str) -> Type {
    let inner = ffi_name
        .strip_prefix("Array<")
        .or_else(|| ffi_name.strip_prefix("array<"))
        .and_then(|rest| rest.strip_suffix('>'))
        .map(str::trim);
    match inner {
        Some("Int" | "int") => Type::TypedArray(Box::new(Type::Int)),
        Some("Float" | "float") => Type::TypedArray(Box::new(Type::Float)),
        _ => Type::Array,
    }
}

/// Binding for one `extern fn`, as `(name, type)`.
///
/// Variadic declarations bind as `Type::Any`: the call-site check on
/// `Type::Function` is an exact-arity comparison, and a C variadic
/// accepts any number of trailing arguments by construction. `Any`
/// resolves the name — which is all this ticket promises for variadics
/// — without inventing an arity the declaration does not have.
pub(crate) fn binding(decl: &ExternDecl) -> (String, Type) {
    if decl.is_variadic {
        return (decl.resilient_name.clone(), Type::Any);
    }
    let params = decl
        .parameters
        .iter()
        .map(|(ty, _)| resilient_type(ty))
        .collect();
    (
        decl.resilient_name.clone(),
        Type::Function {
            params,
            return_type: Box::new(resilient_type(&decl.return_type)),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    fn decl(params: &[(&str, &str)], ret: &str, is_variadic: bool) -> ExternDecl {
        ExternDecl {
            resilient_name: "f".to_string(),
            c_name: "f".to_string(),
            parameters: params
                .iter()
                .map(|(t, n)| (t.to_string(), n.to_string()))
                .collect(),
            return_type: ret.to_string(),
            requires: Vec::new(),
            ensures: Vec::new(),
            trusted: false,
            is_variadic,
            span: Span::default(),
        }
    }

    #[test]
    fn scalar_params_and_return_map_to_their_resilient_types() {
        let (name, ty) = binding(&decl(&[("Int", "a"), ("Float", "b")], "Bool", false));
        assert_eq!(name, "f");
        assert_eq!(
            ty,
            Type::Function {
                params: vec![Type::Int, Type::Float],
                return_type: Box::new(Type::Bool),
            }
        );
    }

    #[test]
    fn cstr_is_a_string_at_the_call_site() {
        let (_, ty) = binding(&decl(&[("CStr", "s")], "CStr", false));
        assert_eq!(
            ty,
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::String),
            }
        );
    }

    #[test]
    fn int32_keeps_its_pinned_width() {
        let (_, ty) = binding(&decl(&[("Int32", "v")], "Int32", false));
        assert_eq!(
            ty,
            Type::Function {
                params: vec![Type::Int32],
                return_type: Box::new(Type::Int32),
            }
        );
    }

    #[test]
    fn array_params_track_their_element_type() {
        let (_, ty) = binding(&decl(
            &[("Array<Int>", "xs"), ("Array<Float>", "ys")],
            "Void",
            false,
        ));
        assert_eq!(
            ty,
            Type::Function {
                params: vec![
                    Type::TypedArray(Box::new(Type::Int)),
                    Type::TypedArray(Box::new(Type::Float)),
                ],
                return_type: Box::new(Type::Void),
            }
        );
    }

    #[test]
    fn opaque_ptr_erases_to_any() {
        let (_, ty) = binding(&decl(&[("OpaquePtr", "h")], "OpaquePtr", false));
        assert_eq!(
            ty,
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Any),
            }
        );
    }

    #[test]
    fn repr_c_struct_names_keep_their_identity() {
        let (_, ty) = binding(&decl(&[("OneInt", "s")], "OneInt", false));
        assert_eq!(
            ty,
            Type::Function {
                params: vec![Type::Struct("OneInt".to_string())],
                return_type: Box::new(Type::Struct("OneInt".to_string())),
            }
        );
    }

    #[test]
    fn variadic_decls_bind_without_an_arity() {
        let (name, ty) = binding(&decl(&[("CStr", "fmt")], "Int32", true));
        assert_eq!(name, "f");
        assert_eq!(ty, Type::Any);
    }
}
