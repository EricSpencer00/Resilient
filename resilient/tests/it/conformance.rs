//! RES-3983: conformance suite scaffold for `STABILITY.md`'s Stable
//! surface (roadmap track F-E1, `#3933`).
//!
//! The 1.0 gate is: "every Stable bullet in STABILITY.md has a
//! conformance test that all three backends (tree-walker, `--vm`,
//! `--jit`) pass identically — or a documented backend-support matrix
//! for exceptions." Nothing in the existing suite is indexed to that
//! list: `differential.rs` covers an ad hoc example set for
//! tree-walker-vs-VM only, and `examples_golden.rs` only exercises the
//! tree-walker. This file is the first slice that is explicitly scoped
//! to the Stable list and includes `--jit` in the matrix (even though,
//! today, every case lands in [`JIT_BACKEND_EXCEPTIONS`] — see below).
//!
//! ## Case format
//!
//! Each case is a `resilient/tests/conformance/<stem>.rz` +
//! `resilient/tests/conformance/<stem>.expected.txt` pair. The
//! `.expected.txt` is the tree-walker's stdout (the project's oracle
//! backend) and is asserted against **both** the tree-walker and
//! `--vm` — a mismatch either way is a real bug, not a documented
//! exception, because both backends are expected to fully support the
//! Stable surface today.
//!
//! ## Why `--jit` now runs the shared assertion too
//!
//! The Cranelift JIT (`jit_backend.rs`) natively lowers only a narrow,
//! i64-only subset: no builtin calls outside a small allowlist, no
//! `while`, no `match` on non-`i64` scrutinees, no `Bool`/`String`/
//! `Bytes` values, and it requires a top-level `return` rather than the
//! `fn main() { ... } main();` idiom every other backend test in this
//! repo uses. Every case seeded here uses `println` and/or the
//! `fn main()` idiom, so every case currently fails to natively lower
//! at all — that gap is documented as data in
//! [`JIT_BACKEND_EXCEPTIONS`].
//!
//! That native-lowering gap used to mean `--jit` exited non-zero with a
//! `jit: unsupported: ...` diagnostic for every case in this suite. As
//! of `#4019` (roadmap track B-E4, "JIT completeness + honest feature
//! matrix"), the CLI's `--jit` dispatch site transparently falls back
//! to the VM whenever `jit_backend.rs` bails out with an error that's
//! detectable *before* any native code executed (see
//! `JitError::is_precompile()` in `jit_backend.rs` and `run_via_vm` in
//! `lib.rs`) — so `--jit` now produces output identical to the
//! tree-walker for every case in [`CASES`], via native lowering where
//! `jit_backend.rs` supports it and via the VM fallback everywhere
//! else. [`interpreter_and_jit_agree_on_every_conformance_case`] is
//! that shared assertion, now unconditionally exercised (under
//! `--features jit`) instead of being deferred. `JIT_BACKEND_EXCEPTIONS`
//! stays in the file as documentation of *native*-lowering gaps — as
//! B-E4 grows real native JIT support, cases move out of that table,
//! but the parity assertion itself doesn't need to change either way.
//!
//! ## Growing this suite
//!
//! See `docs/CONFORMANCE.md` for the full walkthrough. Short version:
//! add `tests/conformance/<stem>.rz`, generate
//! `tests/conformance/<stem>.expected.txt` by running the tree walker
//! once and eyeballing the output, add `<stem>` to [`CASES`], and — if
//! `--jit` can't run it yet — add a row to [`JIT_BACKEND_EXCEPTIONS`]
//! with a one-line reason.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn conformance_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/conformance")
}

fn case_path(stem: &str) -> PathBuf {
    conformance_dir().join(format!("{stem}.rz"))
}

fn expected_path(stem: &str) -> PathBuf {
    conformance_dir().join(format!("{stem}.expected.txt"))
}

/// One run of the driver: stdout, exit code. Mirrors `differential.rs`'s
/// `Run` — stderr carries the non-deterministic `seed=...` line plus
/// resilience/mutation-score warnings that aren't part of the
/// program-semantics contract this suite asserts on.
struct Run {
    stdout: String,
    code: Option<i32>,
}

