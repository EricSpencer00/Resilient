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
//! ## Why `--jit` doesn't run the shared assertion
//!
//! The Cranelift JIT (`jit_backend.rs`) lowers a narrow, i64-only
//! subset: no builtin calls (`println`, `type_of`, ...), no `while`,
//! no `match`, no `Bool`/`String`/`Bytes` values, and it requires a
//! top-level `return` rather than the `fn main() { ... } main();`
//! idiom every other backend test in this repo uses. Every case seeded
//! here uses `println` for observable output, so every case currently
//! fails to lower at all. That's not a silent gap: `--jit` exits
//! non-zero with a `jit: unsupported: ...` diagnostic. This file
//! encodes that as data ([`JIT_BACKEND_EXCEPTIONS`]) and — under
//! `--features jit` — actively asserts the failure is the clean,
//! documented kind rather than a silent wrong-answer or a panic. See
//! `#3933` (track B-E4, "JIT completeness + honest feature matrix") for
//! the follow-up that narrows this list as JIT lowering grows.
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
];

/// RES-3983: cases where `--jit` is known not to lower the program at
/// all today. Each row names the reason so the table reads as
/// documentation, not just a skip-list. Referenced from `#3933` (track
/// B-E4, "JIT completeness + honest feature matrix").
///
/// Invariant enforced by [`jit_backend_exceptions_cover_every_case`]:
/// every stem in [`CASES`] must appear here until B-E4 lands JIT support
/// for it — at which point it should move out of this table and get
/// its own `--jit`-asserting case (or, if it can share the existing
/// `.expected.txt`, simply be removed from here and covered by a
/// (to-be-written) `interpreter_vm_and_jit_agree` variant).
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
];

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
    // code for every case seeded from STABILITY.md's Stable list.
    let mut failures = Vec::new();
    for stem in CASES {
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
fn jit_backend_exceptions_fail_cleanly_not_silently() {
    // RES-3983: this is the test that actually earns the "documented
    // exception" label — rather than just not asserting parity, it
    // pins that the JIT's refusal is the clean, typed kind (non-zero
    // exit + a `jit:`-prefixed diagnostic on stderr), not a panic and
    // not a silent wrong answer sharing the tree-walker's exit code.
    // If a future JIT change makes one of these start silently
    // "succeeding" with different output, this test turns red instead
    // of the gap staying invisible.
    let mut failures = Vec::new();
    for (stem, reason) in JIT_BACKEND_EXCEPTIONS {
        let output = Command::new(bin())
            .arg("--jit")
            .arg(case_path(stem))
            .output()
            .unwrap_or_else(|e| panic!("failed to spawn rz --jit for {stem}: {e}"));
        let code = output.status.code();
        let stderr = String::from_utf8_lossy(&output.stderr);
        let clean_refusal = code.map(|c| c != 0).unwrap_or(false) && stderr.contains("jit:");
        if !clean_refusal {
            failures.push(format!(
                "--- {stem} ({reason}) ---\n  exit: {code:?}\n  stderr:\n{stderr}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} documented JIT exception(s) did not fail in the expected clean way \
         (non-zero exit + `jit:` diagnostic) — a silent-success or panic regression \
         would otherwise hide here:\n{}",
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
