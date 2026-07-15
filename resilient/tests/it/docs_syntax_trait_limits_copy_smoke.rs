//! RES-3375: root syntax docs describe trait limitations with support boundaries.
//!
//! A-E3/RES-3933 test change: the assertions below were updated to match
//! `SYNTAX.md`'s corrected "Limitations" section. The previous text this
//! test pinned was stale on three counts by the time A-E3 landed:
//!
//! - "Projection syntax (`T::AssocType`) in generic bounds" had already
//!   shipped (RES-2695, `where T::AssocType: Bound` at call sites) — and
//!   A-E3 additionally makes `Self::AssocType` resolve and participate in
//!   real type checking inside a concrete `impl`'s own methods.
//! - "Default method bodies are not supported yet" was false — they shipped
//!   in `default_trait_methods.rs` (RES-2697).
//! - "Blanket impls and specialization are not supported yet" was false for
//!   blanket impls — they shipped in `blanket_impl.rs` (RES-2552).
//!
//! `dyn Trait` / vtable dispatch remains genuinely unsupported and is still
//! listed, now pointing at the A-E3 follow-up tracking issue (#4068)
//! instead of the stale `RES-293` reference.

#[test]
fn root_syntax_docs_describe_trait_limitations_without_future_labels() {
    let syntax = include_str!("../../../SYNTAX.md");

    for expected in [
        "`T::AssocType` projections for a generic type parameter `T` at an",
        "`dyn Trait` / virtual tables",
        "there is no",
        "dynamic dispatch",
        "Generic associated types (`type Item<T>;`) and associated constants",
        "are not supported yet",
        "Default trait method bodies",
        "blanket impls (`impl<T: Bound> Trait for T`)",
        "*are* supported today",
    ] {
        assert!(
            syntax.contains(expected),
            "root syntax docs should describe trait limitation boundaries: {expected:?}"
        );
    }

    for stale in [
        "Generic associated types (future)",
        "Default method bodies (future)",
        "Blanket impls or specialization (future)",
        // A-E3/RES-3933: these specific claims are now false — see the
        // module doc above for what shipped and when.
        "Default method bodies are not supported yet",
        "Blanket impls and specialization are not supported yet",
        "Projection syntax (`T::AssocType`) in generic bounds (RES-779 follow-up)",
    ] {
        assert!(
            !syntax.contains(stale),
            "root syntax docs should not use bare future labels or stale claims: {stale:?}"
        );
    }
}
