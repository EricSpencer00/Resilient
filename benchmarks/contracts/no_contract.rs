// Same workload, no contract — measures the cost of the requires check.
fn unsafe_div(int a, int b) {
    return a / b;
}

fn main() {
    let i = 0;
    let total = 0;
    while i < 100000 {
        total = total + unsafe_div(100, 7);
        i = i + 1;
    }
    println(total);
}
main();
