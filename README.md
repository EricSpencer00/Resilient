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
cargo run -- examples/sensor_monitor.rs

# With static type checking
cargo run -- --typecheck examples/sensor_monitor.rs

# With verification audit (shows static-vs-runtime contract coverage)
cargo run -- --audit examples/sensor_monitor.rs
```

### SMT-backed verification (optional)

Resilient ships with a hand-rolled contract verifier that handles
constant folding, let-binding propagation, control-flow assumptions,
and inter-procedural chaining. For contracts beyond that subset
(e.g. universal tautologies like `x + 0 == x`), build with the
optional `z3` feature to get full SMT-backed proofs:

```bash
# macOS:  brew install z3
# Linux:  sudo apt-get install libz3-dev z3
cargo run --features z3 -- --audit prog.rs
```

The audit report tags clauses proven by Z3 separately so users can
see what the SMT layer added.

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

## Examples

The `new_examples` directory contains working examples of the Resilient language:

- `new_examples/simple.rs` - A minimal hello world program
- `new_examples/live_block_demo.rs` - Demonstrates live blocks with enhanced logging

> **Note**: If you encounter issues with existing examples in the `examples` directory, use the new examples as reference for the correct syntax and structure.

## Syntax Requirements

See the [SYNTAX.md](SYNTAX.md) file for detailed syntax requirements and examples. Key points:

- Function parameters carry explicit types; zero-parameter functions use `fn name()`
- Static variables maintain state between function calls
- Live blocks provide self-healing capabilities
- Assertions validate system invariants

## Project Status

Active development happens one ticket at a time. See [.board/ROADMAP.md](.board/ROADMAP.md)
for the goalpost ladder and [.board/tickets/DONE/](.board/tickets/DONE/) for
the full ledger. Each commit of the form `RES-NNN: summary` closes one ticket.

### What works today

- Functions (with and without parameters), forward references
- `let` and `static let` bindings, reassignment
- Arithmetic, comparison, logical, bitwise, and shift operators
- Prefix `!` and `-`
- Hex (`0xFF`) and binary (`0b1010`) integer literals with `_` separators
- Block `/* */` and line `//` comments
- `if` / `else`, `while` (with runaway guard)
- `live { }` self-healing blocks with retry
- `assert(cond, msg)` with operand values in the error
- Built-ins: `println`, `print`, `len`, `abs`, `min`, `max`, `sqrt`,
  `pow`, `floor`, `ceil`
- Clean `line:col:` error diagnostics
- 50+ passing tests covering lexer, parser, typechecker, interpreter,
  and example programs (golden file sidecars in `resilient/examples/`)
- Zero panic paths in the parser or lexer — every error is recoverable

### What's next

- G4 (full source spans with snippets / carets)
- G5 (replace hand-rolled lexer with `logos`)
- G6 (one canonical AST, retire the unwired `parser.rs`)
- G7 (real type checker: inference, unification, exhaustiveness)
- G8–G10 (function contracts, symbolic assert, live-block invariants)
- G11+ (stdlib, structs, pattern matching, cranelift backend, LSP,
  `no_std`, self-hosting)