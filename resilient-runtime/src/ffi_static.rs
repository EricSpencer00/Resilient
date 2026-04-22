//! FFI static registry for no_std embedded hosts.
//!
//! The embedding application calls `register` on a `StaticRegistry` BEFORE
//! dispatching any foreign calls. Lookups are O(N) linear scan over a fixed-size
//! array — allocation-free and no_std-clean.
//!
//! Capacity defaults to 64 entries. Override with exactly ONE of:
//! `--features ffi-static-64`, `--features ffi-static-256`, `--features ffi-static-1024`.
//! Enabling multiple capacity features is a compile error.

// Mutual-exclusion compile_error — only one capacity feature at a time.
#[cfg(all(feature = "ffi-static-64", feature = "ffi-static-256"))]
compile_error!("`ffi-static-64` and `ffi-static-256` are mutually exclusive — pick ONE.");
#[cfg(all(feature = "ffi-static-64", feature = "ffi-static-1024"))]
compile_error!("`ffi-static-64` and `ffi-static-1024` are mutually exclusive — pick ONE.");
#[cfg(all(feature = "ffi-static-256", feature = "ffi-static-1024"))]
compile_error!("`ffi-static-256` and `ffi-static-1024` are mutually exclusive — pick ONE.");

#[cfg(feature = "ffi-static-1024")]
const CAPACITY: usize = 1024;
#[cfg(all(feature = "ffi-static-256", not(feature = "ffi-static-1024")))]
const CAPACITY: usize = 256;
#[cfg(all(
    feature = "ffi-static-64",
    not(any(feature = "ffi-static-256", feature = "ffi-static-1024"))
))]
const CAPACITY: usize = 64;
#[cfg(not(any(
    feature = "ffi-static-64",
    feature = "ffi-static-256",
    feature = "ffi-static-1024"
)))]
const CAPACITY: usize = 64;

/// FFI primitive type tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FfiType {
    Int,
    Float,
    Bool,
    Str,
    Void,
}

/// Signature of a registered foreign function.
#[derive(Clone, Copy, Debug)]
pub struct ForeignSignature {
    pub params: &'static [FfiType],
    pub ret: FfiType,
}

/// A C function pointer, erased to the minimum required type for storage.
pub type ForeignFn = unsafe extern "C" fn();

/// One slot in the static registry.
#[derive(Copy, Clone)]
pub struct Entry {
    pub name: &'static str,
    pub ptr: ForeignFn,
    pub sig: ForeignSignature,
}

/// Errors returned by registry operations.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FfiError {
    /// All `CAPACITY` slots are occupied.
    RegistryFull,
    /// A function with this name is already registered.
    DuplicateSymbol,
    /// No function with this name was found.
    NotFound,
}

/// Fixed-capacity registry of foreign function pointers.
///
/// Construct with `StaticRegistry::new()`, register entries with
/// [`register`][StaticRegistry::register], then hand a reference to the
/// runtime for dispatch.
pub struct StaticRegistry {
    slots: [Option<Entry>; CAPACITY],
    len: usize,
}

impl StaticRegistry {
    /// Create an empty registry. `const`-compatible.
    pub const fn new() -> Self {
        const NONE: Option<Entry> = None;
        Self {
            slots: [NONE; CAPACITY],
            len: 0,
        }
    }

    /// Register a foreign function. Returns `Err` if the registry is full or
    /// the name is already taken.
    pub fn register(
        &mut self,
        name: &'static str,
        ptr: ForeignFn,
        sig: ForeignSignature,
    ) -> Result<(), FfiError> {
        if self.lookup(name).is_some() {
            return Err(FfiError::DuplicateSymbol);
        }
        if self.len == CAPACITY {
            return Err(FfiError::RegistryFull);
        }
        self.slots[self.len] = Some(Entry { name, ptr, sig });
        self.len += 1;
        Ok(())
    }

    /// Look up a registered function by name. O(N) scan.
    pub fn lookup(&self, name: &str) -> Option<&Entry> {
        self.slots[..self.len]
            .iter()
            .flatten()
            .find(|&e| e.name == name)
            .map(|v| v as _)
    }

    /// Number of registered entries.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True if no entries are registered.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

// Default: empty registry.
impl Default for StaticRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe extern "C" fn dummy() {}

    const SIG: ForeignSignature = ForeignSignature {
        params: &[],
        ret: FfiType::Void,
    };

    #[test]
    fn register_then_lookup() {
        let mut r = StaticRegistry::new();
        r.register("f", dummy, SIG).unwrap();
        assert!(r.lookup("f").is_some());
    }

    #[test]
    fn lookup_missing_returns_none() {
        let r = StaticRegistry::new();
        assert!(r.lookup("nope").is_none());
    }

    #[test]
    fn duplicate_registration_errors() {
        let mut r = StaticRegistry::new();
        r.register("f", dummy, SIG).unwrap();
        let err = r.register("f", dummy, SIG).unwrap_err();
        assert_eq!(err, FfiError::DuplicateSymbol);
    }

    #[test]
    fn full_registry_errors_on_next_registration() {
        let mut r = StaticRegistry::new();
        for i in 0..CAPACITY {
            // Box::leak is only used in test code (runs on host with std).
            let name: &'static str = Box::leak(format!("f{}", i).into_boxed_str());
            r.register(name, dummy, SIG).unwrap();
        }
        let err = r.register("overflow", dummy, SIG).unwrap_err();
        assert_eq!(err, FfiError::RegistryFull);
    }
}
