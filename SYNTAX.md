# Resilient Language Syntax Guide

This document outlines the syntax requirements and quirks discovered during implementation of the Resilient programming language.

## Function Declarations

Functions declare their parameters with types. Zero-parameter
functions are written with empty parentheses, just like you'd expect:

```rust
// Both are valid
fn main() {
    println("Hello, world!");
}

fn add(int a, int b) {
    return a + b;
}
```

### Function Calls

When calling functions, provide values for each declared parameter:

```rust
main();
let sum = add(2, 3);
process_data(42, "sensor1");
```

> **Historical note**: older versions of Resilient required every
> function to declare at least one parameter (the "dummy parameter"
> workaround) because of a parser limitation. That limitation is gone
> as of RES-004; you can and should write `fn main()` going forward.

## Variable Declarations

Variables are declared using the `let` keyword:

```rust
let x = 42;
let name = "Resilient";
```

## Static Variables

Static variables maintain their values between function calls:

```rust
static let counter = 0;
counter = counter + 1; // Increments across calls
```

## Live Blocks

Live blocks provide self-healing functionality:

```rust
live {
    // Code in this block will be retried if an assertion fails
    let sensor_value = read_sensor(0);
    assert(is_valid_reading(sensor_value), "Invalid reading");
    process_data(sensor_value, threshold);
}
```

## Assertions

Assertions validate system invariants:

```rust
assert(condition, "Error message if condition fails");
```

## Data Types

Resilient supports these basic types:
- `int`: Integer values
- `float`: Floating-point values
- `string`: Text strings
- `bool`: Boolean values (true/false)

## Control Flow

```rust
if condition {
    // Code when condition is true
} else {
    // Code when condition is false
}
```

## Working Examples

See the `examples/` directory for working examples:
- `sensor_example2.rs`: Demonstrates sensor reading with live blocks
- `self_healing2.rs`: Shows self-healing capabilities
- `test.rs`: A minimal working example

## Compiling and Running

```bash
# Run a program
cargo run -- examples/test.rs

# Run with type checking
cargo run -- --typecheck examples/test.rs

# Start the REPL
cargo run
```
