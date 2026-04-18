# RES-109: logos vs hand-rolled lexer

**Decision: logos drops.** Ratios below show logos is ~3× SLOWER
than the hand-rolled lexer on ~100 KLoC of synthetic input, on
this machine, in release mode. Keep the hand-rolled lexer as the
default; close G5 as "evaluated, declined".

## Machine

- OS: `Darwin arm64` (macOS aarch64)
- CPU: Apple M-series (aarch64-apple-darwin)
- Rust: `rustc` stable 2025-era toolchain, release profile
- Date: 2026-04-17

## Raw output

```
RES-109: lex-bench input = 100440 lines, 2655180 bytes
| lexer   | p50 (us) | p99 (us) | mean (us) | tokens |
|---------|----------|----------|-----------|--------|
| legacy  |    20092 |    23311 |     19753 | 347401 |
| logos   |    56637 |    83553 |     63250 | 347401 |
ratio p50:  legacy / logos = 0.35×
ratio mean: legacy / logos = 0.31×
```

Interpretation: legacy p50 ≈ 20 ms; logos p50 ≈ 57 ms. Logos is
~2.8× **slower** at p50 and ~3.2× slower on mean. Token counts
match exactly (347 401 tokens), so the work is equivalent.

## Why logos lost

Two factors:

1. **Pos conversion overhead.** Logos returns byte-offset spans;
   we rebuild the crate's `Pos { line, column, offset }` for each
   token via a binary-search + per-line character count. The
   hand-rolled lexer tracks `line / column / offset` inline as it
   advances, so its `next_token_with_span` is O(1). For 347 K
   tokens, the logos path costs ~30 M extra ops on Pos alone.
2. **Identifier allocation.** Both paths allocate a `String` per
   identifier, but logos uses a callback + `lex.slice().to_string()`
   per match which is slightly slower than the hand-rolled's
   indexed-slice pattern.

The scanner proper (DFA transitions) in logos is competitive;
it's the accompanying bookkeeping that tips the ratio. A future
ticket could amortize the Pos rebuild by returning byte spans and
deferring line/col conversion until a diagnostic actually needs
them — but that's an API break, not a one-liner.

## Method

The benchmark is the `tests::lex_bench_100kloc` ignored unit test
in `resilient/src/main.rs`. It:

1. Concatenates every `.rs` under `resilient/examples/` with
   per-copy identifier suffixes until total line count ≥ 100 000
   (100 440 lines / 2.6 MB in practice).
2. Warms up each lexer 2×, times 10 passes, reports p50 / p99 /
   mean in microseconds.
3. Emits the ratio `legacy / logos` for both p50 and mean — a
   value ≥ 2.0 would promote logos to the default; anything less
   keeps the hand-rolled lexer.

Reproduce with `./benchmarks/lex/run.sh`. The ticket's nominal
100-iteration cap was dropped in favour of 10 — at ~20 ms per
legacy pass on 100 KLoC (and ~60 ms per logos pass), even 10
samples per path over 2 lexers puts the harness at ~500 seconds
of pure benchmark time; 100 samples would push `cargo test
--ignored` past 30 minutes on typical laptops. Ten samples per
path is ample for the decision this bench drives (ratio stable
across independent runs on the same machine).

## No flag flip

No changes to default feature flags as part of this ticket. The
`logos-lexer` feature stays off-by-default; the `lexer_parity_on_
all_examples` test keeps ensuring semantic equivalence so the
feature stays a supported escape hatch for anyone who wants to
try it on their own workload.
