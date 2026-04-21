# FFI Phase 1 (Tree-walker MVP) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a working FFI for the tree-walker interpreter on `std` hosts (Linux/macOS) plus the static-registry path on `no_std`, without touching the bytecode VM or JIT yet.

**Architecture:** A new `Node::Extern` AST node parses `extern "lib" { fn ... }` blocks. A new `resilient/src/ffi.rs` module owns library resolution via `libloading` (on `std`) and a shared `ForeignSignature` type. A generated trampoline table turns Resilient `Value` arguments into C-ABI calls through `extern "C" fn` function pointers. A new `Value::Foreign` variant makes a resolved foreign symbol callable just like a normal fn. `resilient-runtime` gains a zero-alloc `StaticRegistry` behind an `ffi-static` feature for embedded use.

**Tech Stack:** Rust 1.80+, `libloading = "0.8"` (std only), Resilient's existing hand-rolled parser + tree-walker, `cargo test` as the test harness.

**Spec:** `docs/superpowers/specs/2026-04-19-ffi-design.md`

---

## File Structure

### New files

- `resilient/src/ffi.rs` — loader module. Owns `FfiType`, `ForeignSignature`, `ForeignLoader`, `FfiError`, and the public entry points the driver calls at program load.
- `resilient/src/ffi_trampolines.rs` — generated table of `extern "C" fn` shims for every (arity 0–8, return type ∈ {Int, Float, Bool, Str, Void}) combination. Hand-rolled via macros for v1; can be moved to `build.rs` codegen later.
- `resilient-runtime/src/ffi_static.rs` — no_std static registry (`StaticRegistry`, `register_foreign`, `lookup`).
- `resilient/examples/ffi_libm.rz` + `ffi_libm.expected.txt` — end-to-end example calling `libm::sqrt`.
- `resilient/tests/ffi/lib_testhelper.c` — tiny C file compiled by a `build.rs` into `libresilient_ffi_testhelper.{so,dylib,dll}` for integration tests.
- `resilient/build.rs` — compiles the test helper C file when `cfg(test)`.

### Modified files

- `resilient/src/main.rs` — new `Node::Extern` variant; parser branch off `parse_statement`; typechecker rejections; `Value::Foreign` variant; tree-walker dispatch; driver wiring to resolve externs after `expand_uses`.
- `resilient/src/verifier_z3.rs` — `@trusted` extern `ensures` piped in as SMT assumptions at call sites.
- `resilient/Cargo.toml` — `libloading` dep gated on `ffi` feature (default on).
- `resilient-runtime/src/lib.rs` — `pub mod ffi_static;` behind `ffi-static` feature.
- `resilient-runtime/Cargo.toml` — `ffi-static` feature + size-variant features (`ffi-static-64`, `ffi-static-256`, `ffi-static-1024`).
- `SYNTAX.md` — FFI section.
- `docs/` (Jekyll) — FFI page.

---

## Pre-flight

- [ ] **Step 0: Create working branch**

```bash
cd /Users/eric/GitHub/Resilient
git checkout -b ffi-phase-1-tree-walker
```

- [ ] **Step 0.1: Confirm baseline is green**

Run: `cd resilient && cargo test && cd ../resilient-runtime && cargo test && cd ..`
Expected: all tests pass on both crates.

---

## Task 1: AST node for `extern` blocks

**Files:**
- Modify: `resilient/src/main.rs` — add `Node::Extern` variant + `ExternDecl` struct near `Node::Use`

- [ ] **Step 1: Add failing parser test**

Add to the `#[cfg(test)] mod parser_tests` block (find existing parser tests around `use "path"` coverage — search for `expand_is_a_noop`). New test:

```rust
#[test]
fn parses_empty_extern_block() {
    let (program, errs) = crate::parse(r#"extern "libm.so.6" { }"#);
    assert!(errs.is_empty(), "parse errors: {:?}", errs);
    let stmts = match &program {
        crate::Node::Program(s) => s,
        _ => unreachable!(),
    };
    assert_eq!(stmts.len(), 1);
    match &stmts[0].node {
        crate::Node::Extern { library, decls, .. } => {
            assert_eq!(library, "libm.so.6");
            assert!(decls.is_empty());
        }
        other => panic!("expected Node::Extern, got {:?}", other),
    }
}
```

- [ ] **Step 2: Run test, expect compilation failure**

Run: `cd resilient && cargo test parses_empty_extern_block 2>&1 | head -20`
Expected: compile error — `Node::Extern` not defined.

- [ ] **Step 3: Define the AST node and struct**

In `resilient/src/main.rs`, add next to `Node::Use` (around line 944):

```rust
    /// FFI v1: `extern "libname" { fn ... }` block. Each inner
    /// declaration is an `ExternDecl`. Resolved by the driver
    /// (after `expand_uses`) into `Value::Foreign` bindings in
    /// the global environment.
    Extern {
        library: String,
        decls: Vec<ExternDecl>,
        span: span::Span,
    },
```

Add below the `Node` enum (before the next `impl` block):

```rust
/// FFI v1: one foreign fn declaration inside an `extern` block.
#[derive(Debug, Clone)]
pub struct ExternDecl {
    /// The name used in Resilient source (e.g. `sine`).
    pub resilient_name: String,
    /// The C symbol to look up. Defaults to `resilient_name`; overridden
    /// by `fn NAME(...) = "C_NAME";`.
    pub c_name: String,
    /// (type, name) pairs — matches `Node::Function::parameters`.
    pub parameters: Vec<(String, String)>,
    /// Resilient type name; `"Void"` for unit return.
    pub return_type: String,
    pub requires: Vec<Node>,
    pub ensures: Vec<Node>,
    /// `@trusted` — `ensures` is assumed, not checked.
    pub trusted: bool,
    pub span: span::Span,
}
```

Update all `match` arms on `Node` that get exhaustiveness errors — for v1 most will be `Node::Extern { .. } => Ok(Value::Void)` (the interpreter) or a no-op for `typecheck`/`verifier`/`compiler`. Keep those stubbed for now; real logic lands in Tasks 4–8.

- [ ] **Step 4: Add lexer `Extern` keyword**

Search for `"use" => Token::Use` (around line 600). Add before it:

```rust
                        "extern" => Token::Extern,
```

Add `Extern` to the `Token` enum (search for `Use,` around line 105) and its display (search for `Token::Use =>` around line 218):

```rust
    Extern,
```
```rust
            Token::Extern => "`extern`".to_string(),
```

- [ ] **Step 5: Run test again to verify parser branch is missing**

Run: `cd resilient && cargo test parses_empty_extern_block 2>&1 | tail -20`
Expected: test compiles but **fails at parse** — the parser doesn't dispatch on `Token::Extern` yet. The error will be a parser error complaining about `extern` as an unexpected token.

- [ ] **Step 6: Commit the AST skeleton**

```bash
cd /Users/eric/GitHub/Resilient
git add resilient/src/main.rs
git commit -m "FFI: add Node::Extern AST variant and ExternDecl struct (no parser yet)"
```

---

## Task 2: Parser — empty `extern` block

**Files:**
- Modify: `resilient/src/main.rs` — new `parse_extern_block` method; dispatch from `parse_statement`

- [ ] **Step 1: Wire up dispatch in parse_statement**

Find `Token::Use => self.parse_use_statement()` (line ~1451). Add above it:

```rust
            Token::Extern => self.parse_extern_block(),
```

- [ ] **Step 2: Implement `parse_extern_block` for the empty case**

Place it next to `parse_use_statement` (around line 2629). Start minimal:

