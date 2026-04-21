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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

impl FfiType {
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
    pub fn from_decl(decl: &ExternDecl) -> Result<Self, FfiError> {
        let mut params = Vec::with_capacity(decl.parameters.len());
        for (ty, _) in &decl.parameters {
            params.push(
                FfiType::from_resilient(ty).ok_or_else(|| FfiError::UnsupportedType(ty.clone()))?,
            );
        }
        let ret = FfiType::from_resilient(&decl.return_type)
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
    use std::collections::HashMap;

    pub struct ForeignLoader {
        libs: HashMap<String, libloading::Library>,
        syms: HashMap<String, std::sync::Arc<ForeignSymbol>>,
    }

    impl ForeignLoader {
        pub fn new() -> Self {
            Self {
                libs: HashMap::new(),
                syms: HashMap::new(),
            }
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
                let sig = ForeignSignature::from_decl(d)?;
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
