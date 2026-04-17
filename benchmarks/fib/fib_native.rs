// Native Rust baseline. NOT Resilient — `.rs` extension here means
// actual Rust source, compiled with rustc -O3.
fn fib(n: u64) -> u64 {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}
fn main() { println!("{}", fib(25)); }