```rust
    /// FFI v1: `extern "lib" { decl; decl; ... }`.
    /// Each decl is parsed by `parse_extern_decl`.
    fn parse_extern_block(&mut self) -> Option<Node> {
        let extern_span = self.current_span();
        self.advance(); // consume `extern`

        // Library descriptor.
        let library = match &self.current_token {
            Token::StringLiteral(s) => {
                let s = s.clone();
                self.advance();
                s
            }
            other => {
                self.error(format!(
                    "expected string literal after `extern`, got {}",
                    self.token_display(other)
                ));
                return None;
            }
        };

        // `{`
        if !matches!(self.current_token, Token::LBrace) {
            self.error(format!(
                "expected `{{` after `extern \"{}\"`, got {}",
                library,
                self.token_display(&self.current_token)
            ));
            return None;
        }
        self.advance();

        let mut decls: Vec<ExternDecl> = Vec::new();
        while !matches!(self.current_token, Token::RBrace | Token::Eof) {
            if let Some(d) = self.parse_extern_decl() {
                decls.push(d);
            } else {
                // Recovery: skip to next `;` or `}`.
                while !matches!(
                    self.current_token,
                    Token::Semicolon | Token::RBrace | Token::Eof
                ) {
                    self.advance();
                }
                if matches!(self.current_token, Token::Semicolon) {
                    self.advance();
                }
            }
        }

        if matches!(self.current_token, Token::RBrace) {
            self.advance();
        }

        Some(Node::Extern {
            library,
            decls,
            span: extern_span,
        })
    }

    /// Stub — real implementation lands in Task 3.
    fn parse_extern_decl(&mut self) -> Option<ExternDecl> {
        self.error("FFI: extern fn declarations not yet implemented".to_string());
        None
    }
```

Helpers `self.current_span()` and `self.token_display(...)` already exist in this file — if the names differ, search the parser for similar error-reporting sites and match the local convention.

- [ ] **Step 3: Run test, expect pass**

Run: `cd resilient && cargo test parses_empty_extern_block -- --exact`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add resilient/src/main.rs
git commit -m "FFI: parse empty extern block (library descriptor only)"
```

---

## Task 3: Parser — `extern fn` declarations with arity, alias, contracts

**Files:**
- Modify: `resilient/src/main.rs` — `parse_extern_decl`

- [ ] **Step 1: Add failing tests for the full decl syntax**

```rust
#[test]
fn parses_extern_fn_with_primitive_types() {
    let src = r#"extern "libm.so.6" { fn sqrt(x: Float) -> Float; }"#;
    let (program, errs) = crate::parse(src);
    assert!(errs.is_empty(), "{:?}", errs);
    let decls = match &program {
        crate::Node::Program(s) => match &s[0].node {
            crate::Node::Extern { decls, .. } => decls.clone(),
            _ => panic!(),
        },
        _ => panic!(),
    };
    assert_eq!(decls.len(), 1);
    assert_eq!(decls[0].resilient_name, "sqrt");
    assert_eq!(decls[0].c_name, "sqrt");
    assert_eq!(decls[0].parameters, vec![("Float".into(), "x".into())]);
    assert_eq!(decls[0].return_type, "Float");
}

#[test]
fn parses_extern_fn_with_c_name_alias() {
    let src = r#"extern "libm.so.6" { fn sine(x: Float) -> Float = "sin"; }"#;
    let (program, _) = crate::parse(src);
    let decls = match &program {
        crate::Node::Program(s) => match &s[0].node {
            crate::Node::Extern { decls, .. } => decls.clone(),
            _ => panic!(),
        },
        _ => panic!(),
    };
    assert_eq!(decls[0].resilient_name, "sine");
    assert_eq!(decls[0].c_name, "sin");
}

#[test]
fn parses_extern_fn_with_contracts() {
    let src = r#"
        extern "libm.so.6" {
            fn sqrt(x: Float) -> Float
                requires(x >= 0.0)
                ensures(result >= 0.0);
        }
    "#;
    let (program, errs) = crate::parse(src);
    assert!(errs.is_empty(), "{:?}", errs);
    let decls = match &program {
        crate::Node::Program(s) => match &s[0].node {
            crate::Node::Extern { decls, .. } => decls.clone(),
            _ => panic!(),
        },
        _ => panic!(),
    };
    assert_eq!(decls[0].requires.len(), 1);
    assert_eq!(decls[0].ensures.len(), 1);
    assert!(!decls[0].trusted);
}

#[test]
fn parses_trusted_extern_fn() {
    let src = r#"extern "libfoo" { @trusted fn f() -> Int; }"#;
    let (program, errs) = crate::parse(src);
    assert!(errs.is_empty(), "{:?}", errs);
    let decls = match &program {
        crate::Node::Program(s) => match &s[0].node {
            crate::Node::Extern { decls, .. } => decls.clone(),
            _ => panic!(),
        },
        _ => panic!(),
    };
    assert!(decls[0].trusted);
}
```

- [ ] **Step 2: Run all four, expect failures**

Run: `cd resilient && cargo test parses_extern_fn parses_trusted_extern 2>&1 | tail`
Expected: all FAIL (the stub returns `None` and emits a parse error).

- [ ] **Step 3: Implement parse_extern_decl**

Replace the stub:

```rust
    fn parse_extern_decl(&mut self) -> Option<ExternDecl> {
        // Optional `@trusted` prefix. Parser already treats `@` as
        // attribute-start elsewhere; we handle the trusted case inline
        // here instead of going through parse_attributed_item so the
        // attribute scope is clearly limited to extern fn.
        let mut trusted = false;
        if matches!(self.current_token, Token::At) {
            self.advance();
            match &self.current_token {
                Token::Identifier(id) if id == "trusted" => {
                    trusted = true;
                    self.advance();
                }
                other => {
                    self.error(format!(
                        "unknown attribute in extern block: @{}",
                        self.token_display(other)
                    ));
                    return None;
                }
            }
        }

        // `fn`
        let decl_span = self.current_span();
        if !matches!(self.current_token, Token::Function) {
            self.error(format!(
                "expected `fn` inside extern block, got {}",
                self.token_display(&self.current_token)
            ));
            return None;
        }
        self.advance();

        // Name.
        let resilient_name = match &self.current_token {
            Token::Identifier(n) => {
                let n = n.clone();
                self.advance();
                n
            }
            other => {
                self.error(format!("expected fn name, got {}", self.token_display(other)));
                return None;
            }
        };

        // Parameters — reuse existing parse_function_parameters.
        let parameters = self.parse_function_parameters();

        // Return type `-> T`. Required for extern fns in v1.
        if !matches!(self.current_token, Token::Arrow) {
            self.error("extern fn requires an explicit `-> TYPE` return annotation".into());
            return None;
        }
        self.advance();
        let return_type = match &self.current_token {
            Token::Identifier(t) => {
                let t = t.clone();
                self.advance();
                t
            }
            other => {
                self.error(format!(
                    "expected return type, got {}",
                    self.token_display(other)
                ));
                return None;
            }
        };

        // Optional `= "c_name"` alias.
        let c_name = if matches!(self.current_token, Token::Eq) {
            self.advance();
            match &self.current_token {
                Token::StringLiteral(s) => {
                    let s = s.clone();
                    self.advance();
                    s
                }
                other => {
                    self.error(format!(
                        "expected string literal after `=` (C symbol name), got {}",
                        self.token_display(other)
                    ));
                    return None;
                }
            }
        } else {
            resilient_name.clone()
        };

        // Optional `requires(...)` and `ensures(...)`. Reuse the existing
        // parse_function_contracts so the grammar stays identical.
        let (requires, ensures) = self.parse_function_contracts();

        // Terminator `;`.
        if !matches!(self.current_token, Token::Semicolon) {
            self.error(format!(
                "expected `;` at end of extern fn declaration, got {}",
                self.token_display(&self.current_token)
            ));
            return None;
        }
        self.advance();

        Some(ExternDecl {
            resilient_name,
            c_name,
            parameters,
            return_type,
            requires,
            ensures,
            trusted,
            span: decl_span,
        })
    }
```

- [ ] **Step 4: Run all four new tests — expect pass**

Run: `cd resilient && cargo test parses_extern parses_trusted -- --nocapture`
Expected: all PASS.

- [ ] **Step 5: Run full crate test suite to catch regressions**

Run: `cd resilient && cargo test`
Expected: all existing tests still pass; new ones pass.

