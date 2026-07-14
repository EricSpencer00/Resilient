# Conformance Suite

Tracks roadmap epic **F-E1** (see [ROADMAP.md](../ROADMAP.md) / issue
[#3933](https://github.com/EricSpencer00/Resilient/issues/3933)) and the
umbrella issue [#3983](https://github.com/EricSpencer00/Resilient/issues/3983).

The 1.0 gate this suite exists to satisfy:

> Every Stable bullet in [STABILITY.md](../STABILITY.md) has a conformance
> test that all three backends (tree-walker, `--vm`, `--jit`) pass
> identically — or a documented backend-support matrix for exceptions.

Before this suite, backend parity was checked by
`resilient/tests/it/differential.rs` (an ad hoc example list, tree-walker
vs `--vm` only) and golden output was checked by
`resilient/tests/it/examples_golden.rs` (tree-walker only). Neither is
indexed to `STABILITY.md`'s Stable list, and neither includes `--jit`.
This suite is the first slice that is.

## Where things live

| Path | Purpose |
|---|---|
| `resilient/tests/conformance/<stem>.rz` | One conformance case's source. |
| `resilient/tests/conformance/<stem>.expected.txt` | The tree-walker's stdout for that case — the project's oracle backend. |
| `resilient/tests/it/conformance.rs` | The runner. Registered as `mod conformance;` in `resilient/tests/it/main.rs`, so it's part of the single `it` integration-test binary and runs under a plain `cargo test`. |

There is **no `--conformance` CLI flag**. The runner shells out to the
`rz` binary the same way `differential.rs` and `examples_golden.rs` do
(`Command::new(env!("CARGO_BIN_EXE_rz"))`), which keeps this PR test-only
and avoids `lib.rs` churn. A CLI-visible conformance mode (e.g.
`rz --conformance-report`) is plausible future work once the case list is
large enough that a human wants a standalone report instead of `cargo
test` output — tracked as follow-up under #3983, not built here.

## What each case asserts

For every case in `CASES` (see `conformance.rs`):

1. **Tree-walker matches its `.expected.txt` golden file.** This is the
   oracle assertion — if this fails, the case itself (or the language) is
   broken, independent of backend parity.
2. **Tree-walker and `--vm` produce identical stdout and the same exit
   code.** This is the core F-E1 assertion for the two backends that
   fully support the Stable surface today.
3. **`--jit` and the tree-walker produce identical stdout and the same
   exit code, for every seeded case — including every case where
   `jit_backend.rs` cannot natively lower the program at all.** Since
   [RES-4019](https://github.com/EricSpencer00/Resilient/issues/4019)
   (track **B-E4**), the `--jit` CLI dispatch site transparently falls
   back to the VM whenever the JIT bails out with a
   `JitError::is_precompile()` error — i.e. before any native code has
   executed, so the retry can't duplicate side effects — instead of
   surfacing a hard error. This assertion only runs under `--features
   jit` (mirroring the existing `#[cfg(feature = "jit")]` tests in
   `examples_smoke.rs`), since the default CI build doesn't compile the
   JIT at all.

## The `BACKEND_EXCEPTIONS` table

`conformance.rs` keeps two parallel tables:

- `CASES` — the seeded case stems.
- `JIT_BACKEND_EXCEPTIONS` — `(stem, reason)` rows documenting every case
  `jit_backend.rs` cannot **natively lower** today.

A test (`jit_backend_exceptions_cover_every_case`) enforces that the two
lists describe exactly the same set of stems: every case's native-JIT
status is either provably exercised or explicitly, individually
documented with a stated reason. Nothing is silently skipped.

**Today every seeded case is a native-JIT exception, but all of them pass
`--jit` anyway.** All eight cases use `println`/`type_of` for observable
output and the `fn main() { ... } main();` idiom; `resilient/src/jit_backend.rs`
supports neither natively — it lowers a narrow, `i64`-only subset
(arithmetic, comparisons, `if`/`else`, `let`, direct function calls) that
requires a top-level `return` and has no builtin-call lowering at all for
non-`i64` types, and its `has_disqualifying_construct` check explicitly
rejects `while`, `match`, array literals, and indexing from the
trivial-leaf inliner. That narrow native subset is not a testing gap — it
is the accurately-recorded shape of `jit_backend.rs` today, tracked under
[#3933](https://github.com/EricSpencer00/Resilient/issues/3933) (track
**B-E4**, "JIT completeness + honest feature matrix").

What changed with B-E4's first PR ([RES-4019](https://github.com/EricSpencer00/Resilient/issues/4019)):
the `--jit` CLI dispatch no longer hard-fails when `jit_backend.rs`
returns one of these documented native-lowering gaps — it transparently
retries the same program on the VM (see `JitError::is_precompile()` and
`run_via_vm` in `resilient/src/lib.rs`) and the run succeeds with output
identical to the tree-walker's. So every stem in `JIT_BACKEND_EXCEPTIONS`
is simultaneously: (a) a documented native-lowering gap, and (b) covered
by the `interpreter_and_jit_agree_on_every_conformance_case` parity
assertion, because the CLI-visible behavior of `--jit` is now correct
even where the native compiler bails. As B-E4 lands real native lowering
for a construct, move the corresponding case out of
`JIT_BACKEND_EXCEPTIONS` — the parity assertion doesn't need to change
either way, since it already covers the fallback and native-success paths
identically.

## A known doc/reality gap this suite surfaced

`STABILITY.md`'s Stable list includes "String/byte literal escape syntax
(`\n`, `\t`, `\\`, `\"`, `\xNN`, `\u{NNNN}`)" as a single bullet. In
practice:

- `\n`, `\t`, `\r`, `\\`, `\"` are decoded in both plain string literals
  (`"..."`) and byte literals (`b"..."`), on both the tree-walker and
  `--vm`.
- `\xNN` is decoded in byte literals (confirmed via the existing
  `examples/bytes_and_or_not.rz` example) but is **not** decoded in plain
  string literals — `"\x41"` prints literally as `\x41` on both backends.
- `\u{NNNN}` is likewise not decoded in plain string literals today.

This is a shared limitation (both backends agree), not a backend-parity
bug, so it doesn't block this suite — `string_escapes.rz` only exercises
the escapes plain strings actually decode, and `bool_bytes_types.rz`
exercises `\xNN` on the byte-literal path where it does work. The gap
between the written Stable-surface promise and the tree-walker's
`read_string` (in `resilient/src/lib.rs`) is flagged as a follow-up, not
fixed here — fixing it is a language-semantics change, out of scope for
a test-only conformance scaffold.

## Adding a new case

1. Write `resilient/tests/conformance/<stem>.rz`. Start the file with a
   comment naming the `STABILITY.md` Stable bullet(s) it pins (the
   `every_case_file_carries_a_stability_reference` test just checks the
   word "Stable" appears — keep the reference specific in prose).
2. Generate the golden file by running the tree walker once and
   capturing stdout:
   ```bash
   cargo build --manifest-path resilient/Cargo.toml --locked
   resilient/target/debug/rz resilient/tests/conformance/<stem>.rz \
     > resilient/tests/conformance/<stem>.expected.txt
   ```
   Read it back before committing — the golden file is truth, not a
   rubber stamp.
3. Add `"<stem>"` to `CASES` in `resilient/tests/it/conformance.rs`.
4. Try `--jit` (as of RES-4019 this should now succeed on every case via
   the VM fallback, even ones `jit_backend.rs` can't natively lower):
   ```bash
   cargo build --manifest-path resilient/Cargo.toml --locked --features jit
   resilient/target/debug/rz --jit resilient/tests/conformance/<stem>.rz
   ```
   `interpreter_and_jit_agree_on_every_conformance_case` already asserts
   parity for every stem in `CASES`, so you don't need a bespoke
   assertion just to cover the new case. Only touch
   `JIT_BACKEND_EXCEPTIONS` if you want to document *why* native
   `jit_backend.rs` lowering can't handle the new case yet (which
   disqualifying construct, or which value type is out of the JIT's
   `i64`-only model) — that table is documentation of the native-lowering
   gap, not a gate on whether `--jit` produces correct output.
5. Run `cargo test --locked --test it conformance` (and, if you touched
   the JIT-exception table, also `--features jit`) before pushing.

## What this scaffold does not cover yet

Only ~8 of `STABILITY.md`'s Stable bullets are seeded so far: core syntax
(`let`/`fn`/`if`-`else`/`while`/`match`/`return`), the `Int`/`Float`/
`Bool`/`String`/`Bytes` primitive types, arithmetic/comparison operators,
function call syntax, and the string/byte escape subset described above.

Not yet seeded (needs a hardware-shaped harness, not just a `.rz` +
`--vm` case, so it's a separate follow-up under #3983):

- `unsafe` blocks / volatile MMIO intrinsics
- `#[interrupt(name = "…")]`
- Region annotation syntax (`region`, `&[R] T`, `&mut[R] T`)
- Region-polymorphic function syntax (`fn f<R, S>(...)`)

The ~69-issue per-feature conformance cluster (RES-3387–3483 and related
tickets) is the implementation content that grows this suite toward full
Stable-surface coverage over time; #3983 is the umbrella that absorbs it.
