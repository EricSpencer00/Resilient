---
title: Module and Package System
parent: Language Reference
nav_order: 8
permalink: /module-system
---

# Resilient Module and Package System

## v1 scope (this document's design decision)

RES-3502 asked for a real module and package system design, grounded in
what's actually implemented rather than a Cargo/crates.io lookalike. The v1
cut this document commits to is:

- **In scope, shipped:** inline `mod name { ... }` namespaces, file-based
  `use "path/to/file.rz";` splicing, a module dependency graph with cycle
  detection, three-level `pub`/`pub(crate)`/private visibility, a
  hand-rolled `resilient.toml` manifest (`name`, `version`,
  `[dependencies]`), a `resilient.lock` lockfile, and **local-path and
  git dependencies** (no central registry).
- **Explicitly out of scope for v1, tracked as follow-ups:** a central
  package registry / `rz publish` upload, workspaces (multi-package
  projects sharing one manifest), a `[features]` manifest section with
  dependency-level feature unification, and semver-range dependency
  resolution against a registry index (there is no registry to resolve
  against yet).

If you're deciding whether a design gap here is a bug or a v1 boundary:
if it involves fetching a package by name+semver-range from somewhere
other than a local path or a git URL, it's out of scope by design, not an
oversight.

---

## Modules

### Inline `mod` blocks (RES-324)

```resilient
mod arithmetic {
    fn add(int x, int y) -> int {
        return x + y;
    }
}

fn main() {
    let result = arithmetic::add(1, 2);
}
```

A `mod name { ... }` block groups declarations under a namespace prefix
*within a single file*. Every `fn`/`struct` declared inside is renamed to
`"name::item"` and registered directly in the environment — the parser
already collapses `arithmetic::add` into a flat identifier via the `::`
path token, so no separate cross-module symbol table is needed at this
tier. This is the lightweight form: no visibility enforcement of its own
(that comes from the full module system below), no separate file required.

### File-based imports: `use "path.rz";` (RES-073)

```resilient
use "sensors/thermal.rz";
use "sensors/thermal.rz" as thermal;   // RES-360: alias
```

`use "path/to/file.rz";` splices the target file's content into the
importing program **before typechecking** (`imports::expand_recursive`
walks and drains every top-level `Node::Use`, either splicing in the
resolved content or aborting compilation with a "could not be resolved"
diagnostic). By the time the typechecker runs, there are no `Use` nodes
left — this is closer to a C `#include` / textual expansion than a
separately-compiled-and-linked module, though the compiler tracks it well
enough to validate `use pkg::module` package-name references against known
package names at import time (RES-3838) — file-based (`use "path.rz"`) and
standard-library (`use std::*`) imports are exempt from that specific
existence check, since they're validated by simpler means (file exists /
stdlib symbol exists).

### Visibility and the module dependency graph (`full_modules.rs`, "Feature 40/50")

