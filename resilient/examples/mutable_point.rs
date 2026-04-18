// RES-153: struct field assignment demo.
//
// Creates a Point, mutates each field in place via `p.x = ...`, and
// prints the result. Uses the interpreter's tree-walker path; the VM
// and JIT pick up struct ops in follow-up tickets (RES-165 / RES-170).

struct Point {
    int x,
    int y,
}

fn main() {
    let p = new Point { x: 1, y: 2 };
    println("start: (" + p.x + ", " + p.y + ")");

    p.x = 10;
    p.y = 20;
    println("after:  (" + p.x + ", " + p.y + ")");
}

main();