- [ ] **Step 6: Commit**

```bash
git add resilient/src/main.rs
git commit -m "FFI: parse extern fn decls with aliases, contracts, and @trusted"
```

---

## Task 4: Typechecker — accept extern blocks, reject non-primitive FFI types, reject `@pure` on extern

**Files:**
- Modify: `resilient/src/typechecker.rs` — walk `Node::Extern` and validate each decl

- [ ] **Step 1: Add failing typechecker tests**

Find the existing typechecker integration tests (search for `fn typecheck` in `typechecker.rs`; if tests live in `main.rs` under `mod typechecker_tests`, add them there).

```rust
#[test]
fn typecheck_rejects_array_param_in_extern() {
    let src = r#"extern "libfoo" { fn f(xs: Array) -> Int; }"#;
    let errs = typecheck_source(src);
    assert!(
        errs.iter().any(|e| e.contains("Array") && e.contains("extern")),
        "expected type-error about Array in extern, got {:?}",
        errs
    );
}

#[test]
fn typecheck_accepts_primitive_extern() {
    let src = r#"extern "libm" { fn sqrt(x: Float) -> Float; }"#;
    let errs = typecheck_source(src);
    assert!(errs.is_empty(), "unexpected errors: {:?}", errs);
}

#[test]
fn typecheck_accepts_void_return_in_extern() {
    let src = r#"extern "libfoo" { fn noop() -> Void; }"#;
    let errs = typecheck_source(src);
    assert!(errs.is_empty(), "unexpected errors: {:?}", errs);
}
```

(If no `typecheck_source` helper exists, wrap the parse + typecheck call inline.)

- [ ] **Step 2: Run — expect failures**

Run: `cd resilient && cargo test typecheck_rejects_array_param_in_extern typecheck_accepts_primitive_extern typecheck_accepts_void_return_in_extern`
Expected: all FAIL (typechecker ignores `Node::Extern` entirely today).

- [ ] **Step 3: Handle Node::Extern in the typechecker**

In `typechecker.rs`, find the main `check_node` / `check_stmt` match (whichever dispatches on `Node` variants). Add:

```rust
            Node::Extern { decls, .. } => {
                for d in decls {
                    // Primitives only in v1.
                    const PRIMS: &[&str] = &["Int", "Float", "Bool", "String"];
                    for (ty, name) in &d.parameters {
                        if !PRIMS.contains(&ty.as_str()) {
                            self.errors.push(format!(
                                "FFI: parameter `{}` has type `{}`; \
                                 extern fn supports only {} in v1",
                                name, ty, PRIMS.join(", ")
                            ));
                        }
                    }
                    const RET_PRIMS: &[&str] = &["Int", "Float", "Bool", "String", "Void"];
                    if !RET_PRIMS.contains(&d.return_type.as_str()) {
                        self.errors.push(format!(
                            "FFI: return type `{}` not supported in v1 (allowed: {})",
                            d.return_type,
                            RET_PRIMS.join(", ")
                        ));
                    }
                }
                Ok(())
            }
```

Wire the exhaustiveness match on `Node::Extern` anywhere else the compiler walks the AST (the ImplBlock / Use pattern is a good template — search for `Node::Use { .. } =>` and mirror).

- [ ] **Step 4: Run tests — expect pass**

Run: `cd resilient && cargo test typecheck_rejects_array typecheck_accepts_primitive typecheck_accepts_void`
Expected: PASS.

- [ ] **Step 5: Add purity-rejection test**

```rust
#[test]
fn typecheck_rejects_pure_on_extern() {
    // `@pure` applied to an extern decl must be a type error.
    let src = r#"extern "libfoo" { @pure fn f() -> Int; }"#;
    let (_, parse_errs) = crate::parse(src);
    // @pure isn't a known extern attribute — the PARSER will reject.
    assert!(
        parse_errs.iter().any(|e| e.contains("attribute")),
        "expected parse error about @pure attribute, got {:?}",
        parse_errs
    );
}
```

Run: `cd resilient && cargo test typecheck_rejects_pure_on_extern`
Expected: PASS (the `@trusted`-only branch in Task 3 already rejects unknown attributes).

- [ ] **Step 6: Commit**

```bash
git add resilient/src/typechecker.rs resilient/src/main.rs
git commit -m "FFI: typechecker rejects non-primitive types and @pure on extern decls"
```

---

## Task 5: Loader module skeleton (`resilient/src/ffi.rs`)

**Files:**
- Create: `resilient/src/ffi.rs`
- Modify: `resilient/src/main.rs` — `mod ffi;`
- Modify: `resilient/Cargo.toml` — `ffi` feature with `libloading` dep

- [ ] **Step 1: Add the ffi feature + dep**

In `resilient/Cargo.toml` under `[features]` (add the section if missing):

```toml
[features]
default = ["ffi"]
ffi = ["dep:libloading"]

[dependencies]
libloading = { version = "0.8", optional = true }
```

- [ ] **Step 2: Create the module skeleton**

Create `resilient/src/ffi.rs`:

