// Sum the integers 1..=N — tests loop overhead.

fn main() {
    let n = 100000;
    let total = 0;
    let i = 1;
    while i <= n {
        total = total + i;
        i = i + 1;
    }
    println(total);
}
main();
