// Non-interference proof by self-composition: `prove_noninterference`
// encodes the body twice (sharing the low param, freeing the high
// param) and proves the two outputs always agree.
#[noninterference(low = "data", high = "key")]
fn mask(int data, int key) -> int {
    return (data + key) - key;
}

fn main() {
    println(mask(7, 99));
}

main();
