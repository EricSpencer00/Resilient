# Resilient Module and Package System

## Overview

Resilient's module system provides visibility control, dependency management, and code organization for both single-file programs and multi-package ecosystems. This design unifies embedded no_std packages with host-side tooling.

---

## Core Concepts

### Module

A **module** is a namespace that groups related code. Modules are organized in a tree structure.

```rust
// file: math.rz
pub mod arithmetic {
    pub fn add(int x, int y) -> int {
        return x + y;
    }
}

pub mod trigonometry {
    pub fn sine(float x) -> float {
        // implementation
    }
}
```

**Usage:**
```rust
use math::arithmetic;
fn main() {
    let result = arithmetic::add(1, 2);
}
```

### Package

A **package** is a self-contained unit of code with metadata, dependencies, and a publish identity.

**Package manifest:**
```yaml
# Resilient.toml
[package]
name = "math"
version = "0.2.0"
authors = ["you"]
edition = "2024"

[dependencies]
# no_std compatible dependencies
serde = { version = "1.0", features = ["no_std"] }

[dev-dependencies]
# testing only
tempfile = "3.0"

[features]
default = ["std"]
std = ["serde/std"]  # bring in std support
```

### Crate

A **crate** is the smallest unit that can be compiled. A crate is produced from:
- A single `.rz` file (binary or library crate)
- A directory with `lib.rz` or `main.rz` (library or binary package)

---

## Module System

### File-Based Modules

```
project/
├── lib.rz              # lib crate root
├── math.rz             # mod math
├── strings/
│   ├── format.rz       # mod strings::format
│   └── parse.rz        # mod strings::parse
└── tests/
    └── math_tests.rz   # integration tests
```

**lib.rz declares submodules:**
```rust
pub mod math;
pub mod strings;

#[cfg(test)]
mod tests;
```

**strings/format.rz declares its children:**
```rust
pub mod format {
    pub fn to_string(int x) -> string {
        // implementation
    }
}
```

### Visibility Rules

| Declaration | Visibility | Example |
|-------------|------------|---------|
| `pub` | Public (exported) | `pub fn add(...)` |
| `pub(crate)` | Visible in crate | `pub(crate) fn internal(...)` |
| `pub(super)` | Visible in parent module | `pub(super) fn helper(...)` |
| (no `pub`) | Private (module-only) | `fn private(...)` |

**Example:**
```rust
pub mod api {
    pub fn process(data: string) -> Result<int, string> {
        let validated = validate_internal(data)?;  // OK: pub(crate)
        return parse(validated);                   // OK: public
    }

    pub(crate) fn validate_internal(s: string) -> Result<string, string> {
        // visible in crate, not re-exported
    }

    pub fn parse(s: string) -> Result<int, string> {
        // public function
    }
}
```

---

## Package System

### Package Metadata (Resilient.toml)

```toml
[package]
name = "physics_sim"
version = "1.2.3"
authors = ["Alice <alice@example.com>", "Bob"]
edition = "2024"
description = "Physics simulation engine"
license = "MIT"
repository = "https://github.com/example/physics_sim"
documentation = "https://docs.example.com"

[profile.release]
opt-level = 3
lto = true

[dependencies]
# Semantic versioning
serde = "1.0"
nalgebra = "0.31"

# Minimum supported version
rand = ">= 0.8, < 0.9"

# No_std compatible
heapless = { version = "0.7", features = ["serde"] }

# Conditional dependencies
parking_lot = { version = "0.12", optional = true }

[dev-dependencies]
criterion = "0.4"  # benchmarking
proptest = "1.0"   # property testing

[features]
default = ["std", "threading"]
std = []
threading = ["parking_lot"]
```

### Semantic Versioning

Resilient follows semantic versioning:

| Version | Change | Implication |
|---------|--------|-------------|
| 1.0.0 → 1.0.1 | Patch (bugfix) | Safe to upgrade automatically |
| 1.0.0 → 1.1.0 | Minor (new API) | Safe to upgrade, new features available |
| 1.0.0 → 2.0.0 | Major (breaking) | Must review changes before upgrade |

**Stability guarantees:**
- `1.x.y` → `1.x.z` (same major.minor): No breaking changes to public API
- `1.x.y` → `1.y.z` (same major): New APIs added, old ones available
- `1.x.y` → `2.x.y` (new major): May have breaking changes

### Dependency Resolution

Resilient uses a **lock file** (`Resilient.lock`) for reproducible builds:

