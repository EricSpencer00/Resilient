fn safe_div(int a, int b) requires b != 0 ensures result * b <= a {
    return a / b;
}

fn main() {
    let r = safe_div(10, 2);
    println(r);
}
main();
