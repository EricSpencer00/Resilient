//! RES-072: Cranelift JIT backend — Phase A scaffolding.
//!
//! This file is the **foundation** for the JIT track. It declares
//! the entry point (`run`), the error type (`JitError`), and the
//! public API the driver hooks into via `--jit`. Actual AST
//! lowering to Cranelift IR is split into follow-up tickets:
//!
//! - **RES-096** lowers the smallest subset (IntegerLiteral + Add)
//!   to native code and runs it.
//! - **RES-097** adds control flow + function calls.
//! - **RES-098** adds top-level `main() -> int` that JIT-runs the
//!   whole program.
//! - **RES-099** is the fib(25) microbench under `--jit`.
//!
//! Until RES-096 lands, `run()` returns
//! `JitError::Unsupported("jit not implemented yet ...")` so the
//! driver surfaces a clear "not yet built" error rather than
//! pretending to compile.
//!
//! The `mod jit_backend;` declaration in `main.rs` is gated on
//! `cfg(feature = "jit")`, so this file is only compiled when the
//! feature is on.

#![allow(dead_code)]

// Pull in cranelift / cranelift-jit at the module level so the
// build verifies they link. Real use lands with RES-096.
#[allow(unused_imports)]
use cranelift::prelude::*;
#[allow(unused_imports)]
use cranelift_jit::{JITBuilder, JITModule};

use crate::Node;

/// Errors the JIT backend can surface. Phase A only emits
/// `Unsupported`; future tickets add `IsaInit`, `LinkError`, etc.
#[derive(Debug, Clone, PartialEq)]
pub enum JitError {
    Unsupported(&'static str),
}

impl std::fmt::Display for JitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JitError::Unsupported(what) => write!(f, "jit: unsupported: {}", what),
        }
    }
}

impl std::error::Error for JitError {}

/// RES-072 Phase A: stub entry point. Returns `Unsupported` until
/// RES-096+ lands actual AST lowering. The driver's `--jit` flag
/// dispatches to this so the dep + flag plumbing is verified
/// end-to-end before any JIT logic is written.
///
/// Returns an `i64` because that's the eventual contract: a JIT-
/// compiled `main() -> int` returns its int value through the
/// process's exit code path (or through the function pointer the
/// JITModule hands back). The plumbing is built around that
/// promise even though this stub never produces an `Ok`.
pub fn run(_program: &Node) -> Result<i64, JitError> {
    Err(JitError::Unsupported(
        "jit not implemented yet — RES-096+ adds AST lowering",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_returns_unsupported_until_res_096() {
        // Sanity check that the stub plumbs through and the deps
        // link. Provide a synthetic empty Program; the stub
        // ignores the AST entirely.
        let program = Node::Program(Vec::new());
        let err = run(&program).expect_err("phase A should always return Unsupported");
        match err {
            JitError::Unsupported(msg) => {
                assert!(
                    msg.contains("RES-096"),
                    "msg should point at follow-up ticket: {}",
                    msg
                );
            }
        }
    }

    #[test]
    fn jit_error_display_is_descriptive() {
        let e = JitError::Unsupported("test case");
        assert_eq!(e.to_string(), "jit: unsupported: test case");
    }
}