```
rz build                    # Resolves deps, creates/updates lock
rz update                   # Updates lock to latest compatible
rz update --aggressive      # Updates to latest (may be major)
```

**Lock file:**
```toml
[[package]]
name = "physics_sim"
version = "1.2.3"
source = "registry"

[[package]]
name = "nalgebra"
version = "0.31.4"
source = "registry"
dependencies = ["simba 0.7"]
```

---

## Import and Use

### Simple Import

```rust
use math::arithmetic::add;

fn main() {
    let x = add(1, 2);  // direct name
}
```

### Re-export

```rust
// math::arithmetic is re-exported as math::add
pub use arithmetic::add;

// Now users can do:
// use math::add;
```

### Glob Import

```rust
use math::*;  // all public items from math

fn main() {
    arithmetic::add(1, 2);      // OK
    trigonometry::sine(1.57);   // OK
}
```

### Selective Import

```rust
use math::{arithmetic, trigonometry};

fn main() {
    arithmetic::add(1, 2);
    trigonometry::sine(1.57);
}
```

---

## Package Distribution

### Publishing to Registry

```bash
rz publish --registry crates.io
```

**Registry requirements:**
- Valid `Resilient.toml` with name, version, description
- All dependencies must exist in registry
- License field required (or UNLICENSED)
- Source code must pass lint checks
- Tests must pass on all supported platforms

### Version Constraints

Users specify version constraints in dependencies:

```toml
[dependencies]
serde = "1.0"               # = 1.x.y (any 1.x version)
rand = "0.8.4"              # >= 0.8.4, < 0.9
nalgebra = ">= 0.30"        # >= 0.30, no upper bound
parking_lot = "~0.12"       # >= 0.12, < 0.13 (tilde)
```

---

## Feature Flags

### Conditional Compilation

```toml
[features]
default = ["std"]
std = []
threading = ["parking_lot"]
serialization = ["serde"]
```

**In code:**
```rust
#[cfg(feature = "std")]
pub mod file_io {
    pub fn read_file(path: string) -> Result<string, string> {
        // std-only implementation
    }
}

#[cfg(feature = "threading")]
pub mod async_compute {
    pub fn parallel_map<T, U>(
        items: &array<T>,
        f: (T) -> U,
    ) -> array<U> {
        // threading implementation
    }
}

#[cfg(not(feature = "std"))]
pub fn embedded_only() {
    // runs only in no_std builds
}
```

**Using features:**
```toml
[dependencies]
my_lib = { version = "1.0", features = ["std", "threading"] }
```

---

## std / no_std / alloc Tiers

### Tier 1: Core (always available)

```rust
// No imports needed, always available
fn pure_fn(int x) -> int {
    return x + 1;
}
```

**Available:** Basic types, pure functions, stack allocation, static allocation.

### Tier 2: Alloc (optional heap)

```rust
#[cfg(feature = "alloc")]
pub fn allocate_vec(capacity: int) -> array<int> {
    // heap allocation
}
```

**Available when:** User enables `alloc` feature.

### Tier 3: Std (host only)

```rust
#[cfg(feature = "std")]
pub fn read_env(key: string) -> Option<string> {
    // environment access (host only)
}
```

**Available when:** Building for host (Linux, Windows, macOS) with `std` feature.

### Package declaring its tiers:

```toml
[features]
default = ["std", "alloc"]
std = []
alloc = []
core = []  # core-only builds
```

---

## Workspaces

A **workspace** coordinates multiple related packages:

```
workspace/
├── Resilient.toml          # workspace manifest
├── packages/
│   ├── core/
│   │   ├── Resilient.toml
│   │   └── lib.rz
│   ├── simulation/
│   │   ├── Resilient.toml
│   │   └── lib.rz
│   └── cli/
│       ├── Resilient.toml
│       └── main.rz
```

**Workspace manifest:**
```toml
[workspace]
members = ["packages/core", "packages/simulation", "packages/cli"]

[workspace.dependencies]
# shared versions
serde = "1.0"
```

**Per-package manifest:**
```toml
[package]
name = "physics_core"
version = "1.0.0"

[dependencies]
# Can reference workspace members
physics_sim = { path = "../simulation" }
serde.workspace = true  # use workspace version
```

---

## Build System

### Build Process

```
rz build                         # debug build
rz build --release               # optimized
rz build --target thumbv7em-none-eabihf  # cross-compile
```

**Build steps:**
1. Parse `Resilient.toml` and resolve dependencies
2. Fetch dependencies from registry or workspace
3. Typecheck all code
4. Generate code for target backend
5. Link into final binary