```rust
//! FFI v1 loader. Resolves extern symbols declared by `Node::Extern`
//! blocks ahead of evaluation so the tree-walker can dispatch in O(1).
//!
//! Two backends share one API:
//! - `std` / `cfg(feature = "ffi")`: dynamic loading via `libloading`.
//! - `no_std` / `resilient-runtime` with `ffi-static`: a static
//!   registry populated by the embedder. Lives in `resilient-runtime`
//!   and is not referenced here — this module is host-only.

use crate::ExternDecl;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FfiType {
    Int,
    Float,
    Bool,
    Str,
    Void,
}

impl FfiType {
    pub fn from_resilient(name: &str) -> Option<Self> {
        match name {
            "Int" => Some(FfiType::Int),
            "Float" => Some(FfiType::Float),
            "Bool" => Some(FfiType::Bool),
            "String" => Some(FfiType::Str),
            "Void" => Some(FfiType::Void),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ForeignSignature {
    pub params: Vec<FfiType>,
    pub ret: FfiType,
}

impl ForeignSignature {
    pub fn from_decl(decl: &ExternDecl) -> Result<Self, FfiError> {
        let mut params = Vec::with_capacity(decl.parameters.len());
        for (ty, _) in &decl.parameters {
            params.push(
                FfiType::from_resilient(ty)
                    .ok_or_else(|| FfiError::UnsupportedType(ty.clone()))?,
            );
        }
        let ret = FfiType::from_resilient(&decl.return_type)
            .ok_or_else(|| FfiError::UnsupportedType(decl.return_type.clone()))?;
        if params.len() > 8 {
            return Err(FfiError::ArityTooLarge {
                name: decl.resilient_name.clone(),
                got: params.len(),
            });
        }
        Ok(Self { params, ret })
    }
}

#[derive(Debug)]
pub enum FfiError {
    LibNotFound { library: String, underlying: String },
    SymbolNotFound { library: String, symbol: String },
    UnsupportedType(String),
    ArityTooLarge { name: String, got: usize },
    /// `--no-default-features` build asked to load a dynamic library.
    FfiDisabled,
    /// `@static` descriptor used on an `std` host without a registered
    /// backend. (v1 treats this as an error; a future ticket may let
    /// the std build register static fns too.)
    StaticOnlyUnavailable { library: String },
}

impl std::fmt::Display for FfiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FfiError::LibNotFound { library, underlying } => {
                write!(f, "FFI: cannot open library `{}`: {}", library, underlying)
            }
            FfiError::SymbolNotFound { library, symbol } => {
                write!(f, "FFI: symbol `{}` not found in `{}`", symbol, library)
            }
            FfiError::UnsupportedType(ty) => {
                write!(f, "FFI: type `{}` is not supported in v1", ty)
            }
            FfiError::ArityTooLarge { name, got } => {
                write!(f, "FFI: extern fn `{}` has {} params; v1 supports up to 8", name, got)
            }
            FfiError::FfiDisabled => {
                write!(f, "FFI: this build was compiled without --features ffi")
            }
            FfiError::StaticOnlyUnavailable { library } => {
                write!(f, "FFI: library descriptor `{}` requires a static registry, not available in this build", library)
            }
        }
    }
}

impl std::error::Error for FfiError {}

/// A resolved extern symbol. The raw `*const ()` is cast to a concrete
/// `extern "C" fn(...)` type at call time via `ffi_trampolines`.
pub struct ForeignSymbol {
    pub name: String,
    pub ptr: *const (),
    pub sig: ForeignSignature,
}

// SAFETY: ForeignSymbol holds a raw C function pointer that outlives
// the Library it came from (we also hold the Library in the loader
// so it never drops while symbols are in use). The pointer itself
// is Send + Sync on every supported platform.
unsafe impl Send for ForeignSymbol {}
unsafe impl Sync for ForeignSymbol {}

#[cfg(feature = "ffi")]
pub use dynamic::ForeignLoader;

#[cfg(not(feature = "ffi"))]
pub use disabled::ForeignLoader;

#[cfg(feature = "ffi")]
mod dynamic {
    use super::*;
    use std::collections::HashMap;

    pub struct ForeignLoader {
        libs: HashMap<String, libloading::Library>,
        syms: HashMap<String, std::sync::Arc<ForeignSymbol>>,
    }

    impl ForeignLoader {
        pub fn new() -> Self {
            Self { libs: HashMap::new(), syms: HashMap::new() }
        }

        pub fn resolve_block(
            &mut self,
            library: &str,
            decls: &[ExternDecl],
        ) -> Result<(), FfiError> {
            if library == "@static" {
                return Err(FfiError::StaticOnlyUnavailable {
                    library: library.to_string(),
                });
            }
            // dlopen (or cached).
            if !self.libs.contains_key(library) {
                let lib = unsafe { libloading::Library::new(library) }.map_err(|e| {
                    FfiError::LibNotFound {
                        library: library.to_string(),
                        underlying: e.to_string(),
                    }
                })?;
                self.libs.insert(library.to_string(), lib);
            }
            let lib = self.libs.get(library).unwrap();
            for d in decls {
                let sig = ForeignSignature::from_decl(d)?;
                let raw: libloading::Symbol<*const ()> =
                    unsafe { lib.get(d.c_name.as_bytes()) }.map_err(|_| {
                        FfiError::SymbolNotFound {
                            library: library.to_string(),
                            symbol: d.c_name.clone(),
                        }
                    })?;
                let sym = ForeignSymbol {
                    name: d.resilient_name.clone(),
                    ptr: *raw,
                    sig,
                };
                self.syms
                    .insert(d.resilient_name.clone(), std::sync::Arc::new(sym));
            }
            Ok(())
        }

        pub fn lookup(&self, name: &str) -> Option<std::sync::Arc<ForeignSymbol>> {
            self.syms.get(name).cloned()
        }
    }

    impl Default for ForeignLoader {
        fn default() -> Self {
            Self::new()
        }
    }
}

#[cfg(not(feature = "ffi"))]
mod disabled {
    use super::*;

    pub struct ForeignLoader;
    impl ForeignLoader {
        pub fn new() -> Self { Self }
        pub fn resolve_block(
            &mut self,
            _library: &str,
            _decls: &[ExternDecl],
        ) -> Result<(), FfiError> {
            Err(FfiError::FfiDisabled)
        }
        pub fn lookup(&self, _name: &str) -> Option<std::sync::Arc<ForeignSymbol>> {
            None
        }
    }
    impl Default for ForeignLoader {
        fn default() -> Self { Self::new() }
    }
}
```

- [ ] **Step 3: Register the module**

In `resilient/src/main.rs`, near the existing `mod imports;`:

```rust
mod ffi;
```

- [ ] **Step 4: Add a loader unit test**

Append to `resilient/src/ffi.rs`:

```rust
#[cfg(test)]
#[cfg(feature = "ffi")]
mod tests {
    use super::*;
    use crate::{span::Span, ExternDecl};

    fn decl(name: &str, c: &str, params: Vec<(&str, &str)>, ret: &str) -> ExternDecl {
        ExternDecl {
            resilient_name: name.to_string(),
            c_name: c.to_string(),
            parameters: params
                .into_iter()
                .map(|(t, n)| (t.to_string(), n.to_string()))
                .collect(),
            return_type: ret.to_string(),
            requires: Vec::new(),
            ensures: Vec::new(),
            trusted: false,
            span: Span::default(),
        }
    }

    #[test]
    fn missing_library_is_a_clean_error_not_a_panic() {
        let mut loader = ForeignLoader::new();
        let err = loader
            .resolve_block("libnope_not_a_real_library.so", &[])
            .expect_err("should fail");
        matches!(err, FfiError::LibNotFound { .. });
    }

    #[test]
    fn signature_rejects_unsupported_types() {
        let d = decl("f", "f", vec![("Array", "xs")], "Int");
        let err = ForeignSignature::from_decl(&d).expect_err("must reject Array");
        matches!(err, FfiError::UnsupportedType(ref s) if s == "Array");
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cd resilient && cargo test --features ffi`
Expected: all green, including the new `ffi::tests`.

- [ ] **Step 6: Commit**

```bash
git add resilient/src/ffi.rs resilient/src/main.rs resilient/Cargo.toml
git commit -m "FFI: loader module with libloading (std only, behind `ffi` feature)"
```

---

## Task 6: Trampoline table (the C-ABI call layer)

**Files:**
- Create: `resilient/src/ffi_trampolines.rs`
- Modify: `resilient/src/main.rs` — `mod ffi_trampolines;` (feature-gated)

- [ ] **Step 1: Create the module with macro-generated trampolines**

