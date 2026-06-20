// Valid: destructuring at parameter 0, call with extra arguments
fn unpack_and_add((int, int) _a_b, int c) {
    return;
}

fn main() {
    unpack_and_add(10, 20, 30);
}