A second, independently-wired pass (`full_modules::check`, in the
typechecker's `<EXTENSION_PASSES>` block) extends the textual-splicing
model above with:

- A **visibility registry**: `pub` (public), `pub(crate)` (crate-visible),
  or private (the default, no modifier). `pub(super)`-style syntax is not
  a distinct visibility level in the checker today — only `pub` and
  `pub(crate)` are recognized; anything else falls back to private. Don't
  write `pub(super)` expecting parent-module-only visibility semantics
  yet.
- A **module dependency graph** built from `use` statements.
- A **cycle detector** over that graph — a circular `use` dependency is a
  compile-time error, not a runtime surprise.

---

## Packages

### Manifest: `resilient.toml`

```toml
[package]
name = "physics_sim"
version = "1.2.3"

[dependencies]
heapless = { path = "../libs/heapless" }
netutil = { git = "https://github.com/user/netutil", rev = "abc123" }
```

The manifest file is named **`resilient.toml`** (lowercase); `rz.toml` is
also accepted as an alternate filename. The parser (`package_manager.rs`)
is a small hand-rolled line-based reader — it understands `[package]`
(`name`, `version`) and `[dependencies]` sections. There is currently no
`[features]`, `authors`, `edition`, `license`, or `[dev-dependencies]`
section recognized by the manifest parser; don't add them expecting any
effect yet.

### Dependencies: path and git only

```toml
[dependencies]
mylib = { path = "../libs/mylib" }
netutil = { git = "https://github.com/user/netutil", rev = "abc123" }
```

`pkg_deps.rs`'s `DepSource` enum has exactly two variants: `Path` and
`Git` — there is no registry/version-index source. A path dependency is
validated to have its own `resilient.toml` and a `src/` directory before
it's accepted; a git dependency is cloned into
`~/.resilient/cache/git/<hash>/` and checked out at the given
`rev`/`tag`/`branch`. Plain semver-string dependencies (`serde = "1.0"`)
parse without erroring but have nothing to resolve against — there is no
registry to fetch a named+versioned package from yet, so don't rely on
that form actually pulling in code.

CLI: `rz pkg add <name> path:../libs/mylib` or
`rz pkg add <name> git:https://github.com/user/netutil --rev abc123`
appends the corresponding entry to `[dependencies]`.

### Lockfile: `resilient.lock`

```toml
[[package]]
name = "mylib"
source = "path:../libs/mylib"

[[package]]
name = "netutil"
source = "git:https://github.com/user/netutil"
rev = "abc123"
```

### Publishing (`rz pkg publish`, RES-342) — dry-run only

`rz pkg publish` reads the four manifest fields a registry would need
(`name`, `version`, `description`, `entry`), walks the project tree
(respecting a small subset of `.gitignore` patterns), and builds a
deterministic in-memory tarball with a hand-rolled tar writer. **There is
no registry endpoint configured yet** — `pkg publish` requires
`--dry-run` and prints a "registry endpoint not configured" error
otherwise. Deliberately deferred, per the module's own doc comment: the
actual HTTP upload, version-collision detection (nothing to check
collisions against without a registry), and source signing over the
published archive.

### Scaffolding: `rz pkg init`

```
rz pkg init my_project
```

Writes three files and refuses to clobber an existing manifest:
`resilient.toml` (with `[package]` and an empty `[dependencies]`),
`src/main.rz` (hello-world entry point), and `.gitignore`.

---

## Conditional compilation: `#[cfg(...)]`

```resilient
#[cfg(feature = "verbose")]
fn debug_log(string msg) { println(msg); }

#[cfg(not(feature = "verbose"))]
fn debug_log(string msg) { }

#[cfg(any(feature = "std", feature = "alloc"))]
fn needs_heap() { }
```

`#[cfg(key = "value")]` predicates (RES-2581, RES-2988, RES-343), with
`not`/`any`/`all` combinators, gate declarations at compile time based on
`--cfg key=value` (or bare `--cfg test`, which sets the built-in `test`
flag) CLI arguments. This is **independent of the package manifest** —
there is no `[features]` manifest section that declares named feature
sets or wires a `--cfg` value to a dependency's own feature flags; you
pass `--cfg` flags directly to `rz build`/`rz run`.

---

## std / no_std / alloc tiers

The runtime crate (`resilient-runtime`) is `#![no_std]` by default; heap
types gate behind `#[cfg(feature = "alloc")]`, and host-only capabilities
(file I/O, environment access) gate behind the crate's own Cargo
`std`/`alloc` features — see `STDLIB_PORTABILITY.md` for the authoritative
tier breakdown and which builtins are available on which target. This
document does not restate that tier table to avoid the two drifting.

---

## What v2+ would need to add

Tracked as follow-up scope, not implied by anything above:

- A central package registry and the `rz publish` HTTP upload path.
- Workspaces — today every `resilient.toml` describes exactly one
  package; there is no multi-package workspace manifest or member list.
- A `[features]` manifest section with cross-crate feature unification
  (today, `#[cfg(feature = "x")]` and `--cfg` are the only conditional
  compilation mechanism, and they don't consult the manifest).
- Semver-range dependency resolution against a registry index (only
  exact path/git sources resolve today).
- A distinct `pub(super)` visibility level in `full_modules.rs`.

---

## References

- **RES-3502:** Design a real module and package system (this doc).
- **RES-324:** inline `mod name { ... }` namespace blocks.
- **RES-073 / RES-360:** `use "path.rz";` file imports and `as` aliasing.
- **RES-3838:** package-name existence validation at import time.
- **"Feature 40/50" (`full_modules.rs`):** visibility registry + module
  dependency graph + cycle detection.
- **RES-205 / RES-212 / RES-342:** `pkg init`, `pkg add`, `pkg publish`.
- **FAILURE_MODEL.md:** error handling, independent of package boundaries.
- **STDLIB_PORTABILITY.md:** std/no_std/alloc tier table (authoritative).