### Conditional Compilation

```rust
#[cfg(target = "x86_64-unknown-linux-gnu")]
fn platform_specific() {
    eprintln!("running on Linux x86_64");
}

#[cfg(target = "thumbv7em-none-eabihf")]
fn embedded_specific() {
    // Cortex-M4F specific code
}
```

---

## Stability and Deprecation

### Marking APIs as Unstable

```rust
/// Unstable API; may change in next major version.
#[unstable(feature = "future_api", issue = "3502")]
pub fn experimental_feature() {
    // ...
}
```

### Deprecation

```rust
/// Deprecated since 1.5.0; use new_function instead.
#[deprecated(since = "1.5.0", note = "use new_function")]
pub fn old_function() {
    // ...
}
```

---

## Best Practices

### 1. Organize by Feature, Not by Layer

```
// Good
project/
├── auth/
│   ├── credentials.rz
│   └── token.rz
├── storage/
│   ├── cache.rz
│   └── persistence.rz

// Avoid
project/
├── core/
│   ├── auth.rz
│   ├── storage.rz
├── models/
│   ├── user.rz
```

### 2. Keep Modules Focused

**Good:**
```rust
pub mod encryption {
    pub fn encrypt(data: string, key: string) -> Result<string, string> { ... }
    pub fn decrypt(encrypted: string, key: string) -> Result<string, string> { ... }
}
```

**Avoid:**
```rust
pub mod utils {
    pub fn everything_else() { ... }
}
```

### 3. Minimize Public Surface

```rust
// Good: only expose what users need
pub fn process_data(input: string) -> Result<Output, Error> { ... }
pub(crate) fn internal_helper() { ... }

// Avoid: exposing all internals
pub fn _internal_step_1() { ... }
pub fn _internal_step_2() { ... }
```

### 4. Document Module Purpose

```rust
//! Authentication module.
//!
//! Handles user credentials, token validation, and session management.
//! Supports both password-based and token-based authentication.

pub mod credentials;
pub mod tokens;
```

### 5. Version Carefully

- **Patch releases** (1.0.0 → 1.0.1): Only for bugfixes, no API changes
- **Minor releases** (1.0.0 → 1.1.0): New public APIs, backward compatible
- **Major releases** (1.0.0 → 2.0.0): May have breaking changes

---

## Roadmap

### v0.3: Module System Stabilization
- [x] File-based modules
- [x] Visibility rules (pub, pub(crate), pub(super))
- [ ] Module path resolution in compiler
- [ ] Module documentation generation

### v0.4: Package System Foundation
- [ ] Resilient.toml parsing and validation
- [ ] Dependency resolution algorithm
- [ ] Lock file generation and reproducibility
- [ ] Package name registry

### v0.5: Package Distribution
- [ ] Central package registry (crates.io equivalent)
- [ ] Publishing workflow (`rz publish`)
- [ ] Dependency version management
- [ ] Security audit support

### v0.6+: Advanced Features
- [ ] Workspaces with interdependencies
- [ ] Feature flag refinement
- [ ] Build profiles (release, debug, embedded)
- [ ] Conditional compilation across std/no_std/alloc

---

## Example: Complete Package

```toml
# Resilient.toml
[package]
name = "embedded_crypto"
version = "0.3.0"
authors = ["Security Team"]
edition = "2024"
description = "Cryptographic primitives for embedded systems"

[features]
default = ["aes"]
aes = []
sha256 = []

[dependencies]
heapless = "0.7"

[dev-dependencies]
proptest = "1.0"
```

```rust
// lib.rz
//! Embedded cryptography library
//! 
//! Provides AES, SHA256, and HMAC with no_std support.

pub mod aes;

#[cfg(feature = "sha256")]
pub mod sha256;

pub mod hmac;
```

```rust
// aes.rz
pub mod aes {
    const BLOCK_SIZE: int = 16;
    
    pub fn encrypt(plaintext: &array<byte>, key: &array<byte>) -> Result<array<byte>, string> {
        // AES-128 encryption
    }
    
    pub fn decrypt(ciphertext: &array<byte>, key: &array<byte>) -> Result<array<byte>, string> {
        // AES-128 decryption
    }
}
```

---

## References

- **RES-3502:** Design a real module and package system
- **RES-3507:** Design a production-grade standard library portability model
- **FAILURE_MODEL.md:** Error handling across packages
- **MEMORY_MODEL.md:** Allocation tiers for no_std/alloc/std

