//! RES-510: thin bin shim over the Resilient library.
//!
//! The compiler / interpreter / typechecker / parser used to live
//! entirely inside this file (43k+ lines). They've now been moved
//! to `lib.rs` so non-CLI consumers (the WASM playground, future
//! tooling, integration-test harnesses) can depend on `resilient`
//! as a Rust library. This file's only job is to call into that
//! library's CLI entry point.

fn main() {
    const STACK_SIZE: usize = 16 * 1024 * 1024;
    let builder = std::thread::Builder::new().stack_size(STACK_SIZE);
    let handler = builder
        .spawn(resilient::run_cli)
        .expect("failed to spawn CLI thread");
    if let Err(e) = handler.join() {
        std::panic::resume_unwind(e);
    }
}
