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
   # Add resilient/target/release/ to your PATH
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
resilient --typecheck --audit divide.rz
```

`requires` / `ensures` are checked statically; the compiler rejects a call like `divide(10, 0)` at compile time.

---

## Settings

| Setting | Default | Purpose |
|---|---|---|
| `resilient.serverPath` | `resilient` | Path to the `resilient` binary. Point at a dev build when hacking. |
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
