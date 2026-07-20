//! RES-510: thin bin shim over the Resilient library.
//!
//! The compiler / interpreter / typechecker / parser used to live
//! entirely inside this file (43k+ lines). They've now been moved
//! to `lib.rs` so non-CLI consumers (the WASM playground, future
//! tooling, integration-test harnesses) can depend on `resilient`
//! as a Rust library. This file's only job is to call into that
//! library's CLI entry point.

fn main() {
    // RES-4190: 16 MiB was enough headroom for the parser's own
    // recursion, but the typechecker's `check_node` match arm is much
    // heavier per stack frame (large `Node`/`Type` locals across a
    // 19k-line match) — in debug builds it overflowed at ~180 nested
    // frames, well under the parser's MAX_EXPR_DEPTH (500). Bumped to
    // 96 MiB so a full ~550-deep (`check_node`'s own guard,
    // MAX_CHECK_DEPTH in typechecker.rs) parser-accepted AST
    // typechecks cleanly instead of aborting.
    const STACK_SIZE: usize = 96 * 1024 * 1024;
    let builder = std::thread::Builder::new().stack_size(STACK_SIZE);
    let handler = builder
        .spawn(resilient::run_cli)
        .expect("failed to spawn CLI thread");
    if let Err(e) = handler.join() {
        std::panic::resume_unwind(e);
    }
}
