# resilient-runtime-loader-demo (RES-3987, D-E1)

Thin `#![no_std]` binary that embeds a committed `.rzbc` blob
(`../resilient-runtime/fixtures/arithmetic_demo.rzbc`) and runs it
via `resilient_runtime::vm::loader::load_and_run` — the reusable
decode-and-execute glue added in `resilient-runtime/src/vm/loader.rs`.
This is the on-device counterpart to that module's host-side tests:
both consume the exact same fixture bytes and expect the exact same
result (`Value::Int(21)`), so a QEMU run and `cargo test` check the
identical program.

Unlike `resilient-runtime-cortex-m-demo/` (a link-proof for the
value layer, not meant to run under QEMU), this binary's `memory.x`
targets QEMU's `lm3s6965evb` machine on purpose — it is the target
`docs/EMBEDDED_PIPELINE.md` section 4 names for the follow-up
`embedded-runtime.yml` QEMU CI job (D-E1 item 4). That CI wiring is
not part of this crate; this crate is the binary that job will run.

## Target class

- **CPU**: ARM Cortex-M4 (QEMU `lm3s6965evb` — no FPU, unlike the
  M4F demo crate; the VM only uses `i64`/`f64` software arithmetic
  so this doesn't matter).
- **Rust target triple**: `thumbv7em-none-eabihf`.

## Building

```sh
scripts/build_loader_demo.sh
```

Installs the `thumbv7em-none-eabihf` toolchain (idempotent) and runs
`cargo build --release --target thumbv7em-none-eabihf` +
`cargo clippy --release --target thumbv7em-none-eabihf -- -D warnings`.

## Running under QEMU (manual, until the CI job lands)

```sh
cargo build --release --target thumbv7em-none-eabihf
qemu-system-arm -M lm3s6965evb -cpu cortex-m4 -nographic \
  -semihosting-config enable=on,target=native \
  -kernel target/thumbv7em-none-eabihf/release/resilient-runtime-loader-demo
```

Expected output: `loader ok: Int(21)` on the semihosting channel,
then the process exits via `debug::exit(EXIT_SUCCESS)`.

## See also

`../resilient-runtime/src/vm/loader.rs` — the `load_and_run`
function and its host-side tests. `../docs/EMBEDDED_PIPELINE.md` —
the design doc this binary implements section 3.3 of.
