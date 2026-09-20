//! Shared safety limits for builtins that eagerly generate collections.

/// Maximum number of elements a single builtin may generate eagerly.
pub(crate) const MAX_GENERATED_ELEMENTS: usize = 10_000_000;
