// RES-144: interactive demo exercising the `input()` builtin.
//
// `input(prompt)` prints the (possibly empty) prompt, flushes stdout,
// reads one line from stdin, and returns the contents with the
// trailing newline stripped. EOF before any data is an empty string,
// so ctrl-D exits the loop cleanly without raising.
//
// Run with:
//     cargo run --example interactive_greeter
// or simply:
//     cargo run -- examples/interactive_greeter.rs
//
// The accompanying `.interactive` sidecar marks this file as
// "don't exec in CI" — the golden-test harness skips it so builds
// don't hang waiting on stdin.

fn main(int _d) {
    let name = input("What is your name? ");
    if name == "" {
        println("(no name given — goodbye!)");
        return 0;
    }
    println("Hello, " + name + "!");
    return 0;
}

main(0);
