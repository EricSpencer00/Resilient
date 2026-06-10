# Resilient Playground (RES-368)

Static-site playground for trying Resilient in the browser. Built as a
WASM module (`wasm-bindgen`) plus a thin HTML/JS shell. No server.

```
playground/
├── Cargo.toml         WASM crate manifest (cdylib + rlib)
├── src/lib.rs         compile_and_run(source, _input) -> JSON
├── web/               static page (HTML + CSS + JS)
├── build.sh           wasm-pack build + dist/ assembly + size gate
└── dist/              produced by build.sh; deployed to Pages (gitignored)
```

## Status

The page round-trip works end to end: load → init WASM → run → render
result. `compile_and_run` now calls the real
`resilient::run_program` tree-walker through the compiler crate's
library target and returns a JSON result with stdout, diagnostics,
exit code, duration, and `flavor: "tree-walker"`.

## Current limits

The playground is a browser demo surface, not the full native CLI.
The real tree-walker path is wired, but JIT, FFI, Z3-backed
verification, file I/O, the REPL, and watch mode are intentionally
absent from the WASM build.

The compiler crate already cfg-gates native-only dependencies so this
package can compile to `wasm32-unknown-unknown`:

| Dep | Use | WASM-safe path |
|---|---|---|
| `notify` + `notify-debouncer-mini` | `--watch` mode | gated behind `#[cfg(not(target_arch = "wasm32"))]` |
| `rustyline` | REPL | gated behind `#[cfg(not(target_arch = "wasm32"))]` |
| `rand_core` (getrandom) | cert-key generation | uses the `js` feature on wasm32 |

The `_input` parameter to `compile_and_run(source, _input)` is still
reserved for future stdin-style examples; `input()` calls do not have
useful browser-backed stdin today.

## Local development

```bash
# one-time install
cargo install wasm-pack

# build (no size enforcement) and serve
playground/build.sh
python3 -m http.server -d playground/dist 8080
```

Then open http://localhost:8080 .

## Production build

```bash
playground/build.sh --check-size
```

Runs through `wasm-pack build --release`, bakes
`resilient/examples/*.{rz,res}` into `dist/examples.json`, and rejects
any artifact whose gzipped `.wasm` exceeds 2 MiB. CI runs this same
command on every PR that touches `playground/`.

## Acceptance criteria status (#160)

| Criterion | State |
|---|---|
| `wasm32-unknown-unknown` target added to CI | ✅ — `.github/workflows/playground.yml` |
| `resilient` binary compiles to WASM via `wasm-bindgen` | ✅ — `resilient-playground` builds against the compiler library target and calls the real `resilient::run_program` tree-walker; native CLI-only pieces stay cfg-gated |
| HTML+JS page at `playground/index.html` with CodeMirror + run button | ✅ — `playground/web/index.html` |
| Run → WASM → stdout in result pane | ✅ — round-trip returns stdout, diagnostics, exit code, duration, and `flavor: "tree-walker"`; no stub output remains |
| Examples dropdown pre-loads `resilient/examples/` | ✅ — baked into `dist/examples.json` at build time |
| GitHub Pages deploy on push to main | ✅ — `.github/workflows/playground.yml` deploy job |
| Size gate ≤ 2 MiB gzip | ✅ — `playground/build.sh --check-size` |
