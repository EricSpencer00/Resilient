---
title: Tooling Quality Standards
parent: Language Reference
nav_order: 6
permalink: /tooling-quality
---

# Resilient Tooling Quality Standards

## Overview

This document defines the minimum viable quality bar for Resilient developer tools: formatter, LSP, debugger, package workflows, and runtime utilities. These standards ensure the language ecosystem is reliable and productive for users.

---

## Core Principle

**A language is not just a compiler.** Users adopt Resilient based on the entire developer experience. Tools must meet a minimum quality threshold to avoid frustrating users and undermining confidence in the language.

---

## Formatter (`rz fmt`)

### Stability Guarantee: Stable

The formatter produces deterministic, consistent output that all developers can rely on.

### Minimum Standards

| Aspect | Standard | Rationale |
|--------|----------|-----------|
| **Determinism** | Same input always produces same output | Enables diffing, merging, CI validation |
| **Roundtrip Safety** | `fmt` followed by `fmt` produces same output | Avoid formatting loops in editors |
| **Syntax Preservation** | Never changes valid code's meaning | Users trust formatter with codebase |
| **Performance** | Format 10K LOC in < 1 second | Usable in pre-commit hooks |
| **Idempotence** | Multiple passes are no-op | Reliable for automated workflows |
| **Configuration** | Minimal / no config (strongly opinionated) | Reduces team friction |

### Acceptance Tests

```rust
#[test]
fn fmt_idempotent() {
    let code = r#"fn  main(  ) { let x=1; }"#;
    let formatted_once = fmt(code);
    let formatted_twice = fmt(&formatted_once);
    assert_eq!(formatted_once, formatted_twice);
}

#[test]
fn fmt_preserves_semantics() {
    let original = parse(code).unwrap();
    let formatted = fmt(code);
    let reparsed = parse(&formatted).unwrap();
    assert_eq!(original, reparsed);
}

#[test]
fn fmt_performance_10k_loc() {
    let large_code = generate_code(10000);
    let start = time::now();
    fmt(&large_code);
    assert!(time::elapsed() < Duration::from_secs(1));
}
```

### Configuration

**Recommended:** No configuration file. Use sensible defaults:

```rust
// Built-in defaults
const INDENT_WIDTH: usize = 4;
const LINE_LENGTH: usize = 100;
const TRAILING_COMMA: bool = true;
const FUNCTION_SPACING: usize = 1;  // blank line between fns
```

---

## Language Server (LSP)

### Stability Guarantee: Backend-Limited (Host Only)

LSP supports development workflows on host systems (Linux, macOS, Windows).

### Minimum Standards

| Feature | Standard | Notes |
|---------|----------|-------|
| **Completion** | ≥ 80% accuracy on common identifiers | Suggest functions, variables, types |
| **Hover** | Type information + documentation | Quick type checking without build |
| **Go-to-Definition** | Jump to function/type declaration | Essential for navigation |
| **Diagnostics** | Show parse + type errors in editor | Real-time feedback |
| **Rename** | Refactor identifier across file | Safety for large changes |
| **References** | Find all usages of a symbol | Code comprehension |
| **Performance** | Respond to edits within 500ms | Responsive editor experience |

### Acceptance Criteria

```rust
#[test]
fn lsp_completion_offers_in_scope_vars() {
    let editor = LSPServer::start("test.rz");
    editor.edit("let my_var = 5; let x = my");
    let completions = editor.complete_at_cursor();
    assert!(completions.contains("my_var"));
}

#[test]
fn lsp_hover_shows_type() {
    let editor = LSPServer::start("test.rz");
    editor.edit("fn add(int x, int y) -> int { return x + y; } add(");
    let hover = editor.hover_at_cursor();
    assert!(hover.contains("int"));
}

#[test]
fn lsp_diagnostics_within_500ms() {
    let editor = LSPServer::start("test.rz");
    let start = time::now();
    editor.edit("fn f(unknown_type x) {}");
    let diags = editor.get_diagnostics();
    assert!(time::elapsed() < Duration::from_millis(500));
}
```

### Stability Timeline

- **v0.3:** Parse diagnostics only
- **v0.4:** Type errors + completion
- **v0.5:** Rename + references (Stable)
- **v0.6+:** Advanced refactoring

---

## Debugger (future)

### Planned Stability: Stable (v0.5+)

Debugger will provide breakpoints, stepping, and variable inspection for development builds.

### Minimum Standards (When Shipped)

| Feature | Standard |
|---------|----------|
| **Breakpoints** | Set breakpoints by line, stop on hit |
| **Stepping** | Step in/over/out through code |
| **Variables** | Inspect local and global scope |
| **Expressions** | Evaluate expressions in context |
| **Stack Frames** | View call stack with file:line |
| **Performance** | Debug overhead < 10% for normal programs |

### Integration Points

- Compiler generates DWARF debug info (or equivalent)
- Runtime exposes debugging APIs
- CLI integrates with GDB / LLDB / custom debugger

---

## Package Manager (`rz package`)

### Stability Guarantee: Stable (v0.4+)

Package management is critical for ecosystem adoption.

### Minimum Standards

| Operation | Standard | Notes |
|-----------|----------|-------|
| **Install** | Deterministic dependency resolution | Lock file guarantees reproducibility |
| **Add** | `rz add <name>` resolves and installs | Clear error messages on conflicts |
| **Update** | `rz update` respects version constraints | Semantic versioning honored |
| **Publish** | `rz publish` uploads to registry | Handles authentication securely |
| **Search** | Query registry by name/tag | Discover community packages |
| **Verify** | Check integrity of downloaded packages | Detect tampering or corruption |

### Acceptance Tests

