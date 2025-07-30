// A minimal test file for the Resilient language
fn test_func(int x) {
    return x + 1;
}

fn main(int dummy) {
    let result = test_func(5);
    println("Result: " + result);
}

main(0);
