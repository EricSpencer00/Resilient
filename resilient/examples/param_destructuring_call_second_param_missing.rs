// Error: destructuring at position 1, but only 1 argument provided
fn process(int x, (int, int) _a_b) {
    return;
}

fn main() {
    process(100);
}
