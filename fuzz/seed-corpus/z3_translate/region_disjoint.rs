// Region alias-disjointness: the syntactic rule rejects two same-
// region `&mut` params, but `requires(a != b)` routes the obligation
// to `prove_alias_disjoint`.
region A;

fn update_disjoint(&mut[A] int a, &mut[A] int b) requires(a != b) {
    println("ok: Z3 proved a and b are disjoint");
}

fn main() {
    update_disjoint(1, 2);
}

main();
