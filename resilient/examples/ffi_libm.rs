// FFI example: call C's sqrt from libm.
// Linux: extern "libm.so.6" { ... }
// macOS: change to libm.dylib
extern "libm.so.6" {
    fn sqrt(x: Float) -> Float requires _0 >= 0.0 ensures result >= 0.0;
}

fn main() {
    println(sqrt(16.0));
    println(sqrt(2.0));
}
main();
