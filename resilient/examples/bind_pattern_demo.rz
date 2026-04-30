// RES-234: bind-pattern (`name @ inner`) demo.
//
// Demonstrates the three core uses of Pattern::Bind landed in RES-161a:
//   1. `n @ _`       — unconditional bind; n holds the whole scrutinee.
//   2. `n @ <lit>`   — bind only when the inner literal matches.
//   3. guard on bind — `n @ _ if n > 0` refines with a runtime test.

fn main() {
    // 1. Unconditional bind: n @ _ always matches.
    let a = match 42 {
        n @ _ => n + 1,
    };
    println("unconditional: " + a);

    // 2. Literal bind: n @ 5 matches only when scrutinee == 5.
    let b = match 5 {
        n @ 5 => n * 2,
        _ => 0,
    };
    println("literal hit: " + b);

    // 3. Guard on bind: n @ _ if n > 0 tests the bound value.
    let c = match 10 {
        n @ _ if n > 0 => n,
        _ => -1,
    };
    println("guard positive: " + c);
}

main();
