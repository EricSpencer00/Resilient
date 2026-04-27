//! FFI v1 loader. Resolves extern symbols declared by `Node::Extern`
//! blocks ahead of evaluation so the tree-walker can dispatch in O(1).
//!
//! Two backends share one API:
//! - `std` / `cfg(feature = "ffi")`: dynamic loading via `libloading`.
//! - `no_std` / `resilient-runtime` with `ffi-static`: a static
//!   registry populated by the embedder. Lives in `resilient-runtime`
//!   and is not referenced here — this module is host-only.

// Phase 1 skeleton: public types/functions here are wired up in later
// tasks (tree-walker dispatch, trampoline layer). Suppress dead-code and
// unused-import lints so the build stays warning-clean as a stub.
#![allow(dead_code)]

use crate::ExternDecl;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FfiType {
    Int,
    Float,
    Bool,
    Str,
    Void,
    /// RES-215: an opaque C pointer (`*mut c_void`). Resilient code
    /// cannot dereference or inspect it; the language only passes it
    /// through — received from one extern call, handed back to
    /// another. Lets bindings model "handle" types like `FILE*`,
    /// `sqlite3*`, etc. without teaching the language about their
    /// layout.
    OpaquePtr,
    /// RES-216: a Resilient function reference passed to C as a
    /// function pointer. Phase 1 parses and recognises this type but
    /// calling an extern fn with a `Callback` argument returns
    /// `FfiError::CallbackNotYetSupported`. Full trampoline support is
    /// planned for Phase 2 (bytecode VM).
    Callback,
    /// RES-317: a Resilient struct marked `@repr(C)`. Bridged across
    /// the FFI boundary by packing/unpacking each field through a
    /// stack-allocated buffer that matches the C struct's layout.
    /// `name` is the struct's declared name (used for diagnostics and
    /// to look up layouts at marshalling time); `fields` is the
    /// resolved per-field FFI type in declaration order.
    ///
    /// Phase 1 supports structs whose total layout fits in 8 bytes
    /// (one System V integer-class register) — typical sensor /
    /// timestamp pairs of two i32s, an i64 + bool tail, etc. Larger
    /// structs return `FfiError::StructTooLarge`; that subset is
    /// scoped to a follow-up.
    Struct {
        name: String,
        fields: Vec<(String, FfiType)>,
    },
}

impl FfiType {
    /// Resolve a Resilient surface type name to an FFI type **without**
    /// any struct registry — i.e. only the built-in scalar types.
    /// Returns `None` for an unknown name (including struct names).
    pub fn from_resilient(name: &str) -> Option<Self> {
        match name {
            "Int" => Some(FfiType::Int),
            "Float" => Some(FfiType::Float),
            "Bool" => Some(FfiType::Bool),
            "String" => Some(FfiType::Str),
            "Void" => Some(FfiType::Void),
            "OpaquePtr" => Some(FfiType::OpaquePtr),
            "Callback" => Some(FfiType::Callback),
            _ => None,
        }
    }

    /// RES-317: resolve a Resilient surface type name to an FFI type,
    /// consulting `structs` for any name that isn't a built-in scalar.
    /// Returns `None` if the name is neither a built-in nor a known
    /// `@repr(C)` struct.
    pub fn from_resilient_with_structs(
        name: &str,
        structs: &HashMap<String, Vec<(String, FfiType)>>,
    ) -> Option<Self> {
        if let Some(t) = Self::from_resilient(name) {
            return Some(t);
        }
        structs.get(name).map(|fields| FfiType::Struct {
            name: name.to_string(),
            fields: fields.clone(),
        })
    }

    /// Total byte size for marshalling. For aggregates (`Struct`) this is
    /// the natural-alignment-padded total; for scalars it's the C ABI
    /// width on 64-bit SystemV / Windows x64 / AArch64 (every Resilient
    /// FFI target).
    pub fn size_bytes(&self) -> usize {
        match self {
            FfiType::Int => 8,
            FfiType::Float => 8,
            FfiType::Bool => 1,
            // Scalars below are not used inside structs in Phase 1; sizes
            // are listed for completeness so the function never panics.
            FfiType::Str => core::mem::size_of::<usize>() * 2,
            FfiType::Void => 0,
            FfiType::OpaquePtr => core::mem::size_of::<usize>(),
            FfiType::Callback => core::mem::size_of::<usize>(),
            FfiType::Struct { fields, .. } => struct_layout(fields).total,
        }
    }

