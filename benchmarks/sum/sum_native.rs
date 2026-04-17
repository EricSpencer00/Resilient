// Native Rust baseline.
fn main() {
    let n: u64 = 100_000;
    let mut total: u64 = 0;
    let mut i: u64 = 1;
    while i <= n {
        total += i;
        i += 1;
    }
    println!("{}", total);
}
