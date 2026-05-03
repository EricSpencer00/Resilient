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

This is the **scaffold** PR for [#160 (RES-368)](https://github.com/EricSpencer00/Resilient/issues/160).
The page round-trip works end to end: load → init WASM → run → render
result. The interpreter integration is **stubbed** — `compile_and_run`
echoes the source with a "scaffold" notice, so the deploy pipeline,
size budget, and UI flow can be verified before the real interpreter
lands. The page surfaces this with a yellow banner so a casual visitor
is not misled.

## What's blocking full integration

The `resilient` crate is currently `[[bin]]`-only (see the comment at
`resilient/Cargo.toml:138` and the `[[bin]] name = "rz"` block above
it). To embed the tree-walker in WASM we need a `[lib]` target that
re-exports at minimum the lexer, parser, type checker, and interpreter
modules. That requires editing `resilient/src/main.rs` to extract
module declarations into a sibling `lib.rs` — currently held by the
`res-333-supervisor-fresh` file claim. Tracked as a follow-up.

A second blocker: several unconditional dependencies in
`resilient/Cargo.toml` use platform APIs that won't compile to
`wasm32-unknown-unknown`:

| Dep | Use | WASM-safe path |
|---|---|---|
| `notify` + `notify-debouncer-mini` | `--watch` mode | gate behind `#[cfg(not(target_arch = "wasm32"))]` |
| `rustyline` | REPL | gate behind `#[cfg(not(target_arch = "wasm32"))]` |
| `rand_core` (getrandom) | cert-key generation | activate the `js` feature on `wasm32` |

The follow-up ticket for the lib refactor should fold these into
either feature gates or `cfg`-gated compilation.

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

### Example pinning

The WASM interpreter is compiled at a specific point in the language's
history. As the language evolves, older examples may break against newer
parser/typechecker rules. To prevent silent failures, the build process
annotates `examples.json` with:

```json
{
  "language_version": "df097e2e702cd5ad264d304610c54ad1b54e5350",
  "language_date": "2026-05-03 00:39:53 -0500",
  "examples": [...]
}
```

The frontend displays this metadata so users see which language snapshot
the examples are pinned to. After a major language change, the
responsible agent should:

1. Rebuild the playground: `playground/build.sh --check-size`
2. Test all examples via the UI
3. Update any examples that break, or file follow-up tickets for parser/typechecker changes
4. Commit the updated `dist/examples.json` alongside the compiler change

## Acceptance criteria status (#160)

| Criterion | State |
|---|---|
| `wasm32-unknown-unknown` target added to CI | ✅ — `.github/workflows/playground.yml` |
| `resilient` binary compiles to WASM via `wasm-bindgen` | 🟡 — scaffold uses a stand-alone `resilient-playground` crate with a stub interpreter; full integration pending the lib refactor described above |
| HTML+JS page at `playground/index.html` with CodeMirror + run button | ✅ — `playground/web/index.html` |
| Run → WASM → stdout in result pane | ✅ — round-trip works; output is a stub message until full integration |
| Examples dropdown pre-loads `resilient/examples/` | ✅ — baked into `dist/examples.json` at build time |
| GitHub Pages deploy on push to main | ✅ — `.github/workflows/playground.yml` deploy job |
| Size gate ≤ 2 MiB gzip | ✅ — `playground/build.sh --check-size` |