    /// Natural alignment in bytes. See `size_bytes` for context.
    pub fn align_bytes(&self) -> usize {
        match self {
            FfiType::Int | FfiType::Float => 8,
            FfiType::Bool => 1,
            FfiType::Str => core::mem::align_of::<usize>(),
            FfiType::Void => 1,
            FfiType::OpaquePtr | FfiType::Callback => core::mem::align_of::<usize>(),
            FfiType::Struct { fields, .. } => struct_layout(fields).align,
        }
    }
}

/// RES-317: per-field offsets and the struct's total size + alignment.
/// `repr(C)`-style layout: fields placed in declaration order; each
/// field's offset is rounded up to its alignment; the struct's total
/// is rounded up to its largest field's alignment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructLayout {
    /// Per-field byte offsets in declaration order. Same length as the
    /// matching `Struct.fields`.
    pub offsets: Vec<usize>,
    /// Total size of the struct in bytes, rounded to `align`.
    pub total: usize,
    /// Struct alignment (max of every field's alignment, with 1 floor).
    pub align: usize,
}

/// RES-317: compute the C-ABI layout for a flat field list. Pure
/// function — used both at marshalling time and by tests.
pub fn struct_layout(fields: &[(String, FfiType)]) -> StructLayout {
    let mut offsets = Vec::with_capacity(fields.len());
    let mut offset: usize = 0;
    let mut max_align: usize = 1;
    for (_, ty) in fields {
        let align = ty.align_bytes().max(1);
        let size = ty.size_bytes();
        let misalign = offset % align;
        if misalign != 0 {
            offset += align - misalign;
        }
        offsets.push(offset);
        offset += size;
        if align > max_align {
            max_align = align;
        }
    }
    let tail_misalign = offset % max_align;
    if tail_misalign != 0 {
        offset += max_align - tail_misalign;
    }
    StructLayout {
        offsets,
        total: offset,
        align: max_align,
    }
}

/// RES-215: opaque-pointer handle carried in `Value::OpaquePtr`.
///
/// Wraps a `*mut c_void` so the language has a typed container for
/// the raw address. The pointer is never dereferenced by Resilient
/// code — trampolines accept it as an input argument and return it
/// as an output value, and that's the entire surface area. We
/// derive `Copy`/`Clone` because the pointer itself is trivially
/// copyable and the Resilient VM's `Value` is `Clone`.
///
/// SAFETY: the raw pointer is opaque. Holding an `OpaquePtrHandle`
/// confers no permission to read/write the pointee; only the C
/// library that produced it may dereference it. Lifetime is the
/// responsibility of the foreign code — Resilient will never free
/// or validate it.
#[derive(Copy, Clone, Debug)]
pub struct OpaquePtrHandle(pub *mut core::ffi::c_void);

// SAFETY: the pointer is opaque to Resilient and never dereferenced
// by interpreter code. Crossing thread boundaries does not invoke
// any load/store on the pointee; only passing the address along to
// a subsequent FFI call (which is the C library's responsibility).
unsafe impl Send for OpaquePtrHandle {}
unsafe impl Sync for OpaquePtrHandle {}

#[derive(Clone, Debug)]
pub struct ForeignSignature {
    pub params: Vec<FfiType>,
    pub ret: FfiType,
}

impl ForeignSignature {
    /// Resolve an `ExternDecl` to a signature using only the built-in
    /// FFI scalar types. Struct parameters / returns are rejected here
    /// — callers that need struct bridging should use
    /// `from_decl_with_structs` instead.
    pub fn from_decl(decl: &ExternDecl) -> Result<Self, FfiError> {
        Self::from_decl_with_structs(decl, &HashMap::new())
    }

    /// RES-317: resolve an `ExternDecl` to a signature, consulting the
    /// supplied `structs` registry for any non-scalar type names. The
    /// registry is keyed by struct name and holds the resolved field
    /// list (in declaration order).
    pub fn from_decl_with_structs(
        decl: &ExternDecl,
        structs: &HashMap<String, Vec<(String, FfiType)>>,
    ) -> Result<Self, FfiError> {
        let mut params = Vec::with_capacity(decl.parameters.len());
        for (ty, _) in &decl.parameters {
            params.push(
                FfiType::from_resilient_with_structs(ty, structs)
                    .ok_or_else(|| FfiError::UnsupportedType(ty.clone()))?,
            );
        }
        let ret = FfiType::from_resilient_with_structs(&decl.return_type, structs)
            .ok_or_else(|| FfiError::UnsupportedType(decl.return_type.clone()))?;
        if params.len() > 8 {
            return Err(FfiError::ArityTooLarge {
                name: decl.resilient_name.clone(),
                got: params.len(),
            });
        }
        Ok(Self { params, ret })
    }
}

