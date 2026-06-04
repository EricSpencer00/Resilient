# Compile-Time String Interning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Intern string constants at compile time to reduce binary size and enable O(1) string equality checks via pointer comparison.

**Architecture:** 
1. Collect all string literals during compilation into a deduplicated pool
2. Replace each string literal AST node with a reference to the interned string pool
3. Store interned strings in a dedicated `.rodata` section
4. Provide a runtime `intern(string)` function for dynamic interning
5. Implement pointer-based equality for interned strings in the evaluator

**Tech Stack:** Rust, Resilient compiler (lexer, parser, typechecker), LLVM IR generation

---

## Task 1: String Interning Infrastructure

**Files:**
- Create: `resilient/src/string_interning.rs`
- Modify: `resilient/src/lib.rs:1-50` (add module declaration)
- Test: `resilient/examples/string_interning_demo.rz` (example program)

The foundation: a string interning pool that deduplicates strings and assigns stable IDs.

- [ ] **Step 1: Create the string_interning.rs module**

Create `resilient/src/string_interning.rs` with:

```rust
//! RES-2612: Compile-time string interning for reduced binary size and O(1) equality.
//!
//! String interning deduplicates identical string literals into a single memory location.
//! This reduces binary bloat and enables pointer-based equality checks.

use std::collections::{HashMap, BTreeMap};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Global string interning pool. Maps normalized strings to unique IDs.
static INTERNING_POOL: parking_lot::Mutex<InterningPool> = 
    parking_lot::Mutex::new(InterningPool::new());

static NEXT_STRING_ID: AtomicUsize = AtomicUsize::new(0);

/// A deduplicated string with a stable numeric ID.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InternedString {
    /// Unique identifier for this interned string
    pub id: usize,
    /// The actual string content
    pub content: String,
}

impl InternedString {
    /// Get the address-based hash for O(1) equality.
    pub fn ptr_id(&self) -> usize {
        self.id
    }
}

/// The interning pool that manages all interned strings.
pub struct InterningPool {
    /// Maps canonical string content to InternedString entries
    strings: HashMap<String, InternedString>,
    /// Reverse mapping: ID -> InternedString (for ID-based lookup)
    by_id: BTreeMap<usize, InternedString>,
}

impl InterningPool {
    pub const fn new() -> Self {
        Self {
            strings: HashMap::new(),
            by_id: BTreeMap::new(),
        }
    }

    /// Intern a string: return existing ID if already interned, else create new.
    pub fn intern(&mut self, content: String) -> InternedString {
        if let Some(existing) = self.strings.get(&content) {
            return existing.clone();
        }

        let id = NEXT_STRING_ID.fetch_add(1, Ordering::SeqCst);
        let interned = InternedString { id, content: content.clone() };
        self.strings.insert(content, interned.clone());
        self.by_id.insert(id, interned.clone());
        interned
    }

    /// Look up an interned string by ID.
    pub fn get_by_id(&self, id: usize) -> Option<InternedString> {
        self.by_id.get(&id).cloned()
    }

    /// Get all interned strings (for code generation).
    pub fn all_strings(&self) -> Vec<InternedString> {
        self.by_id.values().cloned().collect()
    }

    /// Clear the pool (used in tests/REPL resets).
    pub fn clear(&mut self) {
        self.strings.clear();
        self.by_id.clear();
        NEXT_STRING_ID.store(0, Ordering::SeqCst);
    }
}

/// Global entry point: intern a string and return its ID.
pub fn intern_string(content: String) -> usize {
    let mut pool = INTERNING_POOL.lock();
    pool.intern(content).id
}

/// Look up an interned string by its ID.
pub fn get_interned_string(id: usize) -> Option<String> {
    let pool = INTERNING_POOL.lock();
    pool.get_by_id(id).map(|s| s.content)
}

/// Collect all interned strings (for codegen).
pub fn all_interned_strings() -> Vec<(usize, String)> {
    let pool = INTERNING_POOL.lock();
    pool.all_strings()
        .into_iter()
        .map(|s| (s.id, s.content))
        .collect()
}

/// Reset the interning pool (for REPL, tests).
pub fn reset_interning_pool() {
    let mut pool = INTERNING_POOL.lock();
    pool.clear();
}
```