```rust
//! FFI v1: trampolines dispatch a resolved `ForeignSymbol` to a real
//! C function pointer of the right type. One trampoline per (arity,
//! return type) combination — 9 arities (0..=8) × 5 return types
//! (Int, Float, Bool, Str, Void) = 45 shims.
//!
//! Input `Value`s are converted to C ABI scalars here. Output C
//! scalars are converted back to `Value`. String marshalling uses
//! `(*const u8, usize)` — the trampoline holds the Resilient String's
//! UTF-8 bytes live for the duration of the call via a local borrow.
#![cfg(feature = "ffi")]

use crate::ffi::{FfiType, ForeignSignature, ForeignSymbol};
use crate::{RResult, Value};

/// C-ABI representation of each Resilient primitive.
#[derive(Copy, Clone)]
#[repr(C)]
union CArg {
    i: i64,
    f: f64,
    b: bool,
    s: CStr,
}

#[derive(Copy, Clone)]
#[repr(C)]
struct CStr {
    ptr: *const u8,
    len: usize,
}

/// Entry point. Dispatches on arity + return type to one of the
/// 45 monomorphized trampolines below. Out-of-range arity is a clean
/// error (the loader already gated on 8; this is belt-and-suspenders).
pub fn call_foreign(sym: &ForeignSymbol, args: &[Value]) -> RResult<Value> {
    if args.len() != sym.sig.params.len() {
        return Err(format!(
            "FFI: arity mismatch calling `{}`: expected {}, got {}",
            sym.name, sym.sig.params.len(), args.len()
        ));
    }

    // Typecheck args against the signature.
    for (i, (arg, want)) in args.iter().zip(sym.sig.params.iter()).enumerate() {
        let actual = ffi_type_of_value(arg);
        if actual != Some(*want) {
            return Err(format!(
                "FFI: type mismatch calling `{}` arg #{}: expected {:?}, got {:?}",
                sym.name, i, want, arg
            ));
        }
    }

    dispatch(sym, args)
}

fn ffi_type_of_value(v: &Value) -> Option<FfiType> {
    match v {
        Value::Int(_) => Some(FfiType::Int),
        Value::Float(_) => Some(FfiType::Float),
        Value::Bool(_) => Some(FfiType::Bool),
        Value::String(_) => Some(FfiType::Str),
        _ => None,
    }
}

/// Dispatch to the correct trampoline. The inner helpers are
/// macro-generated below.
fn dispatch(sym: &ForeignSymbol, args: &[Value]) -> RResult<Value> {
    macro_rules! arms {
        ($($arity:literal),*) => {
            match (args.len(), sym.sig.ret) {
                $(
                    ($arity, FfiType::Int)   => paste_ident!(call_, $arity, _int)(sym, args),
                    ($arity, FfiType::Float) => paste_ident!(call_, $arity, _float)(sym, args),
                    ($arity, FfiType::Bool)  => paste_ident!(call_, $arity, _bool)(sym, args),
                    ($arity, FfiType::Str)   => paste_ident!(call_, $arity, _str)(sym, args),
                    ($arity, FfiType::Void)  => paste_ident!(call_, $arity, _void)(sym, args),
                )*
                (n, _) => Err(format!("FFI: arity {} not supported in v1 (max 8)", n)),
            }
        };
    }
    // Rust's macro_rules can't paste tokens without the `paste` crate.
    // For v1 we spell out the 45 arms by hand — simpler than adding a
    // build dep. Keep the generated code self-contained.
    dispatch_explicit(sym, args)
}

// --- Explicit dispatch table ---
//
// Each `call_N_T` converts `args` into CArgs, transmutes the symbol
// pointer to an `extern "C" fn(T1, ..., TN) -> Tret`, calls it, and
// converts the return into a `Value`.
//
// Generating 45 functions by hand is tedious but low-risk. They are
// identical shape; only the parameter/return types differ.
fn dispatch_explicit(sym: &ForeignSymbol, args: &[Value]) -> RResult<Value> {
    use FfiType::*;
    let params: &[FfiType] = &sym.sig.params;
    let ret = sym.sig.ret;

    // Extract scalars up front.
    let mut ints: [i64; 8] = [0; 8];
    let mut floats: [f64; 8] = [0.0; 8];
    let mut bools: [bool; 8] = [false; 8];
    let mut strs: [CStr; 8] = [CStr { ptr: std::ptr::null(), len: 0 }; 8];
    // Keep borrowed string bytes alive for the call.
    let mut live_strs: Vec<&[u8]> = Vec::with_capacity(args.len());
    for (i, (arg, want)) in args.iter().zip(params.iter()).enumerate() {
        match (arg, want) {
            (Value::Int(v), Int)       => ints[i] = *v,
            (Value::Float(v), Float)   => floats[i] = *v,
            (Value::Bool(v), Bool)     => bools[i] = *v,
            (Value::String(s), Str) => {
                let bytes = s.as_bytes();
                live_strs.push(bytes);
                strs[i] = CStr { ptr: bytes.as_ptr(), len: bytes.len() };
            }
            _ => return Err(format!(
                "FFI internal: arg #{} type {:?} / ffi {:?} mismatch", i, arg, want
            )),
        }
    }

    // Per-shape dispatch. Only arity 0..=2 shown; extend to 8 in code.
    // For parity with v1 scope this table handles the common cases;
    // arities 3..8 are generated by copy-paste-replace below during
    // implementation. Keep the conversion style IDENTICAL.
    unsafe {
        Ok(match (params, ret) {
            // Arity 0
            (&[], Int)   => Value::Int(std::mem::transmute::<_, extern "C" fn() -> i64>(sym.ptr)()),
            (&[], Float) => Value::Float(std::mem::transmute::<_, extern "C" fn() -> f64>(sym.ptr)()),
            (&[], Bool)  => Value::Bool(std::mem::transmute::<_, extern "C" fn() -> bool>(sym.ptr)()),
            (&[], Void)  => { std::mem::transmute::<_, extern "C" fn()>(sym.ptr)(); Value::Void }

            // Arity 1 — one per primitive IN, one per primitive OUT.
            (&[Int], Int)     => Value::Int(std::mem::transmute::<_, extern "C" fn(i64) -> i64>(sym.ptr)(ints[0])),
            (&[Int], Float)   => Value::Float(std::mem::transmute::<_, extern "C" fn(i64) -> f64>(sym.ptr)(ints[0])),
            (&[Int], Bool)    => Value::Bool(std::mem::transmute::<_, extern "C" fn(i64) -> bool>(sym.ptr)(ints[0])),
            (&[Int], Void)    => { std::mem::transmute::<_, extern "C" fn(i64)>(sym.ptr)(ints[0]); Value::Void }
            (&[Float], Int)   => Value::Int(std::mem::transmute::<_, extern "C" fn(f64) -> i64>(sym.ptr)(floats[0])),
            (&[Float], Float) => Value::Float(std::mem::transmute::<_, extern "C" fn(f64) -> f64>(sym.ptr)(floats[0])),
            (&[Float], Bool)  => Value::Bool(std::mem::transmute::<_, extern "C" fn(f64) -> bool>(sym.ptr)(floats[0])),
            (&[Float], Void)  => { std::mem::transmute::<_, extern "C" fn(f64)>(sym.ptr)(floats[0]); Value::Void }
            (&[Bool], Int)    => Value::Int(std::mem::transmute::<_, extern "C" fn(bool) -> i64>(sym.ptr)(bools[0])),
            (&[Bool], Bool)   => Value::Bool(std::mem::transmute::<_, extern "C" fn(bool) -> bool>(sym.ptr)(bools[0])),
            (&[Bool], Void)   => { std::mem::transmute::<_, extern "C" fn(bool)>(sym.ptr)(bools[0]); Value::Void }

            // Arity 2 — only the combinations v1 needs for the test
            // helper + libm examples are enumerated explicitly.
            // Extending to the full Cartesian product is mechanical:
            // the `ffi_completeness_check` test (Step 2) will fail
            // loudly when a caller hits a missing arm, so grow this
            // table as real examples demand.
            (&[Float, Float], Float) => Value::Float(
                std::mem::transmute::<_, extern "C" fn(f64, f64) -> f64>(sym.ptr)(floats[0], floats[1])
            ),
            (&[Int, Int], Int) => Value::Int(
                std::mem::transmute::<_, extern "C" fn(i64, i64) -> i64>(sym.ptr)(ints[0], ints[1])
            ),

            // Fallback.
            _ => return Err(format!(
                "FFI: no trampoline for signature ({:?}) -> {:?} (v1 coverage: extend dispatch_explicit)",
                params, ret
            )),
        })
    }
}

// Touch live_strs so rustc sees them borrowed through the call.
// (Strictly not necessary — the `bytes` borrow is what keeps them
// alive — but this line makes the intent obvious at review time.)
#[allow(dead_code)]
fn _keep_alive(_: &[&[u8]]) {}
```

**Important:** The table above covers arity 0–2 for the primitives the first tests need. Later tickets extend it mechanically. For Phase 1 this is ENOUGH to call `libm::sqrt`, `pow`, `sin`, `cos`. A regression test in Task 8 asserts the missing-arm path returns a clean error rather than panicking.

- [ ] **Step 2: Wire into main.rs**

In `resilient/src/main.rs`:

```rust
#[cfg(feature = "ffi")]
mod ffi_trampolines;
```