#[derive(Debug)]
pub enum FfiError {
    LibNotFound {
        library: String,
        underlying: String,
    },
    SymbolNotFound {
        library: String,
        symbol: String,
    },
    UnsupportedType(String),
    ArityTooLarge {
        name: String,
        got: usize,
    },
    /// `--no-default-features` build asked to load a dynamic library.
    FfiDisabled,
    /// `@static` descriptor used on an `std` host without a registered
    /// backend. (v1 treats this as an error; a future ticket may let
    /// the std build register static fns too.)
    StaticOnlyUnavailable {
        library: String,
    },
    /// RES-216: the extern signature declared a `Callback` parameter
    /// and a call attempted to use it. Phase 1 recognises the type
    /// but cannot yet build a stable C function pointer from a
    /// Resilient closure; Phase 2 (bytecode VM) adds real trampolines.
    CallbackNotYetSupported {
        name: String,
    },
    /// RES-317: an `extern fn` referenced a struct type that was not
    /// declared with `@repr(C)`. The Resilient layout makes no ABI
    /// guarantees, so refusing the call is the only sound option.
    StructNotReprC {
        name: String,
    },
    /// RES-317: an `extern fn` referenced a struct whose total layout
    /// exceeds the size that the Phase 1 trampoline can pass / return
    /// by value. Larger structs require an out-pointer convention or
    /// libffi; tracked as a follow-up.
    StructTooLarge {
        name: String,
        size: usize,
        max: usize,
    },
}

impl std::fmt::Display for FfiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FfiError::LibNotFound {
                library,
                underlying,
            } => {
                write!(f, "FFI: cannot open library `{}`: {}", library, underlying)
            }
            FfiError::SymbolNotFound { library, symbol } => {
                write!(f, "FFI: symbol `{}` not found in `{}`", symbol, library)
            }
            FfiError::UnsupportedType(ty) => {
                write!(f, "FFI: type `{}` is not supported in v1", ty)
            }
            FfiError::ArityTooLarge { name, got } => {
                write!(
                    f,
                    "FFI: extern fn `{}` has {} params; v1 supports up to 8",
                    name, got
                )
            }
            FfiError::FfiDisabled => {
                write!(f, "FFI: this build was compiled without --features ffi")
            }
            FfiError::StaticOnlyUnavailable { library } => {
                write!(
                    f,
                    "FFI: library descriptor `{}` requires a static registry, not available in this build",
                    library
                )
            }
            FfiError::CallbackNotYetSupported { name } => {
                write!(
                    f,
                    "FFI: extern fn `{}` uses a Callback parameter; callbacks require the trampoline feature (planned for Phase 2)",
                    name
                )
            }
            FfiError::StructNotReprC { name } => {
                write!(
                    f,
                    "FFI: struct `{}` is referenced from an `extern fn` but not declared `@repr(C)`; the layout has no ABI guarantee",
                    name
                )
            }
            FfiError::StructTooLarge { name, size, max } => {
                write!(
                    f,
                    "FFI: struct `{}` is {} bytes; Phase 1 supports structs up to {} bytes by value (pass/return larger structs via out-pointer is a follow-up)",
                    name, size, max
                )
            }
        }
    }
}

impl std::error::Error for FfiError {}

/// A resolved extern symbol. The raw `*const ()` is cast to a concrete
/// `extern "C" fn(...)` type at call time via `ffi_trampolines`.
pub struct ForeignSymbol {
    pub name: String,
    pub ptr: *const (),
    pub sig: ForeignSignature,
}

impl std::fmt::Debug for ForeignSymbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForeignSymbol")
            .field("name", &self.name)
            .field("ptr", &format_args!("{:p}", self.ptr))
            .field("sig", &self.sig)
            .finish()
    }
}

