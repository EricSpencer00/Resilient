// Non-interference VIOLATION: self-composition finds a counter-
// example (two `secret` values that, with `data` fixed, yield
// different outputs) instead of a proof.
#[noninterference(low = "data", high = "secret")]
fn leaky(int data, int secret) -> int {
    return data + secret;
}

fn main() {
    println(leaky(1, 2));
}

main();
