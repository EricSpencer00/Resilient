# Resilient

A programming language designed for extreme reliability in embedded and safety-critical systems.

## Core Philosophy

Resilient is a statically-typed, compiled programming language designed from the ground up for extreme reliability in embedded and safety-critical systems. Its core philosophy is built on three pillars:

### Resilience
Failures are inevitable. Resilient treats them as expected events, not exceptions. The language provides built-in mechanisms for code to "self-heal" and continue execution, ensuring the system never enters a non-functional state.

### Verifiability
It shouldn't just work; it must be provably correct. Resilient integrates concepts from formal methods to allow developers to define and verify system invariants at compile time.

### Simplicity
The syntax is designed to be minimal and unambiguous, reducing the cognitive load on the developer and minimizing the surface area for bugs.

## Target Applications

- **Automotive**: Engine control units (ECUs), autonomous driving systems, braking systems.
- **Aerospace**: Flight control systems, drone autopilots.
- **Industrial Automation**: Robotic arms, safety controllers on manufacturing lines.
- **Medical Devices**: Infusion pumps, monitoring equipment.

## Key Features

### The "Live" Block: Self-Healing Code

The cornerstone of Resilient is the live block. Any code within a live block is supervised by the Resilient runtime. If a recoverable error occurs within this block, the runtime will not panic or halt. Instead, it will reset the state of the block to its last known-good state and re-execute it.

```
// Example of a live block that handles division by zero
live {
    let result = 100 / user_input;
    println("Result: " + result);
}
```

### Formal Methods Lite: Invariants with assert

For the MVP, we introduce a simple form of formal verification using an enhanced assert macro. These assertions define critical system invariants—conditions that must always be true.

```
// System invariant that must not be violated
assert(fuel_level >= 0, "Fuel level cannot be negative");
```

### Static Type Checking

Resilient includes static type checking to catch type errors before runtime. This helps prevent common errors and ensures more reliable code.

```
// Function with typed parameters
fn calculate_velocity(float distance, float time) {
    return distance / time;
}
```

### Improved Error Handling

Resilient provides detailed error messages and has sophisticated error recovery mechanisms, especially within live blocks.

## Getting Started

### Running the REPL

```bash
cd resilient
cargo run
```

### Running an Example

```bash
cd resilient
./run_example.sh minimal

# With type checking enabled
./run_example.sh minimal --typecheck
```

### Available Examples

- `minimal.rs` - A minimal working example that demonstrates basic functionality
- `comprehensive.rs` - Demonstrates all key language features in a single example
- `sensor_example2.rs` - Demonstrates live blocks with sensor readings
- `self_healing2.rs` - Shows self-healing capabilities

### REPL Commands

- `help` - Show help message
- `exit` - Exit the REPL
- `clear` - Clear the screen
- `examples` - Show example code snippets
- `typecheck` - Toggle type checking

## Example Programs

### Sensor Reading Example

The `examples/sensor_example2.rs` file demonstrates how to use live blocks and assertions to create a resilient sensor reading system.

### Self-Healing Example

The `examples/self_healing2.rs` file shows how Resilient can recover from failures and continue operation.

## Syntax Requirements

See the [SYNTAX.md](SYNTAX.md) file for detailed syntax requirements and examples. Key points:

- Functions must have parameters with explicit types, even if they're not used
- Static variables maintain state between function calls
- Live blocks provide self-healing capabilities
- Assertions validate system invariants

## Project Status

This is an MVP (Minimum Viable Product) implementation of the Resilient language. Future improvements will include:

- More sophisticated type system
- Ownership and borrowing model
- Compiler optimizations
- Enhanced formal verification
- Better tooling and IDE support