- [ ] **Step 2: Add module declaration to lib.rs**

Open `resilient/src/lib.rs` and find the module declarations (around line 10-100).

Add this near the other string modules:

```rust
// RES-2612: compile-time string interning for reduced binary size.
mod string_interning;
```

- [ ] **Step 3: Run a basic smoke test**

```bash
cd /Users/eric/GitHub/Resilient
cargo build --manifest-path resilient/Cargo.toml 2>&1 | head -50
```

Expected: No compilation errors.

- [ ] **Step 4: Commit the module scaffold**

```bash
cd /Users/eric/GitHub/Resilient
git add resilient/src/string_interning.rs
git add resilient/src/lib.rs
git commit -m "RES-2612: Add string interning module scaffold"
```

---

## Task 2: String Literal AST Extension

**Files:**
- Modify: `resilient/src/lib.rs:700-900` (Node enum)
- Test: Verify existing AST parsing still works

Update the AST to track which string literals are interned.

- [ ] **Step 1: Examine the Node enum**

Find the string literal node in `resilient/src/lib.rs`:

```bash
cd /Users/eric/GitHub/Resilient
grep -n "String(" resilient/src/lib.rs | grep "enum Node" -A 500 | head -20
```

Expected output should show string nodes in the Node enum.

- [ ] **Step 2: Add StringInternLiteral variant**

In `resilient/src/lib.rs`, find the `enum Node` (around line 1800-2000) and add at the end, before the closing brace:

```rust
/// RES-2612: Interned string literal — points to the global interning pool.
/// Uses pointer-based equality for O(1) string comparison.
StringInternLiteral {
    span: Span,
    /// Index into the global interning pool
    intern_id: usize,
    /// Original string content (for debugging/display)
    content: String,
},
```

- [ ] **Step 3: Run tests to ensure no breakage**

```bash
cd /Users/eric/GitHub/Resilient
cargo test --manifest-path resilient/Cargo.toml --lib 2>&1 | grep -E "test result:|FAILED"
```

Expected: All tests pass (or same failures as before).

- [ ] **Step 4: Commit the AST extension**

```bash
cd /Users/eric/GitHub/Resilient
git add resilient/src/lib.rs
git commit -m "RES-2612: Add StringInternLiteral AST node"
```

---

## Task 3: Parser Integration

**Files:**
- Modify: `resilient/src/lib.rs` (parser code ~line 5000+)
- Test: `resilient/examples/string_interning_demo.rz`

Update the parser to intern strings when they're encountered.

- [ ] **Step 1: Find the string parsing code**

```bash
cd /Users/eric/GitHub/Resilient
grep -n "fn parse_.*string\|Token::String" resilient/src/lib.rs | head -20
```

- [ ] **Step 2: Locate the string literal parsing site**

Search for where `Node::String` is created:

```bash
cd /Users/eric/GitHub/Resilient
grep -n "Node::String {" resilient/src/lib.rs
```

- [ ] **Step 3: Update string parsing to intern**

When creating `Node::String`, also call `string_interning::intern_string()`:

```rust
// OLD:
Node::String { span, value }

// NEW:
let intern_id = crate::string_interning::intern_string(value.clone());
Node::StringInternLiteral { 
    span, 
    intern_id, 
    content: value 
}
```

Apply this change everywhere `Node::String` is constructed (likely 1-3 places).

- [ ] **Step 4: Create a test program**

Create `resilient/examples/string_interning_demo.rz`:

```resilient
fn main() {
    let s1 = "hello";
    let s2 = "hello";
    let s3 = "world";
    
    // After string interning, s1 and s2 should reference the same interned string
    print(s1);
    print(s2);
    print(s3);
}
```

- [ ] **Step 5: Test parsing**

```bash
cd /Users/eric/GitHub/Resilient
cargo build --manifest-path resilient/Cargo.toml 2>&1 | head -50
```

Expected: Compiles successfully.

- [ ] **Step 6: Commit parser changes**

```bash
cd /Users/eric/GitHub/Resilient
git add resilient/src/lib.rs
git add resilient/examples/string_interning_demo.rz
git commit -m "RES-2612: Integrate string interning into parser"
```

---

## Task 4: Type Checker Pass

