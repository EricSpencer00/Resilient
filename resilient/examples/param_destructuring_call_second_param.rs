// Valid: destructuring at parameter position 1
fn process(int x, (int, int) _a_b) {
    return;
}

fn main() {
    process(100, 5, 7);
}