// SAFETY: ForeignSymbol holds a raw C function pointer that outlives
// the Library it came from (we also hold the Library in the loader
// so it never drops while symbols are in use). The pointer itself
// is Send + Sync on every supported platform.
unsafe impl Send for ForeignSymbol {}
unsafe impl Sync for ForeignSymbol {}

#[cfg(feature = "ffi")]
#[allow(unused_imports)]
pub use dynamic::ForeignLoader;

#[cfg(not(feature = "ffi"))]
#[allow(unused_imports)]
pub use disabled::ForeignLoader;

#[cfg(feature = "ffi")]
mod dynamic {
    use super::*;

    pub struct ForeignLoader {
        libs: HashMap<String, libloading::Library>,
        syms: HashMap<String, std::sync::Arc<ForeignSymbol>>,
        /// RES-317: registry of `@repr(C)` structs known at resolve time.
        /// Populated once before `resolve_block` runs so extern signatures
        /// that reference struct names can resolve them.
        structs: HashMap<String, Vec<(String, FfiType)>>,
    }

    impl ForeignLoader {
        pub fn new() -> Self {
            Self {
                libs: HashMap::new(),
                syms: HashMap::new(),
                structs: HashMap::new(),
            }
        }

        /// RES-317: register a `@repr(C)` struct so subsequent
        /// `resolve_block` calls can reference it from extern signatures.
        /// Idempotent — re-registering the same name overwrites the
        /// previous entry.
        pub fn register_repr_c_struct(&mut self, name: String, fields: Vec<(String, FfiType)>) {
            self.structs.insert(name, fields);
        }

        pub fn resolve_block(
            &mut self,
            library: &str,
            decls: &[ExternDecl],
        ) -> Result<(), FfiError> {
            if library == "@static" {
                return Err(FfiError::StaticOnlyUnavailable {
                    library: library.to_string(),
                });
            }
            let lib = match self.libs.entry(library.to_string()) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    // SAFETY: Loading a dynamic library by path. The library must
                    // remain loaded for the lifetime of any symbols we extract from
                    // it; we enforce this by keeping the Library in `self.libs` for
                    // the lifetime of the ForeignLoader.
                    let lib = unsafe { libloading::Library::new(library) }.map_err(|err| {
                        FfiError::LibNotFound {
                            library: library.to_string(),
                            underlying: err.to_string(),
                        }
                    })?;
                    e.insert(lib)
                }
            };
            for d in decls {
                let sig = ForeignSignature::from_decl_with_structs(d, &self.structs)?;
                // SAFETY: We look up the symbol by its C name as a byte string.
                // The returned Symbol borrows from `lib`; we immediately copy
                // the raw pointer out so the Symbol borrow is released before
                // we return. The `lib` itself stays alive in `self.libs` so the
                // pointed-to code is never unmapped while the ForeignLoader lives.
                let raw: libloading::Symbol<*const ()> = unsafe { lib.get(d.c_name.as_bytes()) }
                    .map_err(|_| FfiError::SymbolNotFound {
                        library: library.to_string(),
                        symbol: d.c_name.clone(),
                    })?;
                let sym = ForeignSymbol {
                    name: d.resilient_name.clone(),
                    ptr: *raw,
                    sig,
                };
                self.syms
                    .insert(d.resilient_name.clone(), std::sync::Arc::new(sym));
            }
            Ok(())
        }

        pub fn lookup(&self, name: &str) -> Option<std::sync::Arc<ForeignSymbol>> {
            self.syms.get(name).cloned()
        }
    }

    impl Default for ForeignLoader {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(not(feature = "ffi"))]
mod disabled {
    use super::*;

    pub struct ForeignLoader;

    impl ForeignLoader {
        pub fn new() -> Self {
            Self
        }

        /// RES-317: matches the dynamic-loader API so callers compile
        /// against either backend. Disabled-loader keeps no state.
        pub fn register_repr_c_struct(&mut self, _name: String, _fields: Vec<(String, FfiType)>) {}

        pub fn resolve_block(
            &mut self,
            _library: &str,
            _decls: &[ExternDecl],
        ) -> Result<(), FfiError> {
            Err(FfiError::FfiDisabled)
        }

        pub fn lookup(&self, _name: &str) -> Option<std::sync::Arc<ForeignSymbol>> {
            None
        }
    }

    impl Default for ForeignLoader {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(test)]
#[cfg(feature = "ffi")]
mod tests {
    use super::*;
    use crate::{ExternDecl, span::Span};

