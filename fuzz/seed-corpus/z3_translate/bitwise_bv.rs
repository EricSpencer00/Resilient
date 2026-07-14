// Bitwise requires/ensures clause: `has_bitwise_ops` selects the
// BV32 theory (RES-354) instead of unbounded LIA.
pure fn masked_add(int a, int b)
    requires (a & 255) >= 0
    ensures (result | 0) == a + b
{
    return a + b;
}

fn main() {
    println(masked_add(3, 4));
}

main();