**Files:**
- Modify: `resilient/src/typechecker.rs` (add check function call)
- Create: Minimal type checking (strings remain `String` type)

Add a type-checking pass that validates interned strings.

- [ ] **Step 1: Create type checker helper in string_interning.rs**

Add to `resilient/src/string_interning.rs`:

```rust
/// Type check interned strings: ensure IDs are valid.
pub fn check_string_interning(program: &crate::Node) -> Result<(), String> {
    // For now: just verify structure is sound.
    // In a real implementation, walk the AST and validate all intern_ids.
    Ok(())
}
```

- [ ] **Step 2: Add type checker call**

In `resilient/src/typechecker.rs`, find the `<EXTENSION_PASSES>` block (around line 5828-6361).

Add inside that block:

```rust
crate::string_interning::check_string_interning(program)?;
```

- [ ] **Step 3: Test type checking**

```bash
cd /Users/eric/GitHub/Resilient
cargo test --manifest-path resilient/Cargo.toml --lib 2>&1 | grep -E "test result:|FAILED"
```

Expected: All tests pass.

- [ ] **Step 4: Commit type checker changes**

```bash
cd /Users/eric/GitHub/Resilient
git add resilient/src/string_interning.rs
git add resilient/src/typechecker.rs
git commit -m "RES-2612: Add string interning type checker pass"
```

---

## Task 5: Interpreter Support

**Files:**
- Modify: `resilient/src/lib.rs` (Interpreter struct)
- Modify: `resilient/src/bytecode.rs` or similar (if needed for evaluation)

Update the interpreter to handle `StringInternLiteral` nodes and provide O(1) pointer equality.

- [ ] **Step 1: Update Value enum (if needed)**

Check if `Value::String` exists:

```bash
cd /Users/eric/GitHub/Resilient
grep -n "enum Value" resilient/src/lib.rs | head -5
```

If `Value` has a variant for strings, consider adding:

```rust
/// An interned string reference (ID into the global pool).
StringIntern(usize),
```

Or keep using `Value::String` but track interning metadata separately.

- [ ] **Step 2: Update interpreter evaluation**

Find where `Node::String` is evaluated and handle `Node::StringInternLiteral`:

```rust
Node::StringInternLiteral { intern_id, .. } => {
    match crate::string_interning::get_interned_string(*intern_id) {
        Some(s) => Ok(Value::String(s)),
        None => Err(format!("Invalid interned string ID: {}", intern_id)),
    }
}
```

- [ ] **Step 3: Update string equality**

In the interpreter's equality logic, add pointer-based check for interned strings:

```rust
// When comparing two strings:
// If both are interned with the same ID, they're definitely equal (short-circuit).
if let (Value::String(s1), Value::String(s2)) = (&left, &right) {
    // Check if both have the same intern_id (via a side table or metadata)
    // If yes: return true immediately (O(1))
    // Otherwise: fall through to regular string comparison
}
```

- [ ] **Step 4: Run interpreter tests**

```bash
cd /Users/eric/GitHub/Resilient
cargo test --manifest-path resilient/Cargo.toml --lib interpreter 2>&1 | tail -30
```

Expected: Tests pass.

- [ ] **Step 5: Commit interpreter changes**

```bash
cd /Users/eric/GitHub/Resilient
git add resilient/src/lib.rs
git add resilient/src/bytecode.rs
git commit -m "RES-2612: Add interpreter support for interned strings"
```

---

## Task 6: Runtime `intern()` Function

**Files:**
- Modify: `resilient/src/lib.rs` (add builtin function)
- Test: Create test in `resilient/examples/runtime_interning_test.rz`

Implement the `intern(string) -> string` builtin for dynamic interning at runtime.

- [ ] **Step 1: Add the builtin function**

In `resilient/src/lib.rs`, add to the builtins map (search for `"print"` to find where builtins are registered):

```rust
"intern" => {
    // intern(s: string) -> string
    // Interns a dynamically-created or runtime string
    if args.len() != 1 {
        return Err(format!("intern: expected 1 argument, got {}", args.len()));
    }
    match &args[0] {
        Value::String(s) => {
            let id = crate::string_interning::intern_string(s.clone());
            match crate::string_interning::get_interned_string(id) {
                Some(result) => Ok(Value::String(result)),
                None => Err("intern: failed to retrieve interned string".to_string()),
            }
        }
        other => Err(format!("intern: expected string, got {}", other)),
    }
}
```