    fn decl(name: &str, c: &str, params: Vec<(&str, &str)>, ret: &str) -> ExternDecl {
        ExternDecl {
            resilient_name: name.to_string(),
            c_name: c.to_string(),
            parameters: params
                .into_iter()
                .map(|(t, n)| (t.to_string(), n.to_string()))
                .collect(),
            return_type: ret.to_string(),
            requires: Vec::new(),
            ensures: Vec::new(),
            trusted: false,
            is_variadic: false,
            span: Span::default(),
        }
    }

    #[test]
    fn missing_library_is_a_clean_error_not_a_panic() {
        let mut loader = ForeignLoader::new();
        let err = loader
            .resolve_block("libnope_not_a_real_library.so", &[])
            .expect_err("should fail");
        assert!(matches!(err, FfiError::LibNotFound { .. }), "got {:?}", err);
    }

    #[test]
    fn signature_rejects_unsupported_types() {
        let d = decl("f", "f", vec![("Array", "xs")], "Int");
        let err = ForeignSignature::from_decl(&d).expect_err("must reject Array");
        assert!(
            matches!(err, FfiError::UnsupportedType(ref s) if s == "Array"),
            "got {:?}",
            err
        );
    }

    #[test]
    fn signature_rejects_more_than_eight_params() {
        let params: Vec<(&str, &str)> = (0..9)
            .map(|i| {
                (
                    "Int",
                    match i {
                        0 => "a",
                        1 => "b",
                        2 => "c",
                        3 => "d",
                        4 => "e",
                        5 => "f",
                        6 => "g",
                        7 => "h",
                        _ => "i",
                    },
                )
            })
            .collect();
        let d = decl("big", "big", params, "Int");
        let err = ForeignSignature::from_decl(&d).expect_err("must reject 9 params");
        assert!(
            matches!(err, FfiError::ArityTooLarge { ref name, got: 9 } if name == "big"),
            "got {:?}",
            err
        );
    }

    #[test]
    fn signature_accepts_opaque_ptr() {
        // RES-215: OpaquePtr is a real FFI type in signatures.
        let d = decl("alloc_point", "alloc_point", vec![], "OpaquePtr");
        let sig = ForeignSignature::from_decl(&d).expect("OpaquePtr must parse");
        assert_eq!(sig.ret, FfiType::OpaquePtr);

        let d = decl("free_point", "free_point", vec![("OpaquePtr", "p")], "Void");
        let sig = ForeignSignature::from_decl(&d).expect("OpaquePtr arg must parse");
        assert_eq!(sig.params, vec![FfiType::OpaquePtr]);
        assert_eq!(sig.ret, FfiType::Void);
    }

    #[test]
    fn resolve_block_rejects_static_library_descriptor() {
        let mut loader = ForeignLoader::new();
        let err = loader
            .resolve_block("@static", &[])
            .expect_err("@static should error on std host");
        assert!(
            matches!(err, FfiError::StaticOnlyUnavailable { ref library } if library == "@static"),
            "got {:?}",
            err
        );
    }

    #[test]
    fn ffi_type_recognises_callback() {
        // RES-216: the Callback type is recognised at declaration time.
        assert_eq!(FfiType::from_resilient("Callback"), Some(FfiType::Callback));
        let d = decl("register", "register", vec![("Callback", "cb")], "Void");
        let sig = ForeignSignature::from_decl(&d).expect("Callback must parse");
        assert_eq!(sig.params, vec![FfiType::Callback]);
        assert_eq!(sig.ret, FfiType::Void);
    }

    #[test]
    fn callback_error_display_mentions_phase_2() {
        let err = FfiError::CallbackNotYetSupported {
            name: "register_handler".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("register_handler"), "msg = {}", msg);
        assert!(msg.contains("Phase 2"), "msg = {}", msg);
    }

    // ============================================================
    // RES-317: layout & registry tests for `@repr(C)` struct bridging.
    // ============================================================

    #[test]
    fn struct_layout_packs_int_field_at_offset_zero() {
        // OneInt { Int v } → 8 bytes, align 8, single field at offset 0.
        let fields = vec![("v".to_string(), FfiType::Int)];
        let layout = struct_layout(&fields);
        assert_eq!(layout.total, 8);
        assert_eq!(layout.align, 8);
        assert_eq!(layout.offsets, vec![0]);
    }

