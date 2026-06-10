# Resilient for VS Code

![Resilient logo](icon.png)

First-class VS Code support for [Resilient](https://ericspencer.us/Resilient) — a compiled, contract-driven language for safety-critical embedded systems.

- **Syntax highlighting** via a TextMate grammar for `.rz` files
- **LSP diagnostics** — errors, warnings, and hover types from the compiler
- **Document & workspace symbols** with semantic token refinement
- **One-click run** — press the ▶ button in the editor title bar (or right-click → *Run Resilient File*)

Full tutorial: [ericspencer.us/Resilient/tutorial](https://ericspencer.us/Resilient/tutorial)

---

## Quick Start

1. Install the [Resilient compiler](https://github.com/EricSpencer00/Resilient):
   ```bash
   git clone https://github.com/EricSpencer00/Resilient.git
   cd Resilient/resilient
   cargo build --release
   # Add resilient/target/release/ to your PATH; it contains the rz binary
   ```

2. Create `hello.rz`:
   ```resilient
   fn main() {
       println("Hello, Resilient world!");
   }
   main();
   ```

3. Press **▶** in the editor title bar (or `Ctrl+Shift+P` → *Resilient: Run Resilient File*).

The output appears in the integrated terminal under the **Resilient** panel.

---

## Toy Example — Safe Divide

```resilient
fn divide(int a, int b) -> int
    requires b != 0
    ensures  result * b == a
{
    return a / b;
}

println(divide(10, 2));   // prints: 5
```

Run with:
```bash
rz --typecheck --audit divide.rz
```

`requires` / `ensures` are checked statically; the compiler rejects a call like `divide(10, 0)` at compile time.

---

## What's New in 1.5.1

This release tracks the major language milestone shipped in the Resilient 1.5 compiler wave.

**Actor model (RES-332)**
- `spawn(fn)`, `send(pid, value)`, `receive()` builtins
- Cooperative round-robin scheduler
- Deadlock detection with source-position diagnostics
- `actor_ping_pong.rz` end-to-end example

**Region system (RES-393 / 394 / 395)**
- `region NAME;` declarations and `&[NAME]` / `&mut[NAME]` parameter annotations
- Borrow checker rejects aliased mutable regions at declaration and call site
- Region-polymorphic functions: `fn f<R, S>(&mut[R] int a, &mut[S] int b)` with call-site substitution
- Z3-assisted alias analysis for `requires`-annotated functions

**Sum types (RES-400)** — exhaustiveness checking, match patterns on enum variants, `Option` / `Result` via enum machinery

**Polymorphic Arrays (RES-402)** — `Array<T>` syntax; mixed-element-type literals are a compile-time error under `--typecheck`

**First-class functions (RES-403)** — `fn(int) -> int` type syntax; anonymous function literals; higher-order functions

**Generics (RES-405)** — `fn f<T>(T x)` declarations; substitution machinery; VM monomorphization (`id$Int`, `id$String` specialized chunks); JIT monomorphization

**MMIO / ISR docs (RES-406)** — `unsafe { }` and `#[interrupt]` reference added to SYNTAX.md and STABILITY.md

**Self-hosting parser (RES-379)** — `self-host/parser.rz` PR 1: recursive-descent Pratt parser emitting JSON AST; covers expressions, statements, and fn declarations

*Note: V2 design work is tracked separately and will be scoped independently. This release is purely additive on the V1 surface.*

---

## Settings

| Setting | Default | Purpose |
|---|---|---|
| `resilient.serverPath` | `rz` | Path to the `rz` binary. Defaults to the one on PATH; point at a dev build such as `resilient/target/debug/rz` when hacking. |
| `resilient.serverArgs` | `["--lsp"]` | Arguments passed to the binary when starting the LSP. |
| `resilient.trace.server` | `off` | `off` / `messages` / `verbose` — traces LSP traffic to the output channel. |

---

## Development

```bash
cd vscode-extension
npm install
npm run compile
# Press F5 in VS Code to open the Extension Development Host
# Open any .rz file in the launched window
```

If the LSP fails to start, open the **Resilient LSP** output channel. The most common cause is `resilient.serverPath` pointing at a binary built without `--features lsp`.

---

## Resources

- **Language docs**: [ericspencer.us/Resilient](https://ericspencer.us/Resilient)
- **Tutorial**: [ericspencer.us/Resilient/tutorial](https://ericspencer.us/Resilient/tutorial)
- **Source**: [github.com/EricSpencer00/Resilient](https://github.com/EricSpencer00/Resilient)
- **Issues**: [github.com/EricSpencer00/Resilient/issues](https://github.com/EricSpencer00/Resilient/issues)