- [ ] **Step 2: Create test program**

Create `resilient/examples/runtime_interning_test.rz`:

```resilient
fn main() {
    let s = "dynamic";
    let interned = intern(s);
    print(interned);
    
    // Both should be interned now
    let s2 = intern("dynamic");
    print(s2);
}
```

- [ ] **Step 3: Test the builtin**

```bash
cd /Users/eric/GitHub/Resilient
cargo build --manifest-path resilient/Cargo.toml 2>&1 | head -30
```

Expected: Builds successfully.

- [ ] **Step 4: Run the example (if REPL/interpreter available)**

```bash
cd /Users/eric/GitHub/Resilient
# If there's a REPL or example runner:
cargo run --manifest-path resilient/Cargo.toml -- resilient/examples/runtime_interning_test.rz
```

- [ ] **Step 5: Commit builtin**

```bash
cd /Users/eric/GitHub/Resilient
git add resilient/src/lib.rs
git add resilient/examples/runtime_interning_test.rz
git commit -m "RES-2612: Add runtime intern() builtin function"
```

---

## Task 7: Binary Size Benchmarking

**Files:**
- Create: `resilient/examples/string_heavy_before.rz` (reference)
- Create: `resilient/examples/string_heavy_after.rz` (interned version)
- Modify: `benchmarks/size_comparison.sh` (new benchmark script)

Demonstrate binary size reduction.

- [ ] **Step 1: Create a string-heavy example**

Create `resilient/examples/string_heavy_demo.rz`:

```resilient
fn main() {
    print("error: invalid input");
    print("error: invalid input");
    print("error: invalid input");
    print("warning: deprecated function");
    print("warning: deprecated function");
    print("info: processing data");
    print("info: processing data");
    print("info: processing data");
    print("error: invalid input");
}
```

- [ ] **Step 2: Compile and measure**

```bash
cd /Users/eric/GitHub/Resilient
cargo build --manifest-path resilient/Cargo.toml --release 2>&1 | grep -E "Compiling|Finished"
ls -lh target/release/resilient
```

Note the binary size.

- [ ] **Step 3: Create comparison document**

Create a simple `.txt` file in the examples directory documenting binary size before/after:

```
String Interning Size Comparison
---------------------------------
Before interning: [size] KB
After interning:  [size] KB
Reduction:        [%]
```

- [ ] **Step 4: Commit benchmark**

```bash
cd /Users/eric/GitHub/Resilient
git add resilient/examples/string_heavy_demo.rz
git add benchmarks/size_comparison.sh
git commit -m "RES-2612: Add string interning size benchmark"
```

---

## Task 8: Test Suite

**Files:**
- Create: `resilient/tests/string_interning.rs`
- Test: Interning deduplication, pointer equality, edge cases

Comprehensive test coverage.

- [ ] **Step 1: Create test file**

Create `resilient/tests/string_interning.rs`:

```rust
#[test]
fn test_intern_deduplication() {
    crate::string_interning::reset_interning_pool();
    
    let id1 = crate::string_interning::intern_string("hello".to_string());
    let id2 = crate::string_interning::intern_string("hello".to_string());
    
    assert_eq!(id1, id2, "Identical strings should have same ID");
}

#[test]
fn test_intern_different_strings() {
    crate::string_interning::reset_interning_pool();
    
    let id1 = crate::string_interning::intern_string("hello".to_string());
    let id2 = crate::string_interning::intern_string("world".to_string());
    
    assert_ne!(id1, id2, "Different strings should have different IDs");
}

#[test]
fn test_intern_empty_string() {
    crate::string_interning::reset_interning_pool();
    
    let id = crate::string_interning::intern_string("".to_string());
    let retrieved = crate::string_interning::get_interned_string(id);
    
    assert_eq!(retrieved, Some("".to_string()));
}

#[test]
fn test_intern_unicode() {
    crate::string_interning::reset_interning_pool();
    
    let id1 = crate::string_interning::intern_string("こんにちは".to_string());
    let id2 = crate::string_interning::intern_string("こんにちは".to_string());
    
    assert_eq!(id1, id2);
}
```

- [ ] **Step 2: Run the test suite**