fn run_with(stem: &str, extra_arg: Option<&str>) -> Run {
    let mut cmd = Command::new(bin());
    if let Some(arg) = extra_arg {
        cmd.arg(arg);
    }
    let output = cmd
        .arg(case_path(stem))
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn rz for {stem}: {e}"));
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        code: output.status.code(),
    }
}

fn run_interpreter(stem: &str) -> Run {
    run_with(stem, None)
}

fn run_vm(stem: &str) -> Run {
    run_with(stem, Some("--vm"))
}

/// RES-4019 (B-E4): run a case through `--jit`. Since the CLI dispatch
/// site transparently falls back to the VM for every documented
/// native-lowering gap (see [`JIT_BACKEND_EXCEPTIONS`]), this should
/// now behave identically to [`run_interpreter`] for every case in
/// [`CASES`] — either via genuine native JIT lowering or via the
/// fallback.
#[cfg(feature = "jit")]
fn run_jit(stem: &str) -> Run {
    run_with(stem, Some("--jit"))
}

fn normalize(s: &str) -> String {
    s.trim_end_matches(['\n', '\r'])
        .lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// RES-3983: the seeded conformance cases, indexed to `STABILITY.md`'s
/// Stable list. Each stem must have a sibling `.rz` and `.expected.txt`
/// under `tests/conformance/`.
///
/// Deliberately narrow for this scaffold PR — see `#3983` for the
/// tracking issue that absorbs the ~69-issue per-feature conformance
/// cluster (RES-3387–3483 and friends) as follow-up cases grow this
/// list toward full Stable-surface coverage. Left out of this first
/// slice: the volatile-MMIO wrapper block keyword (see STABILITY.md's
/// bullet on that gated block form), `#[interrupt(...)]`, region
/// annotations, and region-polymorphic functions — those need
/// hardware-shaped harnesses (or at least a `resilient-runtime-cortex-m-demo`-
/// style host stub) that's a separate follow-up, not a `.rz` + `--vm` case.
const CASES: &[&str] = &[
    // Stable: "Int (i64)" primitive type + arithmetic/comparison operators.
    "int_arithmetic",
    // Stable: "Float (f64)" primitive type + arithmetic/comparison operators.
    "float_arithmetic",
    // Stable: core syntax `if` / `else` + expression-level `if`.
    "control_flow_if",
    // Stable: core syntax `while` + while loops.
    "control_flow_while",
    // Stable: core syntax `match` + match expressions on primitives.
    "match_primitives",
    // Stable: core syntax `fn` / `return` + function call syntax and the
    // `fn name(type arg, ...)` declaration form.
    "functions_calls",
    // Stable: "String" primitive type + string literal escape syntax
    // (the subset plain strings actually decode today — see the module
    // doc comment and docs/CONFORMANCE.md for the `\xNN`/`\u{NNNN}` gap).
    "string_escapes",
    // Stable: "Bool" and "Bytes" primitive types, including the `\xNN`
    // byte-literal escape (which *is* decoded on the `b"..."` path).
    "bool_bytes_types",
    // RES-4023 (F-E1 content expansion): deeper coverage of Stable
    // bullets the eight seed cases above only touch once each.
    //
    // Stable: core syntax `let` — shadowing (including a type change
    // across shadows), `let mut` + reassignment, block-scoped shadows.
    "let_bindings_and_shadowing",
    // Stable: core syntax `if` / `else` — nested `if`, and `if` used as
    // a statement with no `else` branch.
    "if_else_nesting_and_statement_form",
    // Stable: core syntax `while` + while loops — `break` / `continue`
    // as intrinsic loop-control behavior.
    "while_break_continue",
    // Stable: core syntax `match` + match expressions on primitives —
    // guarded arms and match on the Bool primitive.
    "match_guards_and_bool",
    // Stable: "Bool" primitive type — `&&` / `||` / `!` truth tables
    // and short-circuit evaluation.
    "logical_operators",
    // Stable: "Integer ... arithmetic operators" + "Int (i64)" — signed
    // `/` and `%` edge cases (truncating-toward-zero semantics).
    "int_division_modulo_edge_cases",
    // Stable: "... float arithmetic operators" + "Float (f64)" — IEEE
    // 754 precision (`0.1 + 0.2 != 0.3`) and comparison operators.
    "float_comparison_precision",
    // Stable: "String" primitive type — full `==`/`!=`/`<`/`<=`/`>`/`>=`
    // lexicographic comparison family.
    "string_comparison_ops",
    // Stable: "Bytes" primitive type — mixed `\xNN` + shared escapes in
    // one `b"..."` literal, empty-bytes, `bytes_len`/`byte_at`.
    "bytes_ops_and_escapes",
    // Stable: "Core syntax: fn, return" + function call syntax — mutual
    // recursion between two independently declared functions.
    "function_mutual_recursion",
    // Stable: "Function call syntax and the `fn name(type arg, ...)`
    // declaration form" — a signature mixing Int/Float/Bool/String.
    "function_multi_param_types",
    // Stable: "String/byte literal escape syntax" — empty string, a
    // literal backslash immediately followed by an escape, `len()`.
    "string_escape_edge_cases",
    // Stable: the compile-time-gated MMIO-wrapper block keyword (see
    // STABILITY.md's matching Stable bullet, and this case's own `.rz`
    // source for the literal syntax) — ordinary statements inside the
    // block execute like they would outside it. RES-4024 fixed `--vm`
    // dropping the block body, so this case now runs the normal,
    // unconditional `--vm`/`--jit` parity assertions like every other
    // row here.
    "unsafe_block_basic",
    // Stable: "Region annotation syntax" — `region NAME;`, `&[R] T`,
    // `&mut[R] T`, distinct-region acceptance path.
    "region_annotations",
    // Stable: "Region-polymorphic function syntax" — `fn f<R, S>(...)`
    // called with call-site regions that instantiate R and S distinctly.
    "region_polymorphic_functions",
];

/// RES-3983 / RES-4019: cases where `jit_backend.rs` is known not to
/// natively lower the program at all today. Each row names the reason
/// so the table reads as documentation, not just a skip-list.
/// Referenced from `#3933` (track B-E4, "JIT completeness + honest
/// feature matrix").
///
/// As of `#4019`, a native-lowering gap no longer means `--jit` fails
/// for these cases — the CLI dispatch falls back to the VM and the run
/// still succeeds with tree-walker-identical output (see
/// [`jit_backend_exceptions_fall_back_to_vm_and_match_interpreter`] and
/// the suite-wide [`interpreter_and_jit_agree_on_every_conformance_case`]).
/// This table only tracks the narrower question of native JIT
/// compilation coverage.
///
/// Invariant enforced by [`jit_backend_exceptions_cover_every_case`]:
/// every stem in [`CASES`] must appear here until B-E4 lands *native*
/// JIT support for it — at which point it should move out of this
/// table (the parity assertions above don't need to change, since they
/// already cover both the native-success and fallback paths).
const JIT_BACKEND_EXCEPTIONS: &[(&str, &str)] = &[
    (
        "int_arithmetic",
        "uses println()/type_of() builtin calls — jit_backend.rs has no builtin-call lowering",
    ),
    (
        "float_arithmetic",
        "uses println()/type_of() builtin calls — jit_backend.rs has no builtin-call lowering",
    ),
    (
        "control_flow_if",
        "uses println() and fn main(){...} main(); — jit_backend.rs requires a top-level return, not builtin calls",
    ),
    (
        "control_flow_while",
        "uses while — jit_backend.rs's has_disqualifying_construct() rejects Node::WhileStatement",
    ),
    (
        "match_primitives",
        "uses match — jit_backend.rs's has_disqualifying_construct() rejects Node::Match",
    ),
    (
        "functions_calls",
        "uses println() — jit_backend.rs has no builtin-call lowering",
    ),
    (
        "string_escapes",
        "String values are entirely outside jit_backend.rs's i64-only value model",
    ),
    (
        "bool_bytes_types",
        "Bytes values and println()/type_of() calls are outside jit_backend.rs's i64-only, builtin-call-free subset",
    ),
    // RES-4023 (F-E1 content expansion): every new case below also uses
    // println()/fn main(){...} main(); (a CallExpression, one of the
    // constructs has_disqualifying_construct() rejects outright), so
    // none natively lower today either. Reasons name the specific
    // additional disqualifying construct or value type each case adds
    // on top of that shared gap.
    (
        "let_bindings_and_shadowing",
        "a `let` shadow changes value type to String mid-function, and uses println() — String values and builtin calls are outside jit_backend.rs's i64-only, builtin-call-free subset",
    ),
    (
        "if_else_nesting_and_statement_form",
        "uses println() and fn main(){...} main(); — jit_backend.rs has no builtin-call lowering",
    ),
    (
        "while_break_continue",
        "uses while/break/continue — jit_backend.rs's has_disqualifying_construct() rejects Node::WhileStatement",
    ),
    (
        "match_guards_and_bool",
        "uses match (with guards) — jit_backend.rs's has_disqualifying_construct() rejects Node::Match",
    ),
    (
        "logical_operators",
        "Bool values and println() calls are outside jit_backend.rs's i64-only, builtin-call-free subset",
    ),
    (
        "int_division_modulo_edge_cases",
        "uses println() — jit_backend.rs has no builtin-call lowering",
    ),
    (
        "float_comparison_precision",
        "Float values and println() calls are outside jit_backend.rs's i64-only, builtin-call-free subset",
    ),
    (
        "string_comparison_ops",
        "String values are entirely outside jit_backend.rs's i64-only value model",
    ),
    (
        "bytes_ops_and_escapes",
        "Bytes values and println()/bytes_len()/byte_at()/type_of() calls are outside jit_backend.rs's i64-only, builtin-call-free subset",
    ),
    (
        "function_mutual_recursion",
        "Bool return values and println() calls are outside jit_backend.rs's i64-only, builtin-call-free subset",
    ),
    (
        "function_multi_param_types",
        "Float/Bool/String parameter types and println() calls are outside jit_backend.rs's i64-only, builtin-call-free subset",
    ),
    (
        "string_escape_edge_cases",
        "String values and println()/len() calls are outside jit_backend.rs's i64-only, builtin-call-free subset",
    ),
    (
        "unsafe_block_basic",
        "uses the MMIO-wrapper block keyword (see this case's .rz source) and println() — outside jit_backend.rs's i64-only, builtin-call-free subset (no native lowering exists for that wrapper or the volatile intrinsics it gates)",
    ),
    (
        "region_annotations",
        "uses region-annotated reference parameters (`&mut[R] T` / `&[R] T`) and println() — jit_backend.rs's i64-only ABI has no lowering for reference/region parameter types",
    ),
    (
        "region_polymorphic_functions",
        "uses region-polymorphic generics (`fn f<R, S>(...)`) with reference parameters, plus println() — outside jit_backend.rs's i64-only, builtin-call-free subset",
    ),
];

/// RES-4023 (F-E1): cases where `--vm` is known to diverge from the
/// tree-walker oracle on genuinely Stable surface. Each row names the
/// filed bug so this reads as documentation, not a silent skip — see
/// the module-level "Growing this suite" note and
/// `docs/CONFORMANCE.md` for the same convention already established
/// for [`JIT_BACKEND_EXCEPTIONS`].
///
/// Empty as of RES-4024: the sole row (`unsafe_block_basic` — `--vm`
/// dropped the entire body of the MMIO-wrapper block, see
/// `resilient/src/compiler.rs`'s `compile_stmt`/`compile_stmt_in_fn`)
/// was fixed and removed. That case now runs the normal, unconditional
/// `--vm`/`--jit` parity assertions like every other row in [`CASES`].
///
/// Invariant: every stem here must also appear in [`CASES`].
const VM_BACKEND_EXCEPTIONS: &[(&str, &str)] = &[];

#[test]
fn conformance_cases_exist_on_disk() {
    for stem in CASES {
        let rz = case_path(stem);
        assert!(
            rz.exists(),
            "CASES references missing file: {}",
            rz.display()
        );
        let expected = expected_path(stem);
        assert!(
            expected.exists(),
            "CASES entry `{stem}` has no sibling .expected.txt: {}",
            expected.display()
        );
    }
}

#[test]
fn at_least_six_stable_surface_cases_are_seeded() {
    // Ticket acceptance criterion: seed ~6-10 core Stable features for
    // this scaffold PR. Pin the floor so a future cleanup can't
    // silently shrink the matrix back toward zero.
    assert!(
        CASES.len() >= 6,
        "conformance matrix has only {} case(s) — F-E1 scaffold requires \u{2265} 6",
        CASES.len()
    );
}

#[test]
fn interpreter_matches_golden_for_every_case() {
    let mut failures = Vec::new();
    for stem in CASES {
        let expected = std::fs::read_to_string(expected_path(stem))
            .unwrap_or_else(|e| panic!("reading expected file for {stem}: {e}"));
        let run = run_interpreter(stem);
        let (e, a) = (normalize(&expected), normalize(&run.stdout));
        if e != a {
            failures.push(format!("--- {stem} ---\n  expected:\n{e}\n  actual:\n{a}"));
        }
        assert_eq!(
            run.code,
            Some(0),
            "{stem}: tree-walker must exit 0 for a Stable-surface conformance case"
        );
    }
    assert!(
        failures.is_empty(),
        "{} case(s) diverged from their golden file:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn interpreter_and_vm_agree_on_every_conformance_case() {
    // The core F-E1 assertion: tree-walker and `--vm` must produce
    // byte-identical (post-normalization) stdout and the same exit
    // code for every case seeded from STABILITY.md's Stable list —
    // except the documented, individually-ticketed rows in
    // [`VM_BACKEND_EXCEPTIONS`] (see
    // [`vm_backend_exceptions_reproduce_their_documented_divergence`]
    // for the assertion that covers those instead).
    let vm_exception_stems: Vec<&str> = VM_BACKEND_EXCEPTIONS.iter().map(|(s, _)| *s).collect();
    let mut failures = Vec::new();
    for stem in CASES {
        if vm_exception_stems.contains(stem) {
            continue;
        }
        let interp = run_interpreter(stem);
        let vm = run_vm(stem);
        let (i, v) = (normalize(&interp.stdout), normalize(&vm.stdout));
        if i != v || interp.code != vm.code {
            failures.push(format!(
                "--- {stem} ---\n  interpreter (exit {:?}):\n{i}\n  vm (exit {:?}):\n{v}",
                interp.code, vm.code
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} case(s) diverged between tree-walker and --vm:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn vm_backend_exceptions_cover_every_documented_divergence() {
    // Mirrors `jit_backend_exceptions_cover_every_case`'s discipline for
    // the (much smaller, hopefully-empty-over-time) VM-divergence table:
    // every stem referenced there must still be a real case, so a future
    // cleanup that renames/removes a case can't leave a stale row behind.
    for (stem, _) in VM_BACKEND_EXCEPTIONS {
        assert!(
            CASES.contains(stem),
            "VM_BACKEND_EXCEPTIONS references `{stem}` which isn't in CASES — stale entry"
        );
    }
}

#[test]
fn vm_backend_exceptions_reproduce_their_documented_divergence() {
    // Pins the *current, known-buggy* `--vm` behavior for each
    // documented exception so a silent regression (the divergence
    // getting worse, or the VM crashing instead of just being wrong)
    // is still caught, while a genuine fix (the row's ticket landing)
    // is expected to fail this test — at which point the row should be
    // deleted from `VM_BACKEND_EXCEPTIONS`, not have its assertion
    // loosened. See #4024 for the tracked fix.
    for (stem, reason) in VM_BACKEND_EXCEPTIONS {
        let interp = run_interpreter(stem);
        let vm = run_vm(stem);
        let (i, v) = (normalize(&interp.stdout), normalize(&vm.stdout));
        assert_ne!(
            i, v,
            "--- {stem} ({reason}) ---\n`--vm` now matches the tree-walker — the documented \
             divergence appears fixed. Remove this row from VM_BACKEND_EXCEPTIONS instead of \
             leaving it stale, and let this case rejoin the unconditional parity assertions."
        );
        assert_eq!(
            vm.code,
            Some(0),
            "{stem}: --vm must still exit 0 (wrong-but-running), not crash, for this documented exception"
        );
    }
}

#[test]
fn jit_backend_exceptions_cover_every_case() {
    // Enforce the "don't silently skip" rule: every seeded case must
    // either have a documented JIT exception, or (once B-E4 grows JIT
    // coverage) a real `--jit` assertion elsewhere in this file. Today
    // that second option is empty, so the two lists must match exactly.
    let exception_stems: Vec<&str> = JIT_BACKEND_EXCEPTIONS.iter().map(|(s, _)| *s).collect();
    for stem in CASES {
        assert!(
            exception_stems.contains(stem),
            "`{stem}` is in CASES but has no BACKEND_EXCEPTIONS row and no --jit \
             assertion — either JIT genuinely supports it now (add a case) or it \
             needs a documented exception (add a JIT_BACKEND_EXCEPTIONS row)"
        );
    }
    for (stem, _) in JIT_BACKEND_EXCEPTIONS {
        assert!(
            CASES.contains(stem),
            "JIT_BACKEND_EXCEPTIONS references `{stem}` which isn't in CASES — stale entry"
        );
    }
}

#[test]
#[cfg(feature = "jit")]
fn jit_backend_exceptions_fall_back_to_vm_and_match_interpreter() {
    // RES-4019 (B-E4): this test used to pin the *old* `--jit`
    // contract — every documented native-lowering gap had to exit
    // non-zero with a `jit:`-prefixed diagnostic. That contract was
    // exactly the bug B-E4 exists to fix: a program using any
    // construct `jit_backend.rs` can't natively lower (println,
    // `while`, `match`, `String`/`Bytes` values, the `fn main(){}
    // main();` idiom, ...) used to hard-fail `--jit` instead of
    // running correctly.
    //
    // Test changes (RES-4019): `JitError::is_precompile()` now lets the
    // CLI driver detect these "never started executing" errors and
    // transparently retry on the VM (`run_via_vm` in `lib.rs`), so
    // every documented exception must now exit 0 with stdout matching
    // the tree-walker's golden file — the same bar
    // `interpreter_and_jit_agree_on_every_conformance_case` holds the
    // whole `CASES` list to, scoped here to the native-lowering-gap
    // subset so the `reason` column stays load-bearing documentation
    // instead of dead prose. A silent-wrong-answer or panic regression
    // still turns this test red.
    //
    // RES-4023: stems in `VM_BACKEND_EXCEPTIONS` are skipped here too —
    // see `vm_backend_exceptions_reproduce_their_documented_divergence`
    // for the assertion that covers the fallback's known-wrong output
    // for those instead.
    let vm_exception_stems: Vec<&str> = VM_BACKEND_EXCEPTIONS.iter().map(|(s, _)| *s).collect();
    let mut failures = Vec::new();
    for (stem, reason) in JIT_BACKEND_EXCEPTIONS {
        if vm_exception_stems.contains(stem) {
            continue;
        }
        let expected = std::fs::read_to_string(expected_path(stem))
            .unwrap_or_else(|e| panic!("reading expected file for {stem}: {e}"));
        let jit = run_jit(stem);
        let (e, a) = (normalize(&expected), normalize(&jit.stdout));
        if e != a || jit.code != Some(0) {
            failures.push(format!(
                "--- {stem} ({reason}) ---\n  exit: {:?}\n  expected:\n{e}\n  actual:\n{a}",
                jit.code
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} documented JIT native-lowering exception(s) did not cleanly fall back to a \
         correct VM run (exit 0, stdout matching the tree-walker golden):\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
#[cfg(feature = "jit")]
fn interpreter_and_jit_agree_on_every_conformance_case() {
    // RES-4019 (B-E4): the core F-E1 assertion for the third backend —
    // `--jit` and the tree-walker must produce byte-identical
    // (post-normalization) stdout and the same exit code for every
    // case seeded from STABILITY.md's Stable list, whether `--jit`
    // gets there via genuine native lowering or via the transparent VM
    // fallback (`JitError::is_precompile()` in `jit_backend.rs`,
    // `run_via_vm` in `lib.rs`). This subsumes
    // `jit_backend_exceptions_fall_back_to_vm_and_match_interpreter`
    // for the exception subset and additionally covers any case that
    // *does* natively JIT-lower today or in the future — no separate
    // per-case `--jit` assertion is needed as B-E4 grows native
    // coverage, since this test already treats native success and
    // fallback success as equally correct.
    //
    // RES-4023: stems in `VM_BACKEND_EXCEPTIONS` are skipped here too —
    // every case in this suite fails native lowering (see
    // `JIT_BACKEND_EXCEPTIONS`), so `--jit` always takes the VM-fallback
    // path here, which means it inherits the VM's documented bug rather
    // than producing a different, more-correct answer.
    let vm_exception_stems: Vec<&str> = VM_BACKEND_EXCEPTIONS.iter().map(|(s, _)| *s).collect();
    let mut failures = Vec::new();
    for stem in CASES {
        if vm_exception_stems.contains(stem) {
            continue;
        }
        let interp = run_interpreter(stem);
        let jit = run_jit(stem);
        let (i, j) = (normalize(&interp.stdout), normalize(&jit.stdout));
        if i != j || interp.code != jit.code {
            failures.push(format!(
                "--- {stem} ---\n  interpreter (exit {:?}):\n{i}\n  jit (exit {:?}):\n{j}",
                interp.code, jit.code
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} case(s) diverged between tree-walker and --jit (native lowering or VM \
         fallback):\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn every_case_file_carries_a_stability_reference() {
    // Cheap doc-discipline check: each `.rz` case should say *which*
    // Stable bullet it's pinning, so the mapping back to STABILITY.md
    // stays legible as the suite grows past this scaffold.
    let mut missing = Vec::new();
    for stem in CASES {
        let src = std::fs::read_to_string(case_path(stem))
            .unwrap_or_else(|e| panic!("reading case source for {stem}: {e}"));
        if !src.contains("Stable") {
            missing.push(*stem);
        }
    }
    assert!(
        missing.is_empty(),
        "case(s) missing a `Stable` cross-reference comment: {}",
        missing.join(", ")
    );
}

#[test]
fn conformance_dir_has_no_orphaned_files() {
    // Catch copy-paste leftovers: every `.rz`/`.expected.txt` under
    // tests/conformance/ should be reachable from CASES, or a future
    // cleanup will silently stop testing it.
    let mut orphans = Vec::new();
    for entry in std::fs::read_dir(conformance_dir())
        .expect("reading tests/conformance/")
        .filter_map(Result::ok)
    {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let stem = name
            .strip_suffix(".expected.txt")
            .or_else(|| name.strip_suffix(".rz"));
        if let Some(stem) = stem
            && !CASES.contains(&stem)
        {
            orphans.push(name.to_string());
        }
    }
    assert!(
        orphans.is_empty(),
        "tests/conformance/ has file(s) not referenced by CASES: {}",
        orphans.join(", ")
    );
}

#[test]
fn compare_normalize_trims_trailing_whitespace_only() {
    // Unit-test the comparison primitive itself, mirroring
    // differential.rs's `compare_outputs_*` self-checks — a future
    // refactor that makes `normalize` lenient in the wrong way (e.g.
    // collapsing blank lines) would otherwise go uncaught even though
    // every seeded case happens to still match.
    assert_eq!(normalize("a\nb  \n"), "a\nb");
    assert_ne!(normalize("a\nb\n"), normalize("a\n\nb\n"));
}

/// Sanity: the conformance dir itself resolves to a real path relative
/// to the crate root, independent of the current working directory
/// `cargo test` happens to run tests from.
#[test]
fn conformance_dir_resolves_under_crate_root() {
    let dir = conformance_dir();
    assert!(dir.is_dir(), "{} is not a directory", dir.display());
    assert!(Path::new(&dir).ends_with("tests/conformance"));
}
