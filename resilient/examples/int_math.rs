// RES-055 demo: int-pure pipeline through pow/floor/ceil. None of
// these calls demote to Float, so the program never touches the
// soft-float library on a no_std target.
fn main() {
    let p = pow(2, 8);
    let f = floor(7);
    let c = ceil(-3);
    println(p);
    println(f);
    println(c);
}

main();
