// RES-381: match guard demo — temperature classifier.
//
// Demonstrates `pattern if guard => body` arms:
//   - guarded identifier arms fall through when the guard is false
//   - an unguarded catch-all arm handles the remaining cases
//   - pattern bindings (here `t`) are visible inside the guard

fn classify(float temp) -> string {
    match temp {
        t if t < -273.15 => "below absolute zero",
        t if t < 0.0     => "freezing",
        t if t <= 100.0  => "normal",
        t                => "above boiling"
    }
}

fn main() {
    println(classify(-300.0));
    println(classify(-10.0));
    println(classify(37.0));
    println(classify(150.0));
}

main();
