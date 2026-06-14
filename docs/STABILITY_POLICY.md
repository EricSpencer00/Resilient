# Resilient Stability and Compatibility Policy

## Overview

This document defines stability promises, compatibility guarantees, deprecation rules, and the expectations users can rely on as Resilient evolves. It answers: "When will my code break?"

---

## Core Principle

**Stability commitment:** Once a feature reaches the **Stable** tier, it will not break in minor or patch releases.

**Breaking changes** are announced at least **one major version in advance** and supported for at least **three minor releases** before removal.

---

## Semantic Versioning

Resilient follows semantic versioning (MAJOR.MINOR.PATCH):

### Patch Release (1.0.x → 1.0.y)

**When:** Bugfixes only, no new features

**Guarantee:** No breaking changes to any public API

**Example:** 1.0.5 → 1.0.6 is always safe to update

### Minor Release (1.x.0 → 1.y.0)

**When:** New features, incremental improvements

**Guarantee:** All Stable features from 1.x.0 remain compatible

**What may change:**
- New Stable features added
- Performance improvements
- Better error messages
- New warnings (not errors)

**Example:** 1.2.0 → 1.3.0 is safe for production code using 1.2.0

### Major Release (1.0.0 → 2.0.0)

**When:** Design improvements, architectural changes, removal of deprecated features

**Guarantee:** May contain breaking changes

**Required action:** Users must review changelog and update their code

**Example:** 1.9.9 → 2.0.0 requires testing and potential code updates

---

## Feature Tier System

### Stable

**Definition:** Fully specified, tested, documented, and ready for long-term production use.

**Stability guarantee:**
- Will not change in incompatible ways in minor versions
- Will be supported indefinitely (until explicitly deprecated)
- Breaking changes only in major versions

**Migration guarantee:** If removed, users get:
- At least 3 minor releases warning
- Clear deprecation message
- Migration guide
- Compiler support for old syntax (with warning)

**Example Stable features:**
- Basic types (int, float, bool, string, arrays)
- Function definitions and calls
- Control flow (if, while, for, match)
- Memory model (stack, static allocation)
- Pattern matching

### Backend-Limited

**Definition:** Fully implemented on some backends, not all.

**Stability guarantee:**
- Will not change on supported backends without notice
- Support status may change in minor versions

**Migration guarantee:** If support is dropped, users get:
- Compiler error on unsupported backends
- Clear diagnostic
- Documentation of migration path

**Example Backend-Limited features:**
- JIT-specific optimizations
- RISC-V interrupt handling
- Cortex-M MMIO annotations

### Experimental

**Definition:** Under design or implementation, may change frequently.

**Stability guarantee:** None. Breaking changes without notice.

**No migration guarantee:** May be removed or significantly changed at any time.

**Example Experimental features:**
- Async/await (future)
- Generic constraints (future)
- Effects system (future)

---

## Backward Compatibility Rules

### What's NOT a breaking change

The following are safe in minor releases:

