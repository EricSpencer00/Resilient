// Each call to safe_div fires a runtime `requires` check.
// 1,000,000 calls.
fn safe_div(int a, int b) requires b != 0 {
    return a / b;
}

fn main() {
    let i = 0;
    let total = 0;
    while i < 100000 {
        total = total + safe_div(100, 7);
        i = i + 1;
    }
    println(total);
}
main();
