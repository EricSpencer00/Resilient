// RES-172: VM peephole benchmark — tight counter loop that
// exercises the `LoadLocal x; Const 1; Add; StoreLocal x` → IncLocal
// fold on every iteration. A naive compile emits 4 ops per bump;
// the peephole collapses them to 1.
//
// Expected result: 1_000_000 after 1M iterations.

fn count_up() {
    let i = 0;
    while i < 1000000 {
        i = i + 1;
    }
    return i;
}

return count_up();
