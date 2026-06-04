# String Interning in Resilient

## Overview

String interning deduplicates identical string literals in compiled Resilient programs. This reduces binary size and enables O(1) string equality checks by pointer comparison.

## How It Works

### Compile-Time Interning

All string literals in your Resilient program are automatically collected and deduplicated at compile time. Each unique string is assigned a stable numeric ID.

Example:
```resilient
let error1 = "Cannot open file";    // ID: 0
let error2 = "Cannot open file";    // Same ID: 0 (deduplicated!)
let warning = "File may be large";  // ID: 1 (unique)
```

### Pool-Based Deduplication

The compiler maintains a global **interning pool** that:
- Maps string content to unique IDs
- Stores each unique string once in memory
- Enables O(1) equality checks when both strings are interned

### Runtime Support

Use the `intern()` builtin function to intern dynamically-created strings:

```resilient
fn main() {
    let dynamic = concat("hello", "world");
    let interned = intern(dynamic);  // Now deduplicated with other "helloworld" strings
}
```

## Benefits

### 1. Reduced Binary Size

- **Deduplication**: Each unique string stored once
- **Embedded systems**: Critical for flash-constrained environments
- **Typical savings**: 5-30% for string-heavy programs (see benchmarks)

Example: A program with these repeated strings:
```
"error: invalid input"      (3 copies)
"warning: deprecated"       (2 copies)
"info: processing"          (4 copies)
```

**Without interning**: ~200 bytes total  
**With interning**: ~70 bytes total  
**Savings**: ~65%

### 2. O(1) String Equality

Comparing two interned strings with the same ID is instant (just compare IDs):
```resilient
if "hello" == "hello" {     // O(1) when both literals are interned
    // ...
}
```

### 3. Memory Efficiency

Shared string storage reduces overall memory footprint:
- Lower heap fragmentation
- Better cache locality
- Beneficial for real-time systems

## Usage

### Automatic Interning

String literals are interned automatically—no action needed:

```resilient
let s1 = "hello";
let s2 = "hello";       // Automatically deduplicated
// s1 and s2 reference the same interned string
```

### Manual Interning

For dynamically-constructed strings, use `intern()`:

```resilient
fn process_error(code: i32, message: string) -> string {
    let error_msg = concat("Error ", to_string(code), ": ", message);
    let interned = intern(error_msg);  // Deduplicate
    return interned;
}
```

### Real-World Example: Logging

```resilient
enum LogLevel {
    ERROR,
    WARNING,
    INFO,
}

fn log(level: LogLevel, msg: string) {
    let prefix = match level {
        ERROR => intern("ERROR: "),       // Interned once, reused
        WARNING => intern("WARNING: "),   // Interned once, reused
        INFO => intern("INFO: "),         // Interned once, reused
    };
    
    let full_msg = concat(prefix, msg);
    print(full_msg);
}
```

## Limitations

### When String Interning Helps

- ✅ Programs with repeated string literals
- ✅ Configuration keys, error messages, log prefixes
- ✅ Embedded systems with limited flash
- ✅ Safety-critical systems minimizing size

### When String Interning Has Minimal Impact

- ❌ Programs with unique strings (no duplication)
- ❌ Very short programs
- ❌ Systems with abundant memory

## Implementation Details

### Global Interning Pool

- **Thread-safe**: Protected by `Mutex` (single-threaded REPL/compiler)
- **Persistent**: Lives for entire compilation session
- **Resettable**: Can be cleared between compilations (REPL, tests)

### AST Integration

New `StringInternLiteral` AST node tracks:
- `intern_id`: Index into the global pool
- `content`: Original string (for debugging/display)
- `span`: Source code location

### Type System

- `StringInternLiteral` is typed as `String`
- All string operations work transparently
- No API changes for users

## Performance Characteristics

| Operation | Time | Notes |
|-----------|------|-------|
| Literal comparison | O(1) | Both must be interned |
| Dynamic interning | O(n) | n = string length (HashMap lookup) |
| String operations | O(m) | m = string length (same as before) |

## Relationship to Stable Language

String interning is a **transparent optimization**. It doesn't change:
- Program semantics
- String equality behavior
- Type signatures
- Public API

Programs behave identically with or without interning.

## Future Enhancements

Possible improvements:
- Persist interning pool across REPL sessions
- LLVM-level optimization to merge identical strings
- Profile-guided interning (prioritize hot strings)
- Streaming interning for large programs

---

**See also:** [Benchmarks](./BENCHMARKS.md) for real-world size measurements, and [Architecture](./ARCHITECTURE_STRING_INTERNING.md) for implementation details.
