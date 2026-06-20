// Valid: destructuring function with three elements called correctly
fn unpack_triple((int, int, int) _x_y_z) {
    return;
}

fn main() {
    unpack_triple(1, 2, 3);
}