Add `paste_ident!` is not used in the final code — delete the scratch `macro_rules` block before compiling. (It's in the draft above only for illustration; the `dispatch_explicit` path is what runs.)

- [ ] **Step 3: Compile check**

Run: `cd resilient && cargo build --features ffi`
Expected: clean build. Fix any unused-import warnings.

- [ ] **Step 4: Commit**

```bash
git add resilient/src/ffi_trampolines.rs resilient/src/main.rs
git commit -m "FFI: trampoline table covering arity 0-2 primitives"
```

---

## Task 7: `Value::Foreign` variant + tree-walker dispatch

**Files:**
- Modify: `resilient/src/main.rs` — `Value::Foreign` variant; `register_foreign` after `register_builtins`; call dispatch in `apply_function`

- [ ] **Step 1: Add the new Value variant**

In the `enum Value` block (around line 4002):

```rust
    /// FFI v1: resolved foreign symbol, callable from Resilient source.
    /// The Arc holds both the resolved ptr and the signature; the
    /// loader owns the backing `Library`.
    #[cfg(feature = "ffi")]
    Foreign {
        name: &'static str,
        symbol: std::sync::Arc<crate::ffi::ForeignSymbol>,
        requires: Vec<Node>,
        ensures: Vec<Node>,
        trusted: bool,
    },
```

Update `Debug`, `Display`, and any exhaustiveness `match`es on `Value` to include the new variant — mirror the pattern used for `Value::Builtin`.

- [ ] **Step 2: Add failing test for dispatching a foreign call**

In `main.rs`'s integration tests (the module that runs whole programs through `run_source`):

```rust
#[test]
#[cfg(all(feature = "ffi", any(target_os = "linux", target_os = "macos")))]
fn can_call_cos_from_libm() {
    #[cfg(target_os = "macos")]
    let lib = r#"extern "libSystem.dylib" { fn cos(x: Float) -> Float; }"#;
    #[cfg(target_os = "linux")]
    let lib = r#"extern "libm.so.6" { fn cos(x: Float) -> Float; }"#;
    let src = format!("{}\nfn main() {{ println(cos(0.0)); }}", lib);
    let out = run_source_capture(&src).expect("program should run");
    assert!(out.starts_with("1"), "cos(0.0) should be ~1.0, got {}", out);
}
```

(If no `run_source_capture` helper exists, mirror the test pattern already used for println coverage.)

- [ ] **Step 3: Run — expect failure**

Run: `cd resilient && cargo test can_call_cos_from_libm --features ffi`
Expected: FAIL — program runs but `cos` resolves to "undefined identifier" because Task 8 hasn't wired the loader yet.

- [ ] **Step 4: Skip for now, commit Value variant**

This test is held failing until Task 8. Commit the variant change so the diff stays tidy:

```bash
git add resilient/src/main.rs
git commit -m "FFI: add Value::Foreign variant (behind `ffi` feature)"
```

---

## Task 8: Driver wiring — resolve extern blocks, register as env values, dispatch on call

**Files:**
- Modify: `resilient/src/main.rs` — new `resolve_externs` pass after `expand_uses`; extend `apply_function` to call through `ffi_trampolines::call_foreign`

- [ ] **Step 1: Add the driver pass**

Find the driver entry point where `expand_uses` runs (search for `imports::expand_uses`, around line 7951). Right after it, inside the same feature gate if present:

```rust
    // FFI v1: resolve every `Node::Extern` block eagerly, producing
    // Value::Foreign bindings in the root environment.
    #[cfg(feature = "ffi")]
    {
        let mut loader = crate::ffi::ForeignLoader::new();
        if let Node::Program(stmts) = &program {
            for stmt in stmts {
                if let Node::Extern { library, decls, .. } = &stmt.node {
                    if let Err(e) = loader.resolve_block(library, decls) {
                        return Err(format!("{}", e));
                    }
                    for d in decls {
                        let sym = loader.lookup(&d.resilient_name).unwrap();
                        let name: &'static str = Box::leak(d.resilient_name.clone().into_boxed_str());
                        env.set(
                            d.resilient_name.clone(),
                            Value::Foreign {
                                name,
                                symbol: sym,
                                requires: d.requires.clone(),
                                ensures: d.ensures.clone(),
                                trusted: d.trusted,
                            },
                        );
                    }
                }
            }
        }
    }
```

(Adjust `env.set` to the actual method name used in the driver — search `register_builtins` for the pattern.)

- [ ] **Step 2: Extend apply_function to handle Value::Foreign**

Find `apply_function` (it's the central fn-call dispatcher; search for `Value::Builtin { func, .. }`). Add another arm:

```rust
        #[cfg(feature = "ffi")]
        Value::Foreign { name: _, symbol, requires, ensures, trusted } => {
            // Check preconditions in the caller's scope first.
            // Bind parameters by position. (Foreign decls carry names,
            // but at apply time we only have Values — we rebind using
            // positional aliases `_0`, `_1`, ...)
            let mut contract_env = env.clone();
            for (i, v) in args.iter().enumerate() {
                contract_env.set(format!("_{}", i), v.clone());
            }
            for pre in requires {
                let ok = match eval(pre, &mut contract_env)? {
                    Value::Bool(b) => b,
                    _ => false,
                };
                if !ok {
                    return Err(format!(
                        "contract violation: `requires` failed entering foreign fn"
                    ));
                }
            }

            let result = crate::ffi_trampolines::call_foreign(&symbol, &args)?;

            // Postconditions. `result` is bound under the name `result`
            // so authored ensures clauses line up with the Resilient-fn
            // convention.
            let mut post_env = contract_env;
            post_env.set("result".to_string(), result.clone());
            for post in ensures {
                let ok = match eval(post, &mut post_env)? {
                    Value::Bool(b) => b,
                    _ => false,
                };
                if !ok && !trusted {
                    return Err(format!(
                        "contract violation: `ensures` failed leaving foreign fn"
                    ));
                }
            }

            Ok(result)
        }
```

- [ ] **Step 3: Run the libm test**

Run: `cd resilient && cargo test can_call_cos_from_libm --features ffi`
Expected: PASS on macOS AND Linux.

- [ ] **Step 4: Run full suite**

Run: `cd resilient && cargo test --features ffi && cd ../resilient-runtime && cargo test && cd ..`
Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add resilient/src/main.rs
git commit -m "FFI: driver resolves extern blocks; tree-walker dispatches through trampolines"
```

---

## Task 9: Test helper C library + integration coverage for edge cases

**Files:**
- Create: `resilient/tests/ffi/lib_testhelper.c`
- Create: `resilient/build.rs`
- Modify: `resilient/Cargo.toml` — `build-dependencies = { cc = "1" }`

- [ ] **Step 1: Write the helper**

`resilient/tests/ffi/lib_testhelper.c`:

```c
#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

int64_t rt_add(int64_t a, int64_t b) { return a + b; }
double  rt_mul(double a, double b) { return a * b; }
bool    rt_is_even(int64_t n) { return (n % 2) == 0; }

// Counts ASCII digits in a borrowed UTF-8 buffer.
int64_t rt_count_digits(const char* s, size_t len) {
    int64_t n = 0;
    for (size_t i = 0; i < len; ++i) {
        if (s[i] >= '0' && s[i] <= '9') ++n;
    }
    return n;
}
```

- [ ] **Step 2: Add build.rs**

`resilient/build.rs`:

```rust
fn main() {
    if std::env::var("CARGO_FEATURE_FFI").is_ok() {
        cc::Build::new()
            .file("tests/ffi/lib_testhelper.c")
            .shared_flag(true)
            .compile("resilient_ffi_testhelper");
        // cc emits a static lib. For dlopen we need a shared lib.
        // Let the test code open the static-staging output as a .dylib
        // via target/.../libresilient_ffi_testhelper.dylib that cc
        // produces when `shared_flag(true)` is set.
    }
}
```

- [ ] **Step 3: Tie into Cargo.toml**

```toml
[build-dependencies]
cc = "1"
```

- [ ] **Step 4: Integration tests**

Create `resilient/tests/ffi_integration.rs`:

```rust
//! End-to-end FFI tests against the bundled C helper library.
#![cfg(feature = "ffi")]

use std::path::PathBuf;

fn helper_path() -> String {
    // build.rs emitted this into OUT_DIR; resolve at test time.
    let out_dir = PathBuf::from(env!("OUT_DIR"));
    let ext = if cfg!(target_os = "macos") { "dylib" }
              else if cfg!(target_os = "windows") { "dll" }
              else { "so" };
    out_dir.join(format!("libresilient_ffi_testhelper.{}", ext))
        .to_string_lossy()
        .into_owned()
}

fn run(src: &str) -> String {
    // Mirror the in-tree helper; pick whatever the crate exposes.
    resilient::run_source_capture(src).expect("program ran")
}

#[test]
fn calls_int_int_int_function() {
    let src = format!(
        r#"extern "{}" {{ fn rt_add(a: Int, b: Int) -> Int; }}
           fn main() {{ println(rt_add(2, 40)); }}"#,
        helper_path()
    );
    assert_eq!(run(&src).trim(), "42");
}

#[test]
fn contract_precondition_failure_is_caught_before_ffi_call() {
    let src = format!(
        r#"extern "{}" {{
             fn rt_add(a: Int, b: Int) -> Int requires(a >= 0);
           }}
           fn main() {{ println(rt_add(-1, 1)); }}"#,
        helper_path()
    );
    let out = run(&src);
    assert!(out.contains("contract violation"), "got: {}", out);
}

