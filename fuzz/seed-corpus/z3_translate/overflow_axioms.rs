// LIA tautology under an axiom the hand-rolled folder can't decide
// alone: `prove_with_axioms_and_timeout` / `prove_auto` have to hand
// this to Z3.
pure fn heavy(int x, int y) -> int
    requires x >= 0 && y >= 0
    ensures result == x + y
    ensures result >= 0
{
    let s = x + y;
    return s;
}

fn main() {
    println(heavy(3, 4));
}

main();
