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

## The `BACKEND_EXCEPTIONS` tables

`conformance.rs` keeps three parallel tables:

- `CASES` — the seeded case stems.
- `JIT_BACKEND_EXCEPTIONS` — `(stem, reason)` rows documenting every case
  `jit_backend.rs` cannot **natively lower** today.
- `VM_BACKEND_EXCEPTIONS` — `(stem, reason)` rows documenting cases where
  `--vm` genuinely diverges from the tree-walker oracle on Stable
  surface, each naming the filed bug ticket. See "A real `--vm` bug this
  suite found" below — today this table has exactly one row.

A test (`jit_backend_exceptions_cover_every_case`) enforces that `CASES`
and `JIT_BACKEND_EXCEPTIONS` describe exactly the same set of stems:
every case's native-JIT status is either provably exercised or
explicitly, individually documented with a stated reason. Nothing is
silently skipped. `VM_BACKEND_EXCEPTIONS` is checked more loosely
(`vm_backend_exceptions_cover_every_documented_divergence` just asserts
every row references a real case) since — unlike native JIT lowering,
which is expected to stay partial for a while — the goal is for this
table to be empty; a case only lands here when a real bug is filed
against it.

**Today every seeded case is a native-JIT exception, but all of them pass
`--jit` anyway** (modulo the one documented `VM_BACKEND_EXCEPTIONS` row,
which `--jit` inherits via its VM-fallback path — see below). Every case
uses `println`/`type_of`/other builtins for observable output and the
`fn main() { ... } main();` idiom; `resilient/src/jit_backend.rs`
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

## A real `--vm` bug this suite found

Expanding coverage to the previously-unseeded **`unsafe` blocks** Stable
bullet (`unsafe_block_basic.rz`) surfaced a genuine backend-parity bug,
not just a doc/reality gap: `--vm` drops the entire body of an
`unsafe { ... }` block instead of executing its statements. A minimal
repro —

```resilient
fn main() {
    let mut x = 0;
    unsafe {
        x = 42;
    }
    println(x);
}
main();
```

— prints `42` on the tree-walker (correct) and `0` on `--vm` (wrong; it
behaves as if the whole block evaluated to a constant `0` without
running `x = 42`). This reproduces independently against
`resilient/examples/unsafe_block_smoke.rz`, which is not part of this
suite, so it isn't an artifact of the new case's specific shape.

`unsafe` blocks are listed **Stable** in STABILITY.md, so this is a real
gap on surface the project has committed to, not experimental territory.
It's filed as
[#4024](https://github.com/EricSpencer00/Resilient/issues/4024) and
fixing it means editing `resilient/src/vm.rs` / `resilient/src/compiler.rs`
— out of scope for a conformance-suite-content ticket (RES-4023), whose
file ownership is limited to `tests/conformance/`, `tests/it/conformance.rs`,
and this doc. Rather than silently drop the case or weaken the suite's
parity assertion, `unsafe_block_basic` is listed in `VM_BACKEND_EXCEPTIONS`
(and, transitively, skipped by the `--jit` parity assertion too, since
`--jit`'s VM-fallback path inherits the same bug) with a test
(`vm_backend_exceptions_reproduce_their_documented_divergence`) that pins
the *current* wrong-but-non-crashing behavior — so a further regression
(e.g. a panic) still fails CI, and a real fix is expected to flip that
test red as a signal to delete the row, not weaken it.

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

## What this suite covers today

23 cases (RES-3983's original 8 plus RES-4023's 15) cover every bullet in
`STABILITY.md`'s Stable list with at least one passing case, most with
several: core syntax (`let`/`fn`/`if`-`else`/`while`/`match`/`return`,
including shadowing, nesting, `break`/`continue`, and match guards), the
`Int`/`Float`/`Bool`/`String`/`Bytes` primitive types, arithmetic and
comparison operators (including negative-operand `/`/`%` truncation and
float precision), boolean logic (`&&`/`||`/`!` with short-circuit
evaluation), function call syntax (including mutual recursion and
mixed-type parameter lists), the string/byte escape subset described
above, `unsafe` blocks, and both region annotation syntax and
region-polymorphic function syntax.

`#[interrupt(name = "…")]` is the one remaining Stable bullet with **no**
implementation to test against at all — see
[#4025](https://github.com/EricSpencer00/Resilient/issues/4025), filed
during RES-4023, for the doc/reality drift: STABILITY.md and SYNTAX.md
both describe it as implemented and lowering to a
`resilient-runtime-cortex-m-demo` vector table, but no `.rs` source file
references `__resilient_isr` or registers the attribute — the parser
rejects it outright (`unknown attribute #[interrupt]`). That's a
feature-completeness gap to fix in the compiler, not a conformance-suite
gap to paper over with a case that can't pass.

Each case only asserts the success path. Negative cases (a construct that
must fail to *compile* — e.g. the region-aliasing rejection rule, or the
region-polymorphic call-site aliasing check) live in
`resilient/examples/` (`region_aliasing_err.rz`, `region_poly_call.rz`)
under `examples_smoke.rs`'s expected-failure convention, not here — this
suite's contract is specifically "does the success path produce
identical output across backends."

The ~69-issue per-feature conformance cluster (RES-3387–3483 and related
tickets) is further implementation content that can still deepen this
suite (more operators, more builtin coverage, more match-pattern shapes,
etc.) over time; #3983 is the umbrella that absorbs it.