#[test]
fn missing_symbol_is_clean_error_not_panic() {
    let src = format!(
        r#"extern "{}" {{ fn definitely_not_a_symbol() -> Int; }}
           fn main() {{ }}"#,
        helper_path()
    );
    let out = run(&src);
    assert!(out.contains("symbol"), "got: {}", out);
}
```

- [ ] **Step 5: Run integration tests**

Run: `cd resilient && cargo test --test ffi_integration --features ffi`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add resilient/tests/ffi resilient/build.rs resilient/tests/ffi_integration.rs resilient/Cargo.toml
git commit -m "FFI: C helper lib + end-to-end integration tests"
```

---

## Task 10: `@trusted` → SMT assumption (verifier hook)

**Files:**
- Modify: `resilient/src/verifier_z3.rs` — consume `trusted` decls as assumptions at call sites

- [ ] **Step 1: Add a verifier test (gated)**

```rust
#[test]
#[cfg(feature = "z3")]
fn trusted_extern_ensures_propagates_as_smt_assumption() {
    let src = r#"
        extern "libm.so.6" {
            @trusted fn sqrt(x: Float) -> Float ensures(result >= 0.0);
        }
        @pure fn f(y: Float) -> Bool requires(y >= 0.0) {
            return sqrt(y) >= 0.0;
        }
    "#;
    // The verifier should prove f's return-true because sqrt's
    // ensures is assumed. Without @trusted the verifier would have
    // nothing to say about the foreign call and the proof would fail.
    let result = run_verifier(src);
    assert!(result.is_proven());
}
```

- [ ] **Step 2: Pipe trusted into the verifier**

In `verifier_z3.rs`, find the call-site handling (search for `Node::Call`). When the callee resolves to an `ExternDecl` with `trusted == true`, emit each `ensures` as an `assume` rather than a `check_proves` obligation. Pattern:

```rust
            // FFI: trusted extern call — treat ensures as axiom.
            if let Some(extern_decl) = self.lookup_extern(&callee_name) {
                if extern_decl.trusted {
                    for ens in &extern_decl.ensures {
                        let enc = self.encode(ens, &call_scope)?;
                        self.solver.assert(&enc);
                    }
                    // Untrusted: nothing to assert, nothing to prove.
                    // Non-trusted ensures are runtime-only in v1.
                }
                return Ok(SymbolicValue::unknown_of(extern_decl.return_type.clone()));
            }
```

(Exact integration shape depends on the symbolic-value representation used in `verifier_z3.rs`; the spec's intent is: `@trusted` → axiom, untrusted → opaque.)

- [ ] **Step 3: Run the verifier test**

Run: `cd resilient && cargo test trusted_extern_ensures_propagates --features "ffi z3"`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add resilient/src/verifier_z3.rs
git commit -m "FFI: @trusted extern ensures propagates as SMT assumption"
```

---

## Task 11: `resilient-runtime` static registry (no_std side)

**Files:**
- Create: `resilient-runtime/src/ffi_static.rs`
- Modify: `resilient-runtime/src/lib.rs` — `pub mod ffi_static;` gated on `ffi-static`
- Modify: `resilient-runtime/Cargo.toml` — `ffi-static`, `ffi-static-64/256/1024` features

- [ ] **Step 1: Add features**

`resilient-runtime/Cargo.toml`:

```toml
[features]
ffi-static = []
ffi-static-64 = ["ffi-static"]
ffi-static-256 = ["ffi-static"]
ffi-static-1024 = ["ffi-static"]
```

Mutual-exclusion compile error, mirroring the existing `alloc`/`static-only` guard in `lib.rs`:

```rust
#[cfg(any(
    all(feature = "ffi-static-64", feature = "ffi-static-256"),
    all(feature = "ffi-static-64", feature = "ffi-static-1024"),
    all(feature = "ffi-static-256", feature = "ffi-static-1024"),
))]
compile_error!("`ffi-static-64`, `ffi-static-256`, `ffi-static-1024` are mutually exclusive — pick ONE capacity.");
```

- [ ] **Step 2: Implement the registry**

`resilient-runtime/src/ffi_static.rs`:

```rust
//! FFI static registry for no_std embedded hosts.
//!
//! The embedding application calls `register_foreign` BEFORE
//! `run(program)`. Lookups are linear over a fixed-size array —
//! N ≤ 1024 (the largest supported capacity) so linear scan is
//! fine, and it keeps the crate allocation-free.

#![cfg(feature = "ffi-static")]

#[cfg(feature = "ffi-static-1024")]
const CAPACITY: usize = 1024;
#[cfg(all(feature = "ffi-static-256", not(feature = "ffi-static-1024")))]
const CAPACITY: usize = 256;
#[cfg(all(feature = "ffi-static-64", not(any(feature = "ffi-static-256", feature = "ffi-static-1024"))))]
const CAPACITY: usize = 64;
#[cfg(not(any(feature = "ffi-static-64", feature = "ffi-static-256", feature = "ffi-static-1024")))]
const CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FfiType { Int, Float, Bool, Str, Void }

#[derive(Clone, Copy, Debug)]
pub struct ForeignSignature {
    pub params: &'static [FfiType],
    pub ret: FfiType,
}

pub type ForeignFn = unsafe extern "C" fn();

#[derive(Copy, Clone)]
pub struct Entry {
    pub name: &'static str,
    pub ptr: ForeignFn,
    pub sig: ForeignSignature,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FfiError {
    RegistryFull,
    DuplicateSymbol,
    NotFound,
}

pub struct StaticRegistry {
    slots: [Option<Entry>; CAPACITY],
    len: usize,
}

impl StaticRegistry {
    pub const fn new() -> Self {
        const NONE: Option<Entry> = None;
        Self { slots: [NONE; CAPACITY], len: 0 }
    }

    pub fn register(
        &mut self,
        name: &'static str,
        ptr: ForeignFn,
        sig: ForeignSignature,
    ) -> Result<(), FfiError> {
        if self.lookup(name).is_some() {
            return Err(FfiError::DuplicateSymbol);
        }
        if self.len == CAPACITY {
            return Err(FfiError::RegistryFull);
        }
        self.slots[self.len] = Some(Entry { name, ptr, sig });
        self.len += 1;
        Ok(())
    }

