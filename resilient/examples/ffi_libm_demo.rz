// FFI v2 demo: call libm math functions from Resilient via the C ABI.
// macOS: libm.dylib  Linux: change to "libm.so.6"
// Run: cargo run --features ffi -- examples/ffi_libm_demo.res

extern "libm.dylib" {
    fn cos(x: Float) -> Float;
    fn sin(x: Float) -> Float;
    fn sqrt(x: Float) -> Float;
}

fn main() {
    println(cos(0.0));
    println(sin(0.0));
    println(sqrt(4.0));
    println(sqrt(2.0));
}

main();