    #[test]
    fn struct_layout_inserts_padding_for_natural_alignment() {
        // { Bool b, Int v } → b at 0 (1B), then 7B pad, then v at 8.
        // Total 16, align 8.
        let fields = vec![
            ("b".to_string(), FfiType::Bool),
            ("v".to_string(), FfiType::Int),
        ];
        let layout = struct_layout(&fields);
        assert_eq!(layout.offsets, vec![0, 8]);
        assert_eq!(layout.total, 16);
        assert_eq!(layout.align, 8);
    }

    #[test]
    fn struct_layout_empty_struct_is_size_zero_align_one() {
        let layout = struct_layout(&[]);
        assert_eq!(layout.total, 0);
        assert_eq!(layout.align, 1);
        assert_eq!(layout.offsets, Vec::<usize>::new());
    }

    #[test]
    fn struct_layout_three_bools_packed_then_padded_to_align() {
        // Three bools = 3B, align 1; total stays 3 because max align = 1.
        let fields = vec![
            ("a".to_string(), FfiType::Bool),
            ("b".to_string(), FfiType::Bool),
            ("c".to_string(), FfiType::Bool),
        ];
        let layout = struct_layout(&fields);
        assert_eq!(layout.offsets, vec![0, 1, 2]);
        assert_eq!(layout.total, 3);
        assert_eq!(layout.align, 1);
    }

    #[test]
    fn signature_resolves_struct_via_registry() {
        let mut structs: HashMap<String, Vec<(String, FfiType)>> = HashMap::new();
        structs.insert("Reading".to_string(), vec![("v".to_string(), FfiType::Int)]);
        let d = decl("read_one", "read_one", vec![], "Reading");
        let sig = ForeignSignature::from_decl_with_structs(&d, &structs).expect("must resolve");
        match &sig.ret {
            FfiType::Struct { name, fields } => {
                assert_eq!(name, "Reading");
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].0, "v");
                assert_eq!(fields[0].1, FfiType::Int);
            }
            other => panic!("expected Struct, got {:?}", other),
        }
    }

    #[test]
    fn signature_rejects_unknown_struct_when_not_registered() {
        let d = decl("read_one", "read_one", vec![], "MissingStruct");
        let err = ForeignSignature::from_decl(&d).expect_err("unregistered struct name must error");
        assert!(
            matches!(err, FfiError::UnsupportedType(ref s) if s == "MissingStruct"),
            "got {:?}",
            err
        );
    }

    #[test]
    fn signature_resolves_struct_argument_via_registry() {
        let mut structs: HashMap<String, Vec<(String, FfiType)>> = HashMap::new();
        structs.insert(
            "Reading".to_string(),
            vec![
                ("temp".to_string(), FfiType::Int),
                ("ok".to_string(), FfiType::Bool),
            ],
        );
        let d = decl("read_temp", "read_temp", vec![("Reading", "r")], "Int");
        let sig = ForeignSignature::from_decl_with_structs(&d, &structs).expect("must resolve");
        assert_eq!(sig.params.len(), 1);
        match &sig.params[0] {
            FfiType::Struct { name, fields } => {
                assert_eq!(name, "Reading");
                assert_eq!(fields.len(), 2);
            }
            other => panic!("expected Struct param, got {:?}", other),
        }
        assert_eq!(sig.ret, FfiType::Int);
    }

    #[test]
    fn struct_too_large_error_message_mentions_size_and_max() {
        let err = FfiError::StructTooLarge {
            name: "Big".to_string(),
            size: 24,
            max: 8,
        };
        let msg = err.to_string();
        assert!(msg.contains("Big"), "msg = {}", msg);
        assert!(msg.contains("24"), "msg = {}", msg);
        assert!(msg.contains("8"), "msg = {}", msg);
    }
}

#[cfg(test)]
#[cfg(not(feature = "ffi"))]
mod disabled_tests {
    use super::*;

    #[test]
    fn disabled_loader_reports_ffi_disabled() {
        let mut loader = ForeignLoader::new();
        let err = loader
            .resolve_block("libanything", &[])
            .expect_err("disabled loader must error");
        assert!(matches!(err, FfiError::FfiDisabled), "got {:?}", err);
    }

    #[test]
    fn disabled_loader_lookup_returns_none() {
        let loader = ForeignLoader::new();
        assert!(loader.lookup("anything").is_none());
    }
}
