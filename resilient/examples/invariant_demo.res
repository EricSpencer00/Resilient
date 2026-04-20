// RES-222: loop invariant demo — bounded counter loop.
// invariant i >= 0 && i <= n; holds throughout the loop.
fn count_up(int n) {
    let i = 0;
    while i < n {
        invariant i >= 0 && i <= n;
        i = i + 1;
    }
    return i;
}

let result = count_up(5);
println("count_up(5) = " + result);

// Multiple invariants in one loop.
fn bounded_sum(int limit) {
    let i = 0;
    let s = 0;
    while i < limit {
        invariant i >= 0;
        invariant s >= 0;
        s = s + i;
        i = i + 1;
    }
    return s;
}

let total = bounded_sum(5);
println("bounded_sum(5) = " + total);
