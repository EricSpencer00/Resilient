fn add(int a, int b) requires a >= 0 {
    return a + b;
}

fn main() {
    let x = add(1, 2);
    if x > 0 {
        println(x);
    }
}
main();