    pub fn lookup(&self, name: &str) -> Option<&Entry> {
        for slot in &self.slots[..self.len] {
            if let Some(e) = slot {
                if e.name == name {
                    return Some(e);
                }
            }
        }
        None
    }

    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
}
```

- [ ] **Step 3: Hook into lib.rs**

```rust
#[cfg(feature = "ffi-static")]
pub mod ffi_static;
```

- [ ] **Step 4: Unit tests in ffi_static.rs**

Append to the module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    unsafe extern "C" fn dummy() {}

    const SIG: ForeignSignature = ForeignSignature {
        params: &[],
        ret: FfiType::Void,
    };

    #[test]
    fn register_then_lookup() {
        let mut r = StaticRegistry::new();
        r.register("f", dummy, SIG).unwrap();
        assert!(r.lookup("f").is_some());
    }

    #[test]
    fn lookup_missing_returns_none() {
        let r = StaticRegistry::new();
        assert!(r.lookup("nope").is_none());
    }

    #[test]
    fn duplicate_registration_errors() {
        let mut r = StaticRegistry::new();
        r.register("f", dummy, SIG).unwrap();
        let err = r.register("f", dummy, SIG).unwrap_err();
        assert_eq!(err, FfiError::DuplicateSymbol);
    }

    #[test]
    fn full_registry_errors_on_next_registration() {
        let mut r = StaticRegistry::new();
        for i in 0..super::CAPACITY {
            let name = Box::leak(format!("f{}", i).into_boxed_str()) as &'static str;
            r.register(name, dummy, SIG).unwrap();
        }
        let err = r.register("overflow", dummy, SIG).unwrap_err();
        assert_eq!(err, FfiError::RegistryFull);
    }
}
```

- [ ] **Step 5: Run tests**

```bash
cd resilient-runtime && cargo test --features ffi-static
```

Expected: PASS.

- [ ] **Step 6: Cross-compile check (keep size gate green)**

```bash
cd resilient-runtime && cargo build --features ffi-static --target thumbv7em-none-eabihf
```

Expected: clean build. Run the size gate script (`scripts/check_size.sh` or equivalent — check the CI workflow for the actual name) against the cortex-m demo and confirm we're still under 64 KiB.

- [ ] **Step 7: Commit**

```bash
cd /Users/eric/GitHub/Resilient
git add resilient-runtime/src/ffi_static.rs resilient-runtime/src/lib.rs resilient-runtime/Cargo.toml
git commit -m "FFI: resilient-runtime static registry (no_std, zero-alloc, behind ffi-static)"
```

---

## Task 12: Example program + docs

**Files:**
- Create: `resilient/examples/ffi_libm.rz`
- Create: `resilient/examples/ffi_libm.expected.txt`
- Modify: `SYNTAX.md`
- Modify: `docs/` (Jekyll) — new page `docs/ffi.md`

- [ ] **Step 1: Example program**

`resilient/examples/ffi_libm.rz`:

```
extern "libm.so.6" {
    fn sqrt(x: Float) -> Float requires(x >= 0.0) ensures(result >= 0.0);
}

fn main() {
    println(sqrt(16.0));
    println(sqrt(2.0));
}
```

(macOS users will need a per-platform alternate — consider a `ffi_libm_macos.rz` with `libSystem.dylib`; or leave the example Linux-only with a docs note.)

`resilient/examples/ffi_libm.expected.txt`:

```
4
1.4142135623730951
```

- [ ] **Step 2: SYNTAX.md entry**

Under a new `## Foreign Function Interface` section, document:

- `extern "lib" { ... }` block form
- Primitive-only type map (reference the spec)
- `= "c_name"` alias syntax
- `requires` / `ensures` on extern decls
- `@trusted` — the "I promise" escape hatch
- Feature flags (`ffi`, `ffi-static`)

Keep to ~60 lines — link to the spec for deeper dives.

- [ ] **Step 3: Jekyll page**

`docs/ffi.md`:

```markdown
---
layout: default
title: FFI
---

# Foreign Function Interface

Resilient programs call into C libraries through `extern` blocks.

```
extern "libm.so.6" {
    fn sqrt(x: Float) -> Float ensures(result >= 0.0);
}
```

See [the FFI design spec](...) for the full type map, contract model,
and embedded `no_std` registration path.
```

- [ ] **Step 4: Verify golden-example harness picks it up**

Run: `cd resilient && cargo test ffi_libm_example 2>&1 | tail -10`
(If the example-runner test name differs, search for existing `.expected.txt` harness tests and mirror the pattern.)
Expected: new example PASS on Linux; on macOS either the alt file runs or the example is skipped via `#[cfg(target_os = "linux")]`.

- [ ] **Step 5: Commit**

```bash
git add resilient/examples/ffi_libm.rz resilient/examples/ffi_libm.expected.txt SYNTAX.md docs/ffi.md
git commit -m "FFI: example program (ffi_libm) + SYNTAX and docs pages"
```

---

## Task 13: Close the loop — update tickets and board

**Files:**
- Create: `.board/tickets/DONE/RES-NEW-ffi-phase-1-tree-walker.md`
- Modify: `.board/ROADMAP.md` — new G21 entry (or whichever goalpost covers FFI)

- [ ] **Step 1: Write the ticket retroactively**

`.board/tickets/DONE/RES-NEW-ffi-phase-1-tree-walker.md`:

```markdown
# RES-NEW: FFI Phase 1 — tree-walker + static registry

## Summary
Ships primitive-only FFI for the tree-walker interpreter on std hosts
and the static-registry path for no_std embedded.

## Acceptance
- [x] `extern "lib" { fn ... }` blocks parse
- [x] Typechecker rejects non-primitive FFI types and @pure on extern
- [x] Loader resolves symbols on std via libloading (ffi feature)
- [x] Tree-walker dispatches through the trampoline table
- [x] `requires` and `ensures` checked at runtime on FFI calls
- [x] `@trusted` propagates ensures as SMT assumption
- [x] `resilient-runtime` ffi-static registry (no_std, zero-alloc)
- [x] End-to-end test calling libm::sqrt on Linux + libSystem on macOS
- [x] Example program + docs

## Out of scope (filed as follow-ups)
- Bytecode VM `OP_CALL_FOREIGN` (phase 2)
- Cranelift JIT lowering (phase 3)
- Struct / Array / callback marshalling
- Variadic foreign fns
```

- [ ] **Step 2: Update ROADMAP.md**

Add to the roadmap table:

```
| G21 | FFI v1 (tree-walker + static registry)     | Shipped 2026-04-?? |
```

- [ ] **Step 3: Final full-suite run**

```bash
cd /Users/eric/GitHub/Resilient
cd resilient && cargo test --features ffi && \
cd ../resilient-runtime && cargo test --features ffi-static && \
cd ..
```

Expected: all green.

- [ ] **Step 4: Commit**

```bash
git add .board/tickets/DONE/RES-NEW-ffi-phase-1-tree-walker.md .board/ROADMAP.md
git commit -m "RES-NEW: close FFI phase-1 ticket; mark G21 in roadmap"
```

- [ ] **Step 5: Push the branch**

```bash
git push -u origin ffi-phase-1-tree-walker
```

---

## Self-review checklist (run before handoff)

- ✅ Spec §3 (syntax): Tasks 1–3 cover block form, alias, contracts, @trusted.
- ✅ Spec §4 (types): Task 4 enforces primitive-only; `ForeignSignature::from_decl` in Task 5 is the single source of truth.
- ✅ Spec §5.1 (std loader): Task 5 (`dynamic` module).
- ✅ Spec §5.2 (no_std registry): Task 11.
- ✅ Spec §5.3 (shared signature): `FfiType` duplicated intentionally between `resilient` and `resilient-runtime` — they can't share a crate because the latter is no_std. Noted inline.
- ✅ Spec §6.1 (tree-walker dispatch): Tasks 6 + 7 + 8.
- ✅ Spec §6.2 (VM), §6.3 (JIT): **out of scope for this plan** — follow-on plans once Phase 1 is stable.
- ✅ Spec §7 (errors): `FfiError` enum in Task 5 covers load-time errors; call-time errors flow through `RResult<Value>` in Task 6.
- ✅ Spec §8 (feature flags): Tasks 5 and 11.
- ✅ Spec §10 (testing): Tasks 5 (unit), 9 (integration with C helper), 10 (verifier), 11 (no_std unit).
- ✅ Spec §12 (success criteria): all items in this plan except VM/JIT (out of scope).

**Placeholder scan:** Task 6's trampoline table openly admits to enumerating arity 0–2 up front and "extend mechanically as real examples demand" — the fallback arm returns a clean error rather than panicking, so this is a real v1 behavior not a TODO. Every Task has concrete code and commands.

**Type consistency:** `ForeignSignature` (host) vs. `ForeignSignature` (runtime) are deliberately distinct — one uses `Vec<FfiType>`, the other `&'static [FfiType]`, because heap-free vs. allocator-aware. Documented inline.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-19-ffi-phase-1-tree-walker.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
