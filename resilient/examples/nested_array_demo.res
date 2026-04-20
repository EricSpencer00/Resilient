// RES-034 demo: nested index assignment a[i][j] = v.
// Build a 2x3 matrix, mutate one cell, print every cell to confirm
// only the addressed cell changed.
fn main() {
    let m = [[1, 2, 3], [4, 5, 6]];
    m[1][1] = 99;
    println(m[0][0]);
    println(m[0][1]);
    println(m[0][2]);
    println(m[1][0]);
    println(m[1][1]);
    println(m[1][2]);
}

main();