```rust
#[test]
fn pkg_install_deterministic() {
    let lock1 = run("rz add serde");
    let lock2 = run("rz add serde");
    assert_eq!(lock1, lock2);
}

#[test]
fn pkg_version_constraints_respected() {
    let manifest = toml::parse(r#"
        [dependencies]
        serde = "1.0"
    "#);
    let lock = resolve_deps(&manifest);
    let version = lock.get_version("serde");
    assert!(version.starts_with("1."));
}

#[test]
fn pkg_publish_requires_auth() {
    let result = run_cmd("rz publish");
    // Should fail without credentials
    assert!(result.is_err());
}
```

### CLI Stability

Once shipped, these commands remain compatible:

```bash
rz add <package>               # Add dependency
rz remove <package>            # Remove dependency
rz update [package]            # Update lock file
rz publish                      # Publish package
rz search <query>              # Search registry
```

Breaking changes require major version bump.

---

## Build System

### Minimum Standards

| Aspect | Standard |
|--------|----------|
| **Build time** | Typical project < 5 seconds |
| **Incremental** | Change one file, recompile only that |
| **Parallelism** | Use all CPU cores for speed |
| **Caching** | Avoid redundant compilation |
| **Cross-compile** | `rz build --target <triple>` works |

### Acceptance Criteria

```rust
#[test]
fn build_typical_project_under_5sec() {
    let project = create_typical_project();  // ~1000 LOC
    let start = time::now();
    run("rz build");
    assert!(time::elapsed() < Duration::from_secs(5));
}

#[test]
fn build_incremental() {
    let project = create_project();
    run("rz build");  // Full build
    let before = time::now();
    modify_one_file(&project);
    run("rz build");  // Incremental build
    let incremental_time = time::elapsed();
    assert!(incremental_time < Duration::from_millis(500));
}
```

---

## REPL (Interactive)

### Stability Guarantee: Backend-Limited

REPL evaluates code snippets interactively for quick experimentation.

### Minimum Standards

| Feature | Standard |
|---------|----------|
| **Execution** | Evaluate expressions and statements |
| **Type Check** | Show type of evaluated expression |
| **History** | Access previous commands |
| **Editing** | Multi-line input support |
| **Performance** | Execute simple code within 100ms |

### Example Workflow

```
$ rz repl
> let x = 5;
> let y = x + 10;
> print(y);
15
> :quit
```

---

## Documentation Generation

### Minimum Standards

| Aspect | Standard |
|--------|----------|
| **Extraction** | Auto-generate docs from comments |
| **HTML** | Publish static HTML docs |
| **Search** | Enable full-text search in docs |
| **Links** | Cross-reference types/functions |
| **Examples** | Include code examples from comments |

### Example

```rust
/// Computes the sum of two integers.
///
/// # Examples
/// ```
/// assert_eq!(add(2, 3), 5);
/// ```
pub fn add(int x, int y) -> int {
    return x + y;
}
```

Generates:

```html
<h1>add(x: int, y: int) -> int</h1>
<p>Computes the sum of two integers.</p>
<h2>Examples</h2>
<code>assert_eq!(add(2, 3), 5);</code>
```

---

## Error Messages

### Minimum Standards

All tools follow this error format:

```
<file>:<line>:<col>: <severity>[<code>]: <message>
```

**Example:**

```
main.rz:12:5: error[E0308]: mismatched types
  expected: int
  found:    string
```

### Severity Levels

| Level | When | Blocking |
|-------|------|----------|
| error | Code cannot compile/run | Yes |
| warning | Code works but suspicious | No |
| note | Additional context | No |
| hint | Suggestion for fix | No |

---

## Performance Baselines

| Tool | Baseline | Regression Threshold |
|------|----------|----------------------|
| Formatter (`rz fmt`) | < 1s for 10K LOC | + 50% |
| Compiler (`rz build`) | < 5s for typical project | + 50% |
| LSP response | < 500ms | + 100% |
| Tests | < 10s for full suite | + 30% |
| Lint | < 2s for typical file | + 50% |

---

## Adoption Roadmap

### v0.2 (Current)

- [x] Formatter (basic)
- [x] Compiler
- [ ] LSP (basic completion only)
- [ ] Package manager (fetch only)

### v0.3

- [x] Formatter (stable)
- [x] LSP (diagnostics, hover, go-to-def)
- [x] Build system (incremental)

### v0.4

- [x] Package manager (install, add, remove)
- [ ] Debugger (breakpoints)
- [ ] REPL

### v0.5

- [ ] LSP (rename, references)
- [ ] Documentation generator
- [ ] Lint rules

### v0.6+

- [ ] Advanced debugger features
- [ ] Performance profiler
- [ ] Test framework

---

## Quality Assurance

### Continuous Validation

Every tool change must pass:

1. **Unit tests:** Tool functions tested in isolation
2. **Integration tests:** End-to-end tool workflows
3. **Performance tests:** Regression detection
4. **Compatibility tests:** Works across platforms
5. **User tests:** Real-world usage scenarios

### Release Criteria

A tool can release when:

- [ ] All tests pass
- [ ] Performance baselines met
- [ ] No critical bugs reported
- [ ] Documentation complete
- [ ] Works on all target platforms

---

## User Expectations

When a tool is Stable, users can expect:

✅ Reliable behavior (no surprises)  
✅ Consistent performance (no random slowdowns)  
✅ Clear error messages (easy to fix issues)  
✅ Backward compatibility (old code still works)  
✅ Support for all documented features  

---

## References

- **RES-3508:** Set a tooling quality bar for the language platform
- **RES-3502:** Module and package system design
- **STABILITY_POLICY.md:** Backward compatibility guarantees
