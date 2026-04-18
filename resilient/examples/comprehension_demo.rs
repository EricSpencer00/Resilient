// RES-156: array comprehension demo. `[<expr> for <binding> in
// <iterable> (if <guard>)?]` is parser sugar for an immediately-
// invoked fn that threads an accumulator — one `for` clause plus
// at most one optional `if` filter. Works over any iterable the
// language's `for ... in` supports (Arrays today; Set callers use
// `set_items` from RES-149 to lift into an Array first).

fn main(int _d) {
    let xs = [1, 2, 3, 4, 5, 6];

    // Simple map: double each element.
    let doubled = [x * 2 for x in xs];
    println(doubled);

    // Map with filter: keep only evens, then square.
    let even_sq = [x * x for x in xs if x % 2 == 0];
    println(even_sq);

    // Over a Set via set_items (sorted).
    let s = #{3, 1, 4, 1, 5, 9, 2, 6};
    let from_set = [x for x in set_items(s) if x >= 4];
    println(from_set);

    return 0;
}

main(0);
