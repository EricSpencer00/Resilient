// RES-161a: name @ inner bind-pattern demo.
// `name @ inner` binds the scrutinee to `name` and simultaneously
// tests it against `inner`. All three forms are exercised below.

fn main(int _d) {
    // 1. `name @ _` — unconditional bind; v holds the whole value.
    let a = match 42 {
        v @ _ => v,
    };
    println(a);

    // 2. `name @ <literal>` — bind only on an exact match, fall through otherwise.
    let b = match 7 {
        v @ 5 => v * 10,
        _ => 0,
    };
    println(b);

    // 3. Guard using the bound name: fires only when n > 0.
    let c = match 3 {
        n @ _ if n > 0 => n + 100,
        _ => 0,
    };
    println(c);

    return 0;
}

main(0);