✅ Adding new functions or methods  
✅ Adding new public types (as long as existing code doesn't break)  
✅ Making type checking stricter (previously accepted code may now error)  
✅ Adding compiler warnings (not errors)  
✅ Improving error messages  
✅ Performance improvements (as long as semantics unchanged)  
✅ Adding new compiler options/flags  
✅ Optimizing stdlib functions  

### What IS a breaking change

The following require a major version bump:

❌ Removing a public function, type, or constant  
❌ Changing function signature (parameter types, return type)  
❌ Changing type definition (adding non-optional fields, removing fields)  
❌ Changing behavior of a stable function  
❌ Changing error codes or diagnostic format (in a non-backward-compatible way)  
❌ Removing support for a stable backend  
❌ Changing memory layout of public types  

---

## Deprecation Process

### Step 1: Announce (Minor Release N)

Add deprecation warning to the feature:

```rust
/// Deprecated since 1.5.0; use `new_function` instead.
#[deprecated(since = "1.5.0", note = "use new_function")]
pub fn old_function() {
    // ...
}
```

**Action:**
- Release notes mention deprecation
- Compiler warns when code uses feature
- Error message suggests alternative

### Step 2: Support Period (Minor Releases N, N+1, N+2)

The old feature continues to work with warnings:

- **1.5.0:** Warning introduced
- **1.6.0:** Still works, still warns
- **1.7.0:** Still works, still warns
- **1.8.0:** Last chance! Final warning

### Step 3: Hard Error (Minor Release N+3)

Feature becomes hard error:

- **1.9.0:** Compiler rejects the feature
- Users must update their code

### Step 4: Removal (Later Major Release)

In version 2.0+, implementation code may be removed entirely.

---

## Language Reference Stability

### Stable Language Features

These will remain compatible:

- Core syntax (functions, variables, types)
- Arithmetic and logic operators
- Control flow (if, for, while, match)
- Memory safety rules
- Type system basics
- Pattern matching
- Error handling (Result, try)

### Subject to Change (with notice)

- Compiler error messages (format, codes)
- Exact diagnostics and their positions
- Stdlib API organization (re-exports may move)
- Optimization behavior (as long as semantics preserved)
- Backend implementation details

### Not Guaranteed

- Internal compiler structure
- Exact performance characteristics
- Timing of compilation passes
- Specific optimization order
- Internals of stdlib functions

---

## Stdlib Stability

### Tier 0 (Core): Stable

All Tier 0 (Core) functions are Stable:

```rust
pub fn add(int x, int y) -> int { ... }      // Stable
pub fn reverse<T>(array<T>) -> array<T> { ...} // Stable
```

**Guarantee:** Never changes, never removed

### Tier 1 (Alloc): Stable (feature-gated)

Tier 1 functions are Stable when the `alloc` feature is enabled:

```rust
#[cfg(feature = "alloc")]
pub fn allocate_vector(capacity: int) -> Vec<int> { ... } // Stable (when alloc enabled)
```

**Guarantee:** Doesn't change when feature is enabled

### Tier 2 (Std): Stable (host-only)

Tier 2 functions are Stable on host platforms:

```rust
#[cfg(feature = "std")]
pub fn read_file(path: string) -> Result<string, string> { ... } // Stable
```

**Guarantee:** Doesn't change on supported platforms

### Tier 3 (Platform): Backend-Limited

Platform-specific APIs may change:

```rust
#[cfg(target = "thumbv7em-none-eabihf")]
pub fn cortex_m_specific() { ... } // Backend-Limited
```

---

## API Evolution Patterns

### Pattern 1: Rename with Deprecation

```rust
// v1.4.0
#[deprecated(since = "1.4.0", note = "use compute_result")]
pub fn compute(x: int) -> int {
    return compute_result(x);  // delegate to new name
}

pub fn compute_result(x: int) -> int {
    // actual implementation
}

// v1.8.0+: remove old function entirely
```

### Pattern 2: Add Optional Parameter with Feature Gate

```rust
// v1.5.0: old signature
pub fn process(data: string) -> string {
    return process_internal(data, false);
}

// v1.6.0: new optional param (feature-gated)
#[cfg(feature = "strict_mode")]
pub fn process(data: string, strict: bool) -> string {
    return process_internal(data, strict);
}
```

### Pattern 3: Move Function to Module

```rust
// v1.4.0
pub fn old_location_compute() { ... }

// v1.5.0
pub mod computing {
    pub fn compute() { ... }
}

#[deprecated(since = "1.5.0", note = "use computing::compute")]
pub fn old_location_compute() {
    return computing::compute();
}

// v1.9.0: remove old location
```

---

## Tooling Stability

### Compiler (rz)

**Stable:**
- Exit codes (0 = success, 1+ = errors)
- Diagnostic format (file:line:col: level: message)
- Command-line options listed in `--help`

**Subject to change:**
- Exact error messages
- Internal compiler optimizations
- Specific diagnostic colors/formatting

### Package Manager (rz package)

**Stable:**
- Resilient.toml format
- Dependency resolution algorithm
- Lock file format
- Package naming conventions

**Subject to change:**
- CLI subcommand names (with deprecation)
- Config file locations
- Registry protocol details (with migration path)

### LSP Server

**Stable:**
- Basic completion support
- Hover information
- Go to definition

**Subject to change:**
- Exact ordering of completions
- Performance characteristics
- Internal diagnostic codes

---

## Security and Safety

### Security Policy

**Vulnerability reporting:** security@resilient-lang.org

**Supported versions:**
- Latest major version: all patches
- Previous major version: security patches only
- Older: no support

**Notification:** Security fixes announced in release notes

### Memory Safety

The memory safety guarantees (no use-after-free, no data races, etc.) are **not subject to change** except to become stricter.

If a bug is found in the safety checker, it will be:
1. Fixed immediately
2. May cause previously valid code to become invalid
3. Announced in release notes
4. Not considered a breaking change (safety is paramount)

---

## Breaking Change Examples

### Example 1: Function Removal

```
v1.5.0: fn read_sync() [deprecated, use read_async]
v1.6.0: fn read_sync() [still there, warns]
v1.7.0: fn read_sync() [still there, warns]
v1.8.0: fn read_sync() [still there, warns]
v1.9.0: Compilation error [fn read_sync() removed]
v2.0.0: Function gone forever
```

### Example 2: Type Change

```rust
// v1.5.0
pub fn get_value() -> Result<int, string> { ... }

// v1.8.0 [planned for 2.0]
// [deprecation period]

// v2.0.0 [breaking change]
pub fn get_value() -> Result<int, Error> { ... }  // Changed error type
```

### Example 3: Feature Removal

```toml
# v1.5.0
[features]
legacy_api = []  # Experimental API

# v1.8.0
# [removed without warning - was Experimental]

# v1.9.0
# Users must remove feature from Resilient.toml
```

---

## Upgrade Guidance

### From 1.x to 1.y (Same Major)

**Action:** Safe to upgrade automatically (or in CI)

```bash
rz update  # Safe: no breaking changes
```

### From 1.x to 2.0 (Major)

**Action:** Review changelog and test thoroughly

1. Read breaking changes section in release notes
2. Update code to use new APIs
3. Run full test suite
4. Deploy with confidence

---

## Version Pinning Strategy

### For Applications

Pin major version, allow minor/patch:

```toml
[dependencies]
mylib = "1.5"  # Allows 1.5.0, 1.6.0, but not 2.0.0
```

### For Libraries

Pin major version, be conservative with minor:

```toml
[dependencies]
foundation = "1.0"  # Stability critical
utils = "2"         # Less critical
```

### For Research/Experimental

Can use experimental features with understanding of risk:

```toml
[features]
experimental_async = []  # Will break in future
```

---

## Feedback and Discussions

### Proposing Changes

1. Open GitHub discussion for feedback
2. Collect community input
3. Document design decision
4. Implement with proper deprecation if breaking

### Reporting Incompatibilities

If you believe something breaks the stability policy:

1. Check release notes (might be intentional)
2. Open GitHub issue with details
3. Include version information
4. Provide minimal reproduction

---

## Timeline

### v0.x Series (Development)

- Breaking changes allowed
- Features may move between tiers
- No strict compatibility guarantees

### v1.0 (Stability Release)

- Feature set stabilized
- Compatibility policy kicks in fully
- Semver strictly followed

### v1.x Series (Stable)

- Compatibility guaranteed
- Breaking changes only in v2.0+
- Long-term production use recommended

### v2.0+ (Future)

- May include breaking changes
- Clear migration path provided
- Multiple deprecation cycle approach

---

## Summary

| Action | Patch | Minor | Major |
|--------|-------|-------|-------|
| New features | ❌ | ✅ | ✅ |
| Bug fixes | ✅ | ✅ | ✅ |
| Breaking changes | ❌ | ❌ | ✅ |
| Deprecate features | ❌ | ✅ | ✅ |
| Remove deprecated | ❌ | ❌ | ✅ |

---

## References

- **RES-3510:** Publish a stability and compatibility policy
- **RES-3501:** Stabilize language reference and feature-tier policy
- **LANGUAGE.md:** Feature tier definitions
- **MODULE_SYSTEM.md:** Package versioning

