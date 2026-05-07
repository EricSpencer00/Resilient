# 10 Major Missing Features in Resilient

Based on analysis of the current compiler implementation and git history, these are the highest-impact language features that would unlock significant new capabilities:

## 1. Closures & Lambda Expressions

**Status**: Skeleton code exists (RES-164/169a) but incomplete  
**Impact**: Essential for functional programming, callbacks, method chains  
**Current state**: `free_vars` analysis exists but no lambda syntax  
**Examples**:
```rust
let double = |x: int| { x * 2 };
let doubled = array_map([1, 2, 3], |v| v * 2);
```

## 2. Complete Enum/Sum Types

**Status**: RES-400 PR1 complete (payload-less variants), needs PR2-5  
**Impact**: Core language feature for type-safe alternatives  
**Current state**: Can't store data in variants, no pattern matching, no exhaustiveness checking  
**Examples**:
```rust
enum Shape {
    Circle { r: float },
    Rect { w: float, h: float },
}
match s {
    Shape::Circle { r } => 3.14 * r * r,
    Shape::Rect { w, h } => w * h,
}
```

## 3. Function Types as First-Class Values

**Status**: Not implemented  
**Impact**: Higher-order functions, callbacks, dependency injection  
**Current state**: Can pass functions by reference only through trait dispatch  
**Examples**:
```rust
fn apply(callback: fn(int) -> int, x: int) -> int {
    return callback(x);
}
```

## 4. Generic Functions

**Status**: Only traits have generics (RES-783+)  
**Impact**: Type-safe generic containers and algorithms  
**Current state**: `fn foo<T>(x: T) -> T` syntax not supported  
**Examples**:
```rust
fn swap<T>(a: T, b: T) -> (T, T) { return (b, a); }
fn bsearch<T>(arr: Array<T>, target: T) -> Option<int> { ... }
```

## 5. Variadic Functions

**Status**: Not implemented  
**Impact**: Functions with variable argument counts (print, format, sum)  
**Current state**: All functions have fixed arity  
**Examples**:
```rust
fn sum(int ...args) -> int { ... }
fn format(string fmt, ...args) -> string { ... }
```

## 6. Macros (Compile-Time Code Generation)

**Status**: Not implemented  
**Impact**: DSLs, assertion macros, reducing boilerplate  
**Current state**: No metaprogramming facilities  
**Examples**:
```rust
#[macro]
fn assert_eq(a, b) {
    if a != b {
        panic("assertion failed: {a} != {b}");
    }
}
```

## 7. Module System

**Status**: Basic textual include/import only  
**Impact**: Code organization, namespacing, visibility control  
**Current state**: Files are spliced inline; no `mod` keyword, no visibility modifiers  
**Examples**:
```rust
mod math { fn sin(float x) -> float { ... } }
mod graphics { use math::sin; }
use graphics::draw;
```

## 8. Operator Overloading

**Status**: Not implemented  
**Impact**: Custom types can define operators (Vector + Vector, Complex * Complex)  
**Current state**: Only built-in operators work on built-in types  
**Examples**:
```rust
struct Vector { float x, float y, }
impl Add for Vector { fn add(self, other: Vector) -> Vector { ... } }
let v3 = v1 + v2;
```

## 9. String Interpolation

**Status**: Not implemented  
**Impact**: Readable string formatting without verbose concatenation  
**Current state**: Must concatenate or use separate string parts  
**Examples**:
```rust
let x = 42;
let msg = f"The answer is {x}";
let formatted = "x={x}, y={y}";
```

## 10. Error Propagation Operator (?)

**Status**: Not implemented  
**Impact**: Ergonomic Result/Option handling  
**Current state**: Manual unwrap/unwrap_or calls required  
**Examples**:
```rust
fn risky() -> Result<int, string> {
    let x = fallible()?;
    let y = another_risky_call()?;
    return ok(x + y);
}
```

---

## Honorable Mentions

**Lower priority but also valuable**:
- Recursive type definitions (current limitation prevents recursive structs/enums)
- Async/await (for I/O and long-running operations)
- Const functions (compile-time evaluation, `const fn`)
- Pattern matching improvements (struct/record patterns, guard clauses)
- Better type inference (full Hindley-Milner; RES-120 WIP, gated on `infer` feature)
- Destructuring in function parameters (`fn foo((int x, int y)) { ... }`)
- Default trait methods (trait implementations can provide default bodies)
- Attributes and derives (`#[derive(Debug)]`, custom attributes)

---

## Estimated Complexity & Multi-PR Scope

| Feature | Estimated PRs | Complexity | Core Blocker |
|---------|---------------|-----------|--------------|
| Closures | 3-4 | High | Free variable analysis exists; needs syntax + values + typechecker |
| Complete enums | 5 | High | Payloads, matching, exhaustiveness (RES-400 PR2-5) |
| Function types | 2-3 | Medium | Type representation + application syntax |
| Generic functions | 3-4 | High | Parser + typechecker + monomorphization |
| Variadic functions | 2 | Medium | Parser + VM support |
| Macros | 4-5 | Very high | Need AST-at-compile-time + expansion framework |
| Module system | 4-5 | High | Parser + visibility + resolution + scoping |
| Operator overloading | 2-3 | Medium | Typechecker dispatch + trait integration |
| String interpolation | 2 | Low | Parser + runtime formatting |
| Error propagation (?) | 1-2 | Low | Syntactic sugar; desugar to match/unwrap |

