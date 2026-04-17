// RES-073: minimum-viable module imports demo.
// `use "path";` pulls in every top-level fn from the referenced file.
use "helpers.rs";

fn main() {
    let result = square(7);
    shout("imports work");
    println(result);
}

main();