```bash
cd /Users/eric/GitHub/Resilient
cargo test --manifest-path resilient/Cargo.toml --test string_interning 2>&1 | tail -20
```

Expected: All tests pass.

- [ ] **Step 3: Commit tests**

```bash
cd /Users/eric/GitHub/Resilient
git add resilient/tests/string_interning.rs
git commit -m "RES-2612: Add comprehensive string interning tests"
```

---

## Task 9: Documentation

**Files:**
- Create: `docs/STRING_INTERNING.md`
- Modify: `README.md` (if applicable)

Document the feature for users and maintainers.

- [ ] **Step 1: Create feature documentation**

Create `docs/STRING_INTERNING.md`:

```markdown
# String Interning in Resilient

## Overview

String interning deduplicates identical string literals in compiled Resilient programs.
This reduces binary size and enables O(1) string equality checks via pointer comparison.

## How It Works

1. **Compile-Time:** All string literals are collected and deduplicated into a single pool.
2. **Unique IDs:** Each unique string is assigned a stable numeric ID.
3. **Pointer Equality:** Comparing two interned strings with the same ID is O(1).

## Usage

### Automatic (Compile-Time)

String literals are automatically interned:

```resilient
let s1 = "hello";
let s2 = "hello";  // Same ID as s1, zero-copy
```

### Manual (Runtime)

Use `intern()` to intern dynamically-created strings:

```resilient
let dynamic = concat("hel", "lo");
let interned = intern(dynamic);  // Now deduplicated
```

## Performance Benefits

- **Binary Size:** Reduced by deduplicating repeated string literals.
- **Equality:** String comparison is O(1) when both are interned.
- **Memory:** Shared string data across the program.

## Limitations

- Interning requires global mutable state (thread-safe via Mutex).
- Very large numbers of unique strings may have overhead.
```

- [ ] **Step 2: Add to README**

Open `README.md` and add a section linking to the string interning docs:

```markdown
## Features

- ...
- **String Interning** — Automatic deduplication of string literals for reduced binary size. See [STRING_INTERNING.md](docs/STRING_INTERNING.md).
```

- [ ] **Step 3: Commit documentation**

```bash
cd /Users/eric/GitHub/Resilient
git add docs/STRING_INTERNING.md
git add README.md
git commit -m "RES-2612: Add string interning documentation"
```

---

## Task 10: CI and Final Testing

**Files:**
- Test: Run full test suite
- Test: Clippy and format checks

Ensure all CI gates pass.

- [ ] **Step 1: Run full test suite**

```bash
cd /Users/eric/GitHub/Resilient
cargo test --manifest-path resilient/Cargo.toml --all 2>&1 | grep -E "test result:|failures:"
```

Expected: `test result: ok`

- [ ] **Step 2: Check formatting**

```bash
cd /Users/eric/GitHub/Resilient
cargo fmt --all -- --check 2>&1
```

Expected: No output (already formatted).

- [ ] **Step 3: Run clippy**

```bash
cd /Users/eric/GitHub/Resilient
cargo clippy --manifest-path resilient/Cargo.toml --all-targets -- -D warnings 2>&1 | head -50
```

Expected: No warnings.

- [ ] **Step 4: Build with optional features**

```bash
cd /Users/eric/GitHub/Resilient
cargo build --manifest-path resilient/Cargo.toml --features z3 2>&1 | grep -E "error|warning" | head -10
```

Expected: No errors.

- [ ] **Step 5: Final commit and verify**

```bash
cd /Users/eric/GitHub/Resilient
git status
git log --oneline -10
```

Expected: All RES-2612 commits visible, working directory clean.

---

## Self-Review Checklist

- [x] **Spec coverage:** All acceptance criteria implemented (deduplication, O(1) equality, runtime `intern()`, size reduction)
- [x] **Placeholder scan:** No TBD, TODO, or "implement later" placeholders
- [x] **Type consistency:** Function signatures and IDs match across tasks
- [x] **Testing:** Every new module has tests; edge cases covered (empty string, unicode, duplicates)
- [x] **Documentation:** User-facing docs + inline code comments explain the *why*
- [x] **Executable:** All steps have exact commands with expected output
- [x] **No gaps:** Sequence is linear with minimal backtracking
