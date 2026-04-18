//! RES-069 + RES-115: source-position infrastructure.
//!
//! Types live in the sibling `resilient-span` crate so downstream
//! tooling (LSP, future debug-info JIT, fmt, lint) can depend on
//! just the span layer without pulling in the whole compiler. This
//! file is a thin re-export shim — existing imports like
//! `use span::{Pos, Span, Spanned};` / `crate::span::Pos` resolve
//! unchanged.
//!
//! If you're adding new span-shaped types (e.g. a `Spanned<Diag>`
//! wrapper), put them in `resilient-span/src/lib.rs`; nothing new
//! should land here.
#![allow(dead_code)]
// Re-exports not used in the default build (no logos-lexer feature)
// still need to be available via `crate::span::*` for the
// `lexer_logos` module and the test suite — silence the
// `unused_imports` lint on this shim.
#![allow(unused_imports)]

pub use resilient_span::{Pos, Span, Spanned, build_line_table, pos_from_byte};
