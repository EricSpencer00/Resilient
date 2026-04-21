// RES-233: demonstrate assume() — runtime assumption checks.
// assume(expr) passes silently when the expression is true.
// assume(expr, "msg") attaches a custom failure message.
fn main() {
    let x = 42;
    assume(x > 0);
    assume(x > 0, "x must be positive");
    let y = x + 1;
    println("assume passed, y = " + y);
}

main();
