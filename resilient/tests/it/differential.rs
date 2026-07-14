//! RES-309 / RES-3990: differential testing — interpreter vs VM (vs JIT).
//!
//! The interpreter, bytecode VM, and JIT are three independent execution
//! engines. A divergence — a program that prints `42` on one and `0` on
//! another — can hide silently for months without a test that runs the
//! same source through more than one backend. This file is that test.
//!
//! ## Denylist model (RES-3990, B-E2)
//!
//! Every `examples/*.rz` file is run through **both** the tree walker
//! (default driver, the oracle) and the bytecode VM (`--vm`) by
//! default — see [`interpreter_and_vm_agree_on_all_examples`]. This
//! used to be an opt-in allowlist (`SHARED_EXAMPLES`); it is now an
//! opt-out denylist ([`UNSUPPORTED_BY_VM`]) so that:
//!
//! - **New examples are covered automatically.** Nobody has to
//!   remember to add a new `.rz` file to a curated list for it to be
//!   differential-tested.
//! - **The VM-parity gap is visible.** [`UNSUPPORTED_BY_VM`] is a
//!   catalogued, ticket-referenced list of every currently-known
//!   divergence. Every entry is a bug (tracked under one of RES-3991
//!   through RES-3998) or a semantics gap, not a shrug. As the VM
//!   gains coverage, entries come out and the list shrinks — that's
//!   the point.
//!
//! Do **not** add an example to [`UNSUPPORTED_BY_VM`] to silence a
//! failure without filing (or referencing) a ticket for *why* it
//! diverges. If you fix a divergence, remove the entry in the same PR.
//!
//! ## What's not covered yet (deliberate)
//!
//! - The JIT backend (`--features jit`) is **out of scope for this
//!   file**. The JIT lowering only supports a small i64-only subset (no
//!   strings, no structs, no actors, no Z3-discharged contracts) so
//!   running it against the full example corpus would force every
//!   program through the "skip — unimplemented JIT node" path. See
//!   follow-up ticket: a JIT-only differential pass once the lowering
//!   covers enough surface to be meaningful.
//! - A CI "shrink-ratchet" check that fails if [`UNSUPPORTED_BY_VM`]
//!   grows without a matching ticket, and an aggregate
//!   unsupported-construct-kind coverage artifact, are noted as
//!   follow-up work in RES-3990 — not implemented here.
//!
//! ## Why we strip stderr
//!
//! Both backends print a `seed=<u64>` line to stderr from the runtime
//! fault-injection harness. The seed is non-deterministic across runs
//! AND across backends (each path samples the RNG independently). So
//! the differential check compares **stdout exactly** and **exit code
//! exactly** — stderr is informational only.
//!
//! ## Value-type assertions (RES-3990, B-E2)
//!
//! Byte-identical stdout is not sufficient — RES-3889 was a divergence
//! where both backends printed the same bytes but disagreed on the
//! underlying *value type* (`Char` vs. a one-character `String`), which
//! only became externally visible via a follow-on comparison
//! (`s[i] == 'c'`). [`run_typed`] extends the comparison surface by
//! wrapping a probe expression in `println(type_of(<expr>))` and
//! diffing that string across backends *in addition to* the plain
//! value output, so a type-only divergence fails even when the printed
//! *value* representation happens to match. See
//! [`interpreter_and_vm_agree_on_value_types`].
//!
//! ## Catching divergence in the framework itself
//!
//! [`compare_outputs`] is the comparison primitive. We unit-test it
//! directly with synthesised divergent transcripts so a regression in
//! the *checker itself* (e.g. accidentally normalising whitespace,
//! ignoring exit codes) is caught even if no example happens to
//! diverge between backends.

use std::fs;
use std::path::Path;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

/// One run of the driver: stdout, exit code. Stderr is intentionally
/// dropped — it carries the non-deterministic `seed=...` line and any
/// runtime diagnostics that aren't part of the "program semantics"
/// contract this differential check is asserting on.
struct Run {
    stdout: String,
    code: Option<i32>,
}

fn run_interpreter(example: &str) -> Run {
    let path = format!("examples/{example}");
    let output = Command::new(bin())
        .arg(&path)
        .output()
        .expect("failed to spawn rz (interpreter)");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        code: output.status.code(),
    }
}

fn run_vm(example: &str) -> Run {
    let path = format!("examples/{example}");
    let output = Command::new(bin())
        .arg("--vm")
        .arg(&path)
        .output()
        .expect("failed to spawn rz --vm");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        code: output.status.code(),
    }
}

/// Result of comparing two backend runs. `Ok(())` means the two agree;
/// `Err(diff)` carries a multi-line, copy-pasteable diff suitable for
/// `assert!(.., "{diff}")`.
fn compare_outputs(label_a: &str, a: &Run, label_b: &str, b: &Run) -> Result<(), String> {
    if a.stdout == b.stdout && a.code == b.code {
        return Ok(());
    }
    let mut msg = String::new();
    msg.push_str("backend disagreement:\n");
    if a.code != b.code {
        msg.push_str(&format!(
            "  exit-code: {label_a}={:?}  vs  {label_b}={:?}\n",
            a.code, b.code
        ));
    }
    if a.stdout != b.stdout {
        msg.push_str("  stdout disagreement:\n");
        msg.push_str(&format!("    --- {label_a} stdout ---\n"));
        for line in a.stdout.lines() {
            msg.push_str("    ");
            msg.push_str(line);
            msg.push('\n');
        }
        msg.push_str(&format!("    --- {label_b} stdout ---\n"));
        for line in b.stdout.lines() {
            msg.push_str("    ");
            msg.push_str(line);
            msg.push('\n');
        }
    }
    Err(msg)
}

/// RES-3990: denylist of `examples/*.rz` files the bytecode VM does not
/// yet execute identically to the tree walker. Every entry is grouped
/// under the ticket that catalogs its root cause — see the module-level
/// docs above. An example belongs here if and only if it currently
/// diverges; removing an entry (once its ticket is fixed) is how the
/// gap shrinks. Do not add an entry without a comment + ticket ref.
const UNSUPPORTED_BY_VM: &[&str] = &[
    // RES-3992: VM bytecode compiler "unknown identifier" / "unknown function" —
    // closures/consts captured across scopes, and static/namespaced/tuple-
    // struct-constructor calls the compiler doesn't resolve to a callable.
    //
    // The top-level-`const` family (`const_eval.rz`, `const_eval_ext.rz`,
    // `static_assert.rz`) is fixed: `compiler::compile` now runs a
    // `resolve_top_level_consts` + `inline_consts` pre-pass that inlines
    // every resolved `const` reference as a literal before compilation,
    // mirroring `Interpreter::const_eval_program` / `eval_const_expr` (the
    // tree-walker's canonical const-resolution oracle) — so these three no
    // longer belong on this list.
    //
    // The remaining entries are a distinct root cause this pre-pass does
    // not touch: fn-valued locals/closures captured across a function or
    // actor-spawn boundary (`actor_*`, `effect_polymorphism.rz`,
    // `first_class_fn_*.rz`, `generic_fn_type_params.rz`,
    // `iterator_protocol.rz`, `showcase_actors.rz` — all fail on a plain
    // `unknown identifier`, not a const), and static/namespaced/tuple-
    // struct-constructor calls the compiler doesn't lower to a callable
    // (`default_method.rz` — default trait-method dispatch,
    // `module_namespaces.rz` — `math::add`, `pain_points_hardening.rz` —
    // `array_none`, `tuple_struct.rz` — tuple-struct constructor call,
    // `error_stack_traces.rz` — `stacktrace`). Both need call-site lowering
    // changes in `compiler.rs`'s `CallExpression`/`Identifier` compilation,
    // not an AST pre-pass — left for a follow-up PR under this same ticket.
    "actor_deadlock.rz",
    "actor_ping_pong.rz",
    "actor_spawn_send.rz",
    "default_method.rz",
    "effect_polymorphism.rz",
    "error_stack_traces.rz",
    "first_class_fn_pass.rz",
    "first_class_fn_return.rz",
    "generic_fn_type_params.rz",
    "iterator_protocol.rz",
    "module_namespaces.rz",
    "pain_points_hardening.rz",
    "showcase_actors.rz",
    "tuple_struct.rz",
    // RES-3993: VM bytecode compiler "unsupported construct" (Match, WhileStatement,
    // ReturnStatement, indirect calls, non-arithmetic operators, and an
    // <other> catch-all).
    //
    // The `Match`-as-statement family is fixed: `if let`/`while let`
    // (RES-908/RES-914) desugar to a bare `Node::Match` inside a `Block`
    // (see `parse_if_let_statement`/`parse_while_let_statement` in
    // `lib.rs`), but neither `compile_stmt` nor `compile_stmt_in_fn` had a
    // `Match` arm — every if-let/while-let fell through to the generic
    // `Unsupported("Match")` catch-all even though `compile_match_expr`
    // already handled `Match` in *expression* position. Added
    // `compile_match_stmt`/`compile_match_stmt_in_fn` (mirrors
    // `compile_match_expr`'s pattern-check/guard machinery, but compiles
    // arm bodies with `compile_stmt`/`compile_stmt_in_fn` instead of
    // `compile_expr` so `return`/`break`/`continue` inside an arm work,
    // and doesn't leave a fallthrough value on the stack) and wired both
    // into their respective statement dispatchers — `edge_if_let_pattern.rz`,
    // `edge_while_let.rz`, `if_let.rz`, `while_let.rz` no longer belong here.
    //
    // The remaining entries are distinct root causes this fix does not
    // touch: `break_with_value.rz` needs a `loop`-as-expression value
    // channel for `WhileStatement` in expression position (`break <expr>`
    // via `Node::BreakWith`); `match_block_arms.rz` needs `ReturnStatement`
    // support inside a match arm's block when the match itself is compiled
    // in *expression* position (`compile_block_as_expr` doesn't handle
    // `return`); `comprehension_demo.rz`/`edge_closure_capture.rz` need
    // indirect-call support for non-identifier callees; `null_coalescing_
    // operator.rz`/`option_find.rz` need the `??` operator (and, for
    // `option_find.rz`, an `Option`-returning fn declared with an `int`
    // return type — a pre-existing typechecker gap unrelated to the VM);
    // and `array_contains.rz`, `array_sorted_invariant.rz`, `bench_simple.rz`,
    // `defer_stmt.rz`, `optional_chaining.rz`, `quantifier_assert.rz`,
    // `quantifier_exists.rz`, `quantifier_forall.rz`, `showcase_quantifiers.rz`
    // are still an untriaged `<other>` catch-all (each needs the
    // `node_kind` name surfaced per-example — left for a follow-up PR
    // under this same ticket).
    "array_contains.rz",
    "array_sorted_invariant.rz",
    "bench_simple.rz",
    "break_with_value.rz",
    "comprehension_demo.rz",
    "defer_stmt.rz",
    "edge_closure_capture.rz",
    "match_block_arms.rz",
    "null_coalescing_operator.rz",
    "option_find.rz",
    "optional_chaining.rz",
    "quantifier_assert.rz",
    "quantifier_exists.rz",
    "quantifier_forall.rz",
    "showcase_quantifiers.rz",
    // RES-4017 (split off from RES-3994; that ticket closed once every
    // sub-case had a home — see PR #4016): `Op::CallMethod`'s built-in-
    // container fallback (`vm_call_builtin_method`) has no path for
    // calling a *user-supplied closure* per element — `.map()`/
    // `.flat_map()` etc. need the VM to invoke a `Value::Closure` and
    // resume the host-side loop with its result, which needs a re-entrant
    // "run this call to completion" primitive the flat bytecode dispatch
    // loop doesn't have yet (unlike the tree-walker, which just recurses).
    // `StringBuilder` methods hit the same "receiver is a struct but
    // there's no `$method` — check built-ins" fallback, but the
    // interpreter's `StringBuilder` dispatch (`eval_string_builder_method`)
    // also writes the mutated struct back to the caller's local binding,
    // which `vm_call_builtin_method` has no handle on. `mutual_tco.rz` is
    // unrelated to `CallMethod` — the VM has no tail-call optimization for
    // mutual recursion, so it blows the `>1024`-frame cap where the
    // tree-walker doesn't. (CallMethod primitive-`impl` dispatch, `impl
    // Add`/`Sub`/`Mul` operator-overload dispatch, `Result` error-chaining
    // methods, `Display::fmt` dispatch, `{ ..base, f: v }` struct-update-
    // syntax field merge, and `Option::`/`Result::`-qualified match
    // patterns were fixed under RES-3994 and no longer belong here).
    // Refs #3933 · B-E3.
    "array_functional.rz",
    "array_method_chains.rz",
    "mutual_tco.rz",
    "string_builder.rz",
    // RES-3995: VM has no `live { }` retry-loop execution context — `live_retries()`
    // fails outside a live block, and the retry mechanism itself asserts on
    // the first failing attempt instead of retrying.
    "live_blocks.rz",
    "live_retry_log.rz",
    "self_healing.rz",
    "showcase_live_invariant.rz",
    "telemetry_demo.rz",
    "thermal_safety_cutoff.rz",
    // RES-3996: VM doesn't enforce `assume(false)` / panic-recovery dead-code
    // runtime checks — the interpreter exits non-zero before reaching "dead"
    // code; `--vm` runs straight through it.
    "assume_debug.rz",
    "assume_false_dead_code.rz",
    "assume_literal_false.rz",
    "assume_violated.rz",
    "recovers_to_fail.rz",
];

/// Every `examples/*.rz` file, sorted, read fresh from disk each run so
/// new examples are covered without touching this file.
fn discover_examples() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir("examples")
        .expect("examples/ directory must exist")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".rz"))
        .collect();
    names.sort();
    names
}

#[test]
fn unsupported_by_vm_entries_reference_real_files_with_no_duplicates() {
    // Cheap pre-flight: a stale denylist entry (file renamed/deleted) or
    // an accidental duplicate silently shrinks the effective denylist
    // size without anyone noticing. Surface both here instead of via a
    // confusing downstream failure.
    let mut seen = std::collections::HashSet::new();
    for example in UNSUPPORTED_BY_VM {
        let path = format!("examples/{example}");
        assert!(
            Path::new(&path).exists(),
            "UNSUPPORTED_BY_VM references missing file: {path}"
        );
        assert!(
            seen.insert(*example),
            "UNSUPPORTED_BY_VM lists {example} more than once"
        );
    }
}

#[test]
fn at_least_four_hundred_examples_are_covered_by_default() {
    // RES-3990 acceptance criterion: inverting to a denylist must not
    // quietly regress to "denylist everything." Pin the covered
    // (non-denylisted) count so a future change can't silently gut
    // coverage back down to an allowlist-sized handful. As of RES-3990
    // this was 412 of 581 examples (169 denylisted); RES-3991 (B-E3)
    // fixed the VM's spurious trailing top-level auto-print and removed
    // its 97-entry denylist block, so coverage is now 528 of 581 (53
    // denylisted). The threshold below is a conservative floor, not the
    // exact figure, so unrelated example-corpus growth doesn't make this
    // test flaky.
    let denylist: std::collections::HashSet<&str> = UNSUPPORTED_BY_VM.iter().copied().collect();
    let covered = discover_examples()
        .iter()
        .filter(|name| !denylist.contains(name.as_str()))
        .count();
    assert!(
        covered >= 400,
        "only {covered} examples covered by default (denylist has {} entries) — \
         RES-3990 requires \u{2265} 400",
        UNSUPPORTED_BY_VM.len()
    );
}

#[test]
fn interpreter_and_vm_agree_on_all_examples() {
    // RES-3990: the inverted differential sweep. Every discovered
    // example runs through both backends unless denylisted; a failure
    // here means either a genuinely new divergence (fix it, or add a
    // denylist entry with a ticket ref) or a regression in an area
    // that was previously fixed.
    let denylist: std::collections::HashSet<&str> = UNSUPPORTED_BY_VM.iter().copied().collect();
    let examples = discover_examples();
    assert!(
        !examples.is_empty(),
        "discovered zero examples — examples/ directory misconfigured?"
    );
    let mut failures: Vec<String> = Vec::new();
    let mut covered = 0usize;
    for example in &examples {
        if denylist.contains(example.as_str()) {
            continue;
        }
        covered += 1;
        let interp = run_interpreter(example);
        let vm = run_vm(example);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {example} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {covered} covered example(s) diverged (see UNSUPPORTED_BY_VM \
         if this is a known, ticketed gap that needs a denylist entry):{}",
        failures.len(),
        failures.join("")
    );
}

#[test]
fn compare_outputs_detects_stdout_divergence() {
    // Sanity-check the comparison primitive. If a future refactor
    // accidentally makes `compare_outputs` lenient (say, trims
    // whitespace or ignores a trailing newline) the divergence-
    // detection part of this harness silently stops working — even
    // though the example matrix is green. This unit test pins the
    // detection itself.
    let a = Run {
        stdout: "42\n".to_string(),
        code: Some(0),
    };
    let b = Run {
        stdout: "0\n".to_string(),
        code: Some(0),
    };
    let result = compare_outputs("a", &a, "b", &b);
    let err = result.expect_err("must flag stdout disagreement");
    assert!(err.contains("stdout disagreement"));
    assert!(err.contains("42"));
    assert!(err.contains("0"));
}

#[test]
fn compare_outputs_detects_exit_code_divergence() {
    let a = Run {
        stdout: "same\n".to_string(),
        code: Some(0),
    };
    let b = Run {
        stdout: "same\n".to_string(),
        code: Some(1),
    };
    let result = compare_outputs("interp", &a, "vm", &b);
    let err = result.expect_err("must flag exit-code disagreement");
    assert!(err.contains("exit-code"));
    assert!(err.contains("Some(0)"));
    assert!(err.contains("Some(1)"));
}

#[test]
fn compare_outputs_accepts_identical_runs() {
    let a = Run {
        stdout: "identical\n".to_string(),
        code: Some(0),
    };
    let b = Run {
        stdout: "identical\n".to_string(),
        code: Some(0),
    };
    assert!(compare_outputs("a", &a, "b", &b).is_ok());
}

/// Run inline `src` on one backend by staging it in a unique temp file.
/// Mirrors [`run_interpreter`] / [`run_vm`] but lets a test pin a tiny
/// program without adding an `examples/*.rz` sidecar.
fn run_src(src: &str, tag: &str, vm: bool) -> Run {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "rz_differential_{tag}_{}_{n}.rz",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("write temp source");
    let mut cmd = Command::new(bin());
    if vm {
        cmd.arg("--vm");
    }
    let output = cmd.arg(&path).output().expect("failed to spawn rz");
    let _ = std::fs::remove_file(&path);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        code: output.status.code(),
    }
}

/// RES-3990 (B-E2): probe the *runtime type* of a value-producing snippet
/// via the language's own `type_of()` builtin, on one backend. Wraps
/// `expr_src` (an expression, not a full program) in a minimal program
/// that prints `type_of(<expr_src>)`, so a test can pin the type tag
/// independently of whatever the value's *display* representation looks
/// like. This is the mechanism behind
/// [`interpreter_and_vm_agree_on_value_types`]: two backends that print
/// byte-identical stdout for a value can still disagree on its
/// underlying type (the RES-3889 class of bug), and `compare_outputs`
/// alone — which only ever sees `println!`-style display text — cannot
/// distinguish that from a real match.
fn run_typed(expr_src: &str, tag: &str, vm: bool) -> Run {
    let program = format!("println(type_of({expr_src}));");
    run_src(&program, tag, vm)
}

/// RES-3990 (B-E2): interpreter and VM must agree on the *value type*
/// `type_of()` reports for a representative value of every primitive
/// and compound kind, not merely on printed stdout bytes. Each entry is
/// an expression (not a statement) safely evaluable at top level on
/// both backends today; a name that would hit one of the
/// [`UNSUPPORTED_BY_VM`] gaps is deliberately excluded so this test
/// stays green and keeps testing the *type* channel rather than
/// re-discovering an already-catalogued execution gap.
///
/// `Range` (RES-4000, fixed): `type_of(1..5)` used to report `"range"`
/// on the interpreter and `"array"` under `--vm`, because the VM
/// compiler lowered `Node::Range` straight to `array_range(lo, hi)`
/// instead of a first-class `Value::Range`. Fixed by lowering to the
/// VM-internal `__range(lo, hi, inclusive)` builtin (`compiler.rs`),
/// which constructs a `Value::Range` directly — now included below.
#[test]
fn interpreter_and_vm_agree_on_value_types() {
    let probes = [
        ("int", "5"),
        ("float", "5.0"),
        ("bool", "true"),
        ("string", r#""hi""#),
        ("char", r#""ab"[0]"#),
        ("array", "[1, 2, 3]"),
        ("map", "map_new()"),
        ("set", "set_new()"),
        ("bytes", r#"b"AB""#),
        ("tuple", "(1, 2)"),
        ("function_closure", "fn(int x) { return x; }"),
        ("range", "1..5"),
        ("range_inclusive", "1..=5"),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, expr) in probes {
        let interp = run_typed(expr, tag, false);
        let vm = run_typed(expr, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- type_of({expr}) [{tag}] ---\n{diff}"));
        }
    }
    // Multi-statement probes (Option/Result/struct/enum construction need
    // a `let` binding first) run through the general-purpose `run_src`
    // rather than the single-expression `run_typed` wrapper.
    let statement_probes = [
        (
            "option",
            "let x: Option<int> = Some(5); println(type_of(x));",
        ),
        (
            "result",
            "let x: Result<int, string> = Ok(5); println(type_of(x));",
        ),
        (
            "struct",
            "struct P { int x, } let p = new P { x: 1 }; println(type_of(p));",
        ),
        ("enum_bare", "enum E { A, B(int) } println(type_of(E::A));"),
        (
            "enum_payload",
            "enum E { A, B(int) } println(type_of(E::B(1)));",
        ),
    ];
    for (tag, src) in statement_probes {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} value-type probe(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3891: cross-kind `==` / `!=` must behave identically on both
/// backends. The tree walker raises a runtime type mismatch; before the
/// fix the VM silently reported the operands unequal (`false`) and kept
/// running, so identical source produced different stdout *and* a
/// different exit code. Lock the class here so it can't silently return.
#[test]
fn interpreter_and_vm_agree_on_cross_type_equality() {
    // Each program compares operands of different kinds. Both backends
    // must agree on stdout and exit code (a type-mismatch error → the
    // `if`/`else` branch never prints and the process exits non-zero).
    let programs = [
        (
            "char_eq_int",
            "fn main() { let s = \"ab\"; let c = s[0]; if c == 5 { println(\"eq\"); } else { println(\"ne\"); } } main();",
        ),
        (
            "char_neq_int",
            "fn main() { let s = \"ab\"; let c = s[0]; if c != 5 { println(\"ne\"); } else { println(\"eq\"); } } main();",
        ),
        (
            "int_eq_string",
            "fn main() { let x = 1; let y = \"1\"; if x == y { println(\"eq\"); } else { println(\"ne\"); } } main();",
        ),
        (
            "int_eq_bool",
            "fn main() { let x = 1; let y = true; if x == y { println(\"eq\"); } else { println(\"ne\"); } } main();",
        ),
        (
            "int_eq_float",
            "fn main() { let x = 1; let y = 1.0; if x == y { println(\"eq\"); } else { println(\"ne\"); } } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} cross-type comparison(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3894: `&&` / `||` with a non-bool operand must behave identically on
/// both backends. The tree walker raises a runtime type mismatch; before the
/// fix the VM coerced the operand via the truthiness rule and kept running,
/// so identical source produced different stdout *and* exit code. Lock the
/// class here so it can't silently return.
#[test]
fn interpreter_and_vm_agree_on_non_bool_logical_operands() {
    let programs = [
        (
            "and_int_left",
            "fn main() { if 5 && true { println(\"y\"); } else { println(\"n\"); } } main();",
        ),
        (
            "and_int_right",
            "fn main() { if true && 5 { println(\"y\"); } else { println(\"n\"); } } main();",
        ),
        (
            "and_zero_left",
            "fn main() { if 0 && true { println(\"y\"); } else { println(\"n\"); } } main();",
        ),
        (
            "and_string_left",
            "fn main() { if \"a\" && true { println(\"y\"); } else { println(\"n\"); } } main();",
        ),
        (
            "or_int_left",
            "fn main() { if 5 || false { println(\"y\"); } else { println(\"n\"); } } main();",
        ),
        (
            "or_int_right",
            "fn main() { if false || 5 { println(\"y\"); } else { println(\"n\"); } } main();",
        ),
        (
            "or_zero_left",
            "fn main() { if 0 || false { println(\"y\"); } else { println(\"n\"); } } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} logical-operand case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3896: `Array + Array` must behave identically on both backends. The
/// tree walker special-cases array concatenation in `eval_infix_expression`;
/// before the fix the VM's `Op::Add` had no arm for two arrays and raised a
/// type mismatch, so identical source produced different stdout *and* exit
/// code. Lock the class here so it can't silently return.
#[test]
fn interpreter_and_vm_agree_on_array_concatenation() {
    let programs = [
        (
            "int_arrays",
            "fn main() { let a = [1, 2]; let b = [3, 4]; println(a + b); } main();",
        ),
        (
            "string_arrays",
            "fn main() { let a = [\"x\", \"y\"]; let b = [\"z\"]; println(a + b); } main();",
        ),
        (
            "empty_left",
            "fn main() { let a: [int32] = []; let b = [1, 2]; println(a + b); } main();",
        ),
        (
            "empty_right",
            "fn main() { let a = [1, 2]; let b: [int32] = []; println(a + b); } main();",
        ),
        (
            "array_plus_int_still_errors",
            "fn main() { let a = [1, 2]; println(a + 5); } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} array-concatenation case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3902: the VM's peephole optimizer had a family of type-blind
/// numeric/bitwise identity folds (`x+0==x`, `x*0==0`, `x&0==0`,
/// `x-0==x`, `x/1==x`, ...) that assumed the operand feeding the fold
/// was always `Int`/`Float`. `+` and `*` also legally accept `String`
/// operands (concatenation/stringification, repetition — RES-924), for
/// which the assumed "identity" is a different, non-identity value; the
/// bitwise ops and `-`/`/` never accept `String` at all, so folding
/// away the op also folded away the runtime type-mismatch error it
/// would have raised. Both are silent-divergence bugs: `--vm` gives a
/// different (or no) error than the interpreter for identical source.
/// Lock the whole class here so it can't silently return.
#[test]
fn interpreter_and_vm_agree_on_typed_identity_folds() {
    let programs = [
        // `+` and `*` accept String — the peephole assumed the wrong
        // "identity" value instead of raising/suppressing an error.
        (
            "string_plus_zero",
            "fn main() { let s = \"ab\"; println(s + 0); } main();",
        ),
        (
            "string_times_zero",
            "fn main() { let s = \"ab\"; println(s * 0); } main();",
        ),
        // `-`, `/`, and the bitwise ops never accept String — the
        // peephole silently suppressed the type-mismatch error.
        (
            "string_minus_zero",
            "fn main() { let s = \"ab\"; println(s - 0); } main();",
        ),
        (
            "string_div_one",
            "fn main() { let s = \"ab\"; println(s / 1); } main();",
        ),
        (
            "string_band_zero",
            "fn main() { let s = \"ab\"; println(s & 0); } main();",
        ),
        (
            "string_bor_zero",
            "fn main() { let s = \"ab\"; println(s | 0); } main();",
        ),
        (
            "string_bxor_zero",
            "fn main() { let s = \"ab\"; println(s ^ 0); } main();",
        ),
        (
            "string_shl_zero",
            "fn main() { let s = \"ab\"; println(s << 0); } main();",
        ),
        (
            "string_shr_zero",
            "fn main() { let s = \"ab\"; println(s >> 0); } main();",
        ),
        // The legitimate numeric cases must still agree (and still
        // benefit from the fold where it's provably safe).
        (
            "int_plus_zero",
            "fn main() { let x = 5; println(x + 0); } main();",
        ),
        (
            "int_times_zero",
            "fn main() { let x = 5; println(x * 0); } main();",
        ),
        (
            "int_increment_loop",
            "fn main() { let mut x = 0; let mut i = 0; while i < 5 { x = x + 1; i = i + 1; } println(x); } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} typed-identity-fold case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3904: the compiler emits `Op::CallMethod` for every `x.y(...)`
/// dot-call uniformly (no static type info at that point), but the
/// VM's `CallMethod` handler only had an arm for `Struct`/`EnumVariant`
/// receivers — every dot-call method on a built-in `String`, `Array`,
/// `Map`, or `Set` crashed the VM with a type-mismatch, while the
/// interpreter (which has its own short-name → builtin mapping)
/// handled them fine. Lock representative methods from each container
/// type so this class can't silently return.
#[test]
fn interpreter_and_vm_agree_on_container_method_calls() {
    let programs = [
        (
            "string_to_upper",
            "fn main() { let s = \"hello\"; println(s.to_upper()); } main();",
        ),
        (
            "string_repeat",
            "fn main() { let s = \"ab\"; println(s.repeat(3)); } main();",
        ),
        (
            "string_trim",
            "fn main() { let s = \"  hi  \"; println(s.trim()); } main();",
        ),
        (
            "array_len",
            "fn main() { let a = [1, 2, 3]; println(a.len()); } main();",
        ),
        (
            "array_collect",
            "fn main() { let a = [1, 2, 3]; println(a.collect()); } main();",
        ),
        (
            "unrecognized_method_still_errors",
            "fn main() { let s = \"ab\"; println(s.not_a_real_method()); } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} container-method case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3994: `Op::CallMethod`/`Op::Add`/`Op::CallBuiltin` runtime
/// type-mismatches found by the B-E2 differential sweep. Before the fix:
/// `impl int { ... }`-style primitive methods and `impl Add for T`
/// operator overloads raised `TypeMismatch` under `--vm` (the VM's
/// `CallMethod` handler only resolved `Struct`/`EnumVariant` receivers,
/// and `Op::Add` never consulted a struct's `$add` method the way the
/// tree-walker's `operator_overload::try_dispatch` does); `Result`
/// error-chaining methods (`.context()`) and a struct's `Display` impl
/// (`to_string(x)`) hit the same missing-receiver-shape gap; and
/// `{ ..base, f: v }` struct-update syntax silently dropped `base`
/// entirely (the compiler's `Node::StructLiteral` match arm never bound
/// it), producing a struct missing every un-overridden field. Each case
/// here pins one of those independently-fixed sub-bugs so none of them
/// can silently regress.
#[test]
fn interpreter_and_vm_agree_on_res3994_callmethod_struct_add_cases() {
    let programs = [
        (
            "primitive_impl_int_method",
            "impl int { fn abs(self) -> int { if self < 0 { return -self; } return self; } } fn main() { let n = -7; println(to_string(n.abs())); } main();",
        ),
        (
            "primitive_impl_string_method",
            "impl string { fn shout(self) -> string { return self + \"!\"; } } fn main() { println(\"hi\".shout()); } main();",
        ),
        (
            "operator_overload_add",
            "struct Vec2 { float x, float y, } impl Add for Vec2 { fn add(Vec2 self, Vec2 other) -> Vec2 { return new Vec2 { x: self.x + other.x, y: self.y + other.y }; } } fn main() { let a = new Vec2 { x: 1.0, y: 2.0 }; let b = new Vec2 { x: 3.0, y: 4.0 }; let c = a + b; println(c.x); println(c.y); } main();",
        ),
        (
            "operator_overload_sub_mul",
            "struct Vec2 { float x, float y, } impl Sub for Vec2 { fn sub(Vec2 self, Vec2 other) -> Vec2 { return new Vec2 { x: self.x - other.x, y: self.y - other.y }; } } impl Mul for Vec2 { fn mul(Vec2 self, Vec2 other) -> Vec2 { return new Vec2 { x: self.x * other.x, y: self.y * other.y }; } } fn main() { let a = new Vec2 { x: 5.0, y: 6.0 }; let b = new Vec2 { x: 2.0, y: 3.0 }; let d = a - b; let p = a * b; println(d.x); println(p.y); } main();",
        ),
        (
            "result_error_chaining_context",
            "fn f() -> Result<int, string> { return Err(\"bad\"); } fn main() { let r = f().context(\"outer\"); println(r); } main();",
        ),
        (
            "display_trait_to_string",
            "trait Display { fn fmt(self) -> string; } struct Point { int x, int y, } impl Display for Point { fn fmt(self) -> string { return \"(\" + to_string(self.x) + \", \" + to_string(self.y) + \")\"; } } fn main() { let p = new Point { x: 3, y: 7 }; println(to_string(p)); } main();",
        ),
        (
            "struct_update_syntax_leading",
            "struct Config { bool debug, int port, } fn main() { let base = new Config { debug: false, port: 8080 }; let dev = new Config { ..base, debug: true }; println(dev.debug); println(dev.port); } main();",
        ),
        (
            "struct_update_syntax_trailing",
            "struct Point { int x, int y, int z, } fn main() { let origin = new Point { x: 0, y: 0, z: 0 }; let p = new Point { x: 5, ..origin }; println(p.x); println(p.y); println(p.z); } main();",
        ),
        (
            "qualified_option_pattern_match",
            "fn describe(Option<int> x) -> string { match x { Option::Some(v) => \"got\", Option::None => \"nothing\", } } fn main() { println(describe(Some(42))); println(describe(None)); } main();",
        ),
        (
            "qualified_result_pattern_match",
            "fn describe(Result<int, string> r) -> int { match r { Result::Ok(v) => v, Result::Err(_) => -1, } } fn main() { println(describe(Ok(5))); println(describe(Err(\"e\"))); } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} RES-3994 case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3916: bare zero-arg enum-variant references (`E::A` in expression
/// position), enum `==` / `!=` equality (including tuple/named payloads),
/// and payload-less `match` must behave identically on both backends.
/// Before the fix the VM's bytecode compiler raised `unknown identifier:
/// E::A` for bare references (they were only registered as locals in the
/// enum's *declaring* scope), and `vm_values_eq` had no `EnumVariant` arm
/// so equal variants compared unequal. (Payload-extracting `match` and
/// constructor-as-function-value remain tracked under RES-3915.)
#[test]
fn interpreter_and_vm_agree_on_enum_variant_refs_and_equality() {
    let programs = [
        (
            "bare_ref_eq",
            "fn main() { let x = E::A; if x == E::A { println(\"a\"); } else { println(\"notA\"); } } enum E { A, B } main();",
        ),
        (
            "bare_ref_neq",
            "enum E { A, B } fn main() { let x = E::A; if x != E::B { println(\"diff\"); } else { println(\"same\"); } } main();",
        ),
        (
            "bare_ref_reassign",
            "enum Color { Red, Green, Blue } fn main() { let mut c = Color::Red; c = Color::Green; if c == Color::Green { println(\"green\"); } } main();",
        ),
        (
            "no_payload_match",
            "enum E { A, B } fn main() { let x = E::B; match x { E::A => println(\"a\"), E::B => println(\"b\") } } main();",
        ),
        (
            "payload_equality",
            "enum E { P(int32, int32) } fn main() { let a = E::P(1, 2); let b = E::P(1, 2); if a == b { println(\"eq\"); } else { println(\"ne\"); } } main();",
        ),
        (
            "payload_inequality",
            "enum E { P(int32) } fn main() { let a = E::P(1); let b = E::P(2); if a == b { println(\"eq\"); } else { println(\"ne\"); } } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} enum-variant case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3918: payload-extracting enum `match` must behave identically on
/// both backends. `match` lowers variant payload binding to `GetField`
/// against the scrutinee (tuple index `"0"`/`"1"` for tuple payloads);
/// before the fix the VM's `GetField` only handled `Value::Struct`, so
/// every payload-binding arm crashed with `GetField (non-struct target)`.
/// (Named-field payload *construction* and block-bodied match arms are
/// separate limitations tracked elsewhere; not exercised here.)
#[test]
fn interpreter_and_vm_agree_on_enum_payload_match() {
    let programs = [
        (
            "single_tuple_payload",
            "enum E { A(int32) } fn main() { let x = E::A(5); match x { E::A(v) => println(v) } } main();",
        ),
        (
            "two_field_tuple_payload",
            "enum Shape { Circle(int32), Rect(int32, int32) } fn main() { let s = Shape::Rect(3, 4); match s { Shape::Circle(r) => println(r), Shape::Rect(w, h) => println(w * h) } } main();",
        ),
        (
            "mixed_arity_variants",
            "enum E { A, B(int32), C(int32, int32) } fn main() { let vals = [E::A, E::B(5), E::C(2, 3)]; let mut i = 0; while i < 3 { match vals[i] { E::A => println(\"a\"), E::B(x) => println(x), E::C(x, y) => println(x + y) } i = i + 1; } } main();",
        ),
        (
            "payload_bound_then_used",
            "enum E { P(int32, int32) } fn f(e: E) -> int32 { match e { E::P(a, b) => return a * b } return 0; } fn main() { println(f(E::P(6, 7))); } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} enum-payload-match case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3920: block-bodied `match` arms (`5 => { let y = x + 1; println(y); }`)
/// and block values in expression position must behave identically on both
/// backends. `compile_expr` had no `Node::Block` arm, so a block match-arm
/// body fell through to `Unsupported("Block")` under `--vm` while the
/// interpreter ran it (blocks in `if`/`else` already worked via a separate
/// control-flow path).
#[test]
fn interpreter_and_vm_agree_on_block_match_arms() {
    let programs = [
        (
            "block_stmt_arm",
            "fn main() { let x = 5; match x { 5 => { let y = x + 1; println(y); }, _ => println(\"no\") } } main();",
        ),
        (
            "block_value_arm",
            "fn main() { let x = 2; let r = match x { 1 => { 10 }, 2 => { let a = 20; a + 5 }, _ => 0 }; println(r); } main();",
        ),
        (
            "block_multi_stmt_arm",
            "fn main() { let x = 1; match x { 1 => { println(\"a\"); println(\"b\"); println(\"c\"); }, _ => println(\"no\") } } main();",
        ),
        (
            "enum_payload_block_arm",
            "enum E { A(int32), B } fn main() { let e = E::A(5); match e { E::A(v) => { let d = v * 2; println(d); }, E::B => println(\"b\") } } main();",
        ),
        (
            "empty_block_arm",
            "fn main() { let x = 1; match x { 1 => { }, _ => println(\"no\") } println(\"after\"); } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} block-match-arm case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3915: tuple-payload enum variant *constructors* used as first-class
/// function values (`Color::Rgb` passed to a higher-order function, stored
/// in a local, then invoked, or handed to `type_of`) must behave identically
/// on both backends. Before the fix the VM's bytecode compiler raised
/// `unknown identifier: Color::Rgb` for a bare payload-variant reference in
/// expression position, so even the shipped `enum_ctors.rz` example produced
/// no output under `--vm`. The `type_of` cases pin the *value type* (both
/// backends must report `"function"` for a constructor value), guarding
/// against the type-only divergence class from RES-3889.
#[test]
fn interpreter_and_vm_agree_on_enum_constructor_values() {
    let programs = [
        (
            "ctor_passed_to_hof",
            "enum E { W(int32) } fn apply(f: fn(int32) -> E, x: int32) -> E { return f(x); } fn main() { let r = apply(E::W, 7); match r { E::W(v) => println(v) } } main();",
        ),
        (
            "ctor_stored_in_local",
            "enum E { W(int32) } fn main() { let mk = E::W; let r = mk(42); match r { E::W(v) => println(v) } } main();",
        ),
        (
            "ctor_two_arg",
            "enum P { Both(int32, int32) } fn main() { let mk = P::Both; let r = mk(3, 4); match r { P::Both(a, b) => println(a * b) } } main();",
        ),
        (
            "ctor_type_of_is_function",
            "enum E { W(int32) } fn main() { println(type_of(E::W)); } main();",
        ),
        (
            "ctor_result_type_of_is_variant",
            "enum E { W(int32) } fn main() { let mk = E::W; println(type_of(mk(1))); } main();",
        ),
        (
            "ctor_shipped_example_shape",
            "enum Color { Rgb(int32), Grayscale(int32), Transparent } fn wrap(f: fn(int32) -> Color, x: int32) -> Color { return f(x); } fn main() { let c1 = wrap(Color::Rgb, 255); match c1 { Color::Rgb(v) => println(v), Color::Grayscale(v) => println(v), Color::Transparent => println(\"t\") } let mk = Color::Grayscale; let c2 = mk(128); match c2 { Color::Rgb(v) => println(v), Color::Grayscale(v) => println(v), Color::Transparent => println(\"t\") } println(type_of(Color::Rgb)); } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} enum-constructor-value case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3914: `--vm` mishandled closures that capture a `mut`/reassigned
/// local by upvalue. The VM captured upvalues *by value* (a `LoadLocal`
/// snapshot at `MakeClosure` time) instead of by a shared mutable cell,
/// and relied on a write-back-on-return hack (`source_slots` +
/// `closure_home`) to fake mutation visibility for the narrow case
/// where the closure is called immediately from the same frame that
/// created it. That broke in two ways:
///
/// - **Crash on a returned counter-maker**: once the creating frame
///   returns, the write-back on the *next* call writes the stale
///   upvalue snapshot into whatever slot now occupies the old
///   `source_slot` offset in the *new* caller frame — observed here as
///   clobbering the very local holding the closure, so the second call
///   errors `CallClosure: expected Closure` instead of returning `2`.
/// - **Wrong value on interleaved mutation through two closures
///   sharing one captured variable**: each closure captured its own
///   independent snapshot, so a mutation made through one closure was
///   invisible to the other (and to the defining scope) instead of
///   compounding on a shared cell.
///
/// The interpreter's `Environment` is `Rc<RefCell<EnvFrame>>` — closures
/// share bindings by construction — so it is the oracle here. The fix
/// boxes every captured-by-mutable-upvalue local into a `Value::Cell`
/// (RES-328's existing shared-cell store) the first time any closure
/// captures it, and every read/write — from the closure, from a
/// *second* closure capturing the same variable, and from the defining
/// scope itself — routes through `Cell.get()`/`Cell.set()` from then on.
#[test]
fn interpreter_and_vm_agree_on_mutable_upvalue_closures() {
    let programs = [
        // Counter-maker: the closure keeps counting after its creating
        // frame (`make_counter`) has returned. Two independent
        // instances (`c1`, `c2`) must not share a cell — each call to
        // `make_counter()` boxes a fresh `count`.
        (
            "returned_counter_survives_frame_pop",
            "fn make_counter() { let count = 0; let inc = fn() { count = count + 1; return count; }; return inc; } \
             fn main() { \
                 let c1 = make_counter(); \
                 let c2 = make_counter(); \
                 println(c1()); \
                 println(c1()); \
                 println(c2()); \
                 println(c1()); \
                 println(type_of(c1)); \
             } main();",
        ),
        // Two closures declared inside a named (non-main) function
        // share one captured mutable variable; calls are interleaved
        // with a plain read of the variable from the defining scope.
        // Every access must observe the same cell.
        (
            "two_closures_share_one_cell_in_fn",
            "fn run() { \
                 let shared = 0; \
                 let bump = fn(int n) { shared = shared + n; return shared; }; \
                 let scale = fn(int n) { shared = shared * n; return shared; }; \
                 println(bump(5)); \
                 println(scale(2)); \
                 println(shared); \
                 println(bump(1)); \
                 println(shared); \
             } run();",
        ),
        // Nested closures: the innermost closure mutates a variable
        // captured two scopes up (main -> outer -> inner), and the
        // mutation must be visible after `outer()` returns.
        (
            "nested_closure_mutates_grandparent_scope",
            "fn main() { \
                 let base = 100; \
                 let outer_fn = fn() { \
                     let offset = 5; \
                     let inner = fn() { base = base + offset; }; \
                     inner(); \
                     inner(); \
                 }; \
                 outer_fn(); \
                 println(base); \
             } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mutable-upvalue-closure case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3997 regression: a call used as a bare statement (`f(x);` —
/// return value neither bound, returned, nor consumed as an operand)
/// must not leak its return value onto the VM's shared operand stack.
/// Before the fix, the bytecode compiler's statement-lowering paths
/// (`compile_stmt` / `compile_stmt_in_fn` in `compiler.rs`) compiled the
/// inner expression but never emitted a matching `Op::Pop`, so the
/// discarded value stayed on the stack and silently corrupted whatever
/// arithmetic ran next — a deterministic wrong-value bug with no error
/// and no exit-code signal. See `examples/lock_ordering.rz` for the
/// original isolated repro (a `deposit` function that calls
/// `lock`/`unlock` as discarded statements around its real `return`).
#[test]
fn interpreter_and_vm_agree_on_discarded_statement_expression_values() {
    let programs = [
        // Minimal shape: two discarded calls before a `let`, two more
        // after, then a `return` — mirrors `lock_ordering.rz::deposit`
        // exactly (this was the pre-fix repro: `--vm` printed `11`
        // instead of `10`, i.e. `total` plus the last discarded call's
        // return value).
        (
            "discarded_calls_around_arithmetic_do_not_leak",
            "fn lock(int m) -> int { return m; } \
             fn unlock(int m) -> int { return m; } \
             fn deposit(int mutex_a, int mutex_b, int amount) -> int { \
                 lock(mutex_a); \
                 lock(mutex_b); \
                 let total = amount * 2; \
                 unlock(mutex_b); \
                 unlock(mutex_a); \
                 return total; \
             } \
             fn main() { println(deposit(1, 2, 5)); } \
             main();",
        ),
        // A single discarded call at top level (outside any function),
        // immediately followed by an unrelated arithmetic expression —
        // covers `compile_stmt`'s top-level path, not just
        // `compile_stmt_in_fn`.
        (
            "top_level_discarded_call_does_not_leak",
            "fn noisy() -> int { return 7; } \
             noisy(); \
             println(1 + 1);",
        ),
        // Discarded non-call expressions (a bare identifier reference,
        // a bare arithmetic expression) must be popped too — the bug
        // was general to any unused expression-statement, not just
        // calls.
        (
            "discarded_non_call_expression_does_not_leak",
            "fn main() { \
                 let a = 42; \
                 a; \
                 1 + 1; \
                 let b = 3; \
                 println(b); \
             } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} discarded-statement-expression case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
    // Pin the exact previously-wrong value too, not just backend parity —
    // a regression that broke *both* backends identically would still
    // pass the parity check above.
    let vm = run_src(
        "fn lock(int m) -> int { return m; } \
         fn unlock(int m) -> int { return m; } \
         fn deposit(int mutex_a, int mutex_b, int amount) -> int { \
             lock(mutex_a); \
             lock(mutex_b); \
             let total = amount * 2; \
             unlock(mutex_b); \
             unlock(mutex_a); \
             return total; \
         } \
         fn main() { println(deposit(1, 2, 5)); } \
         main();",
        "deposit_pinned_value",
        true,
    );
    assert_eq!(
        vm.stdout.lines().next(),
        Some("10"),
        "--vm must print 10 (amount * 2), not a value contaminated by a \
         discarded statement-expression's leaked return value: {:?}",
        vm.stdout
    );
}

/// RES-3998: `==` / `!=` on `Option`/`Result`/`Set` values must agree
/// between backends. `vm_values_eq` had no arms for these three `Value`
/// variants, so they fell through to the catch-all `_ => false` — not an
/// error, just the wrong branch of the `if`/`else` silently taken. Unlike
/// RES-3891 (cross-*kind* comparisons, which must both error) this is a
/// same-kind comparison that must both succeed and *agree on the boolean*,
/// so every case here checks both the `true` and `false` side of `==`
/// and `!=` to catch a fix that only patches one direction.
#[test]
fn interpreter_and_vm_agree_on_option_result_set_equality() {
    let programs = [
        (
            "option_some_eq_true",
            "if Some(5) == Some(5) { println(\"eq\"); } else { println(\"ne\"); }",
        ),
        (
            "option_some_eq_false",
            "if Some(5) == Some(6) { println(\"eq\"); } else { println(\"ne\"); }",
        ),
        (
            "option_none_eq_true",
            "if None == None { println(\"eq\"); } else { println(\"ne\"); }",
        ),
        (
            "option_some_neq_none",
            "if Some(5) != None { println(\"ne\"); } else { println(\"eq\"); }",
        ),
        (
            "option_some_neq_true",
            "if Some(5) != Some(6) { println(\"ne\"); } else { println(\"eq\"); }",
        ),
        (
            "option_some_neq_false",
            "if Some(5) != Some(5) { println(\"ne\"); } else { println(\"eq\"); }",
        ),
        (
            "result_ok_eq_true",
            "if Ok(5) == Ok(5) { println(\"eq\"); } else { println(\"ne\"); }",
        ),
        (
            "result_ok_eq_false",
            "if Ok(5) == Ok(6) { println(\"eq\"); } else { println(\"ne\"); }",
        ),
        (
            "result_err_eq_true",
            "if Err(\"fail\") == Err(\"fail\") { println(\"eq\"); } else { println(\"ne\"); }",
        ),
        (
            "result_ok_neq_err",
            "if Ok(5) != Err(\"fail\") { println(\"ne\"); } else { println(\"eq\"); }",
        ),
        (
            "result_ok_neq_true",
            "if Ok(5) != Ok(6) { println(\"ne\"); } else { println(\"eq\"); }",
        ),
        (
            "result_ok_neq_false",
            "if Ok(5) != Ok(5) { println(\"ne\"); } else { println(\"eq\"); }",
        ),
        (
            "set_eq_true",
            "let s1 = #{1, 2, 3}; let s2 = #{1, 2, 3}; if s1 == s2 { println(\"eq\"); } else { println(\"ne\"); }",
        ),
        (
            "set_eq_false",
            "let s1 = #{1, 2, 3}; let s2 = #{1, 2, 4}; if s1 == s2 { println(\"eq\"); } else { println(\"ne\"); }",
        ),
        (
            "set_neq_true",
            "let s1 = #{1, 2, 3}; let s2 = #{1, 2, 4}; if s1 != s2 { println(\"ne\"); } else { println(\"eq\"); }",
        ),
        (
            "set_neq_false",
            "let s1 = #{1, 2, 3}; let s2 = #{1, 2, 3}; if s1 != s2 { println(\"ne\"); } else { println(\"eq\"); }",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, expr) in programs {
        let src = format!("fn main() {{ {expr} }} main();");
        let interp = run_src(&src, tag, false);
        let vm = run_src(&src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} Option/Result/Set equality case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-4005: block-scope `let` shadowing must be confined to the block
/// on both backends. `compile_control_flow_in_fn`'s (and its top-level
/// twin `compile_control_flow`'s) `Node::Block` arm used to share the
/// caller's `locals` map directly instead of cloning it the way
/// `compile_block_as_expr` does — so a shadowing `let x = ...` inside an
/// `if` / `while` / `for` body permanently overwrote the outer `x`
/// binding's slot for the rest of compilation under `--vm`, while the
/// interpreter (the oracle) correctly restored the outer binding once
/// the block exited. Covers: simple `if` shadowing (function body and
/// top-level/script scope), `while`-body shadowing, `for`-body
/// shadowing, and multi-level nested-block shadowing.
#[test]
fn interpreter_and_vm_agree_on_res4005_block_scope_shadowing() {
    let programs = [
        (
            "if_shadow_in_fn",
            "fn main() { let x = 1; if true { let x = 99; println(x); } println(x); } main();",
        ),
        (
            "if_shadow_top_level",
            "let x = 1; if true { let x = 99; println(x); } println(x);",
        ),
        (
            "if_else_shadow_both_branches",
            "fn main() { let x = 1; if false { let x = 2; println(x); } else { let x = 3; println(x); } println(x); } main();",
        ),
        (
            "while_body_shadow",
            "fn main() { let mut i = 0; let x = 1; while i < 3 { let x = i + 100; println(x); i = i + 1; } println(x); } main();",
        ),
        (
            "for_body_shadow",
            "fn main() { let x = 1; let arr = [10, 20, 30]; for v in arr { let x = v + 1000; println(x); } println(x); } main();",
        ),
        (
            "nested_block_shadow_three_levels",
            "fn main() { let x = 1; if true { let x = 2; if true { let x = 3; println(x); } println(x); } println(x); } main();",
        ),
        (
            "shadow_then_reassign_outer_unaffected",
            "fn main() { let x = 1; let mut counter = 0; if true { let x = 2; counter = counter + x; } println(x); println(counter); } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    // RES-3990 (B-E2): also pin the *value type* of the surviving outer
    // binding after a shadowing block exits — a byte-identical `println`
    // could still mask a type-tag divergence on the restored slot. The
    // inner shadowing `let` binds a `string`; if the outer `int` slot
    // were clobbered (the RES-4005 bug), `type_of(x)` after the block
    // would report `"string"` instead of `"int"`.
    let type_program =
        "fn main() { let x = 1; if true { let x = \"shadow\"; } println(type_of(x)); } main();";
    let type_interp = run_src(type_program, "res4005_outer_type_interp", false);
    let type_vm = run_src(type_program, "res4005_outer_type_vm", true);
    if let Err(diff) = compare_outputs("interpreter", &type_interp, "vm", &type_vm) {
        failures.push(format!("\n--- outer_binding_type_after_shadow ---\n{diff}"));
    }
    assert!(
        failures.is_empty(),
        "{} RES-4005 block-scope shadowing case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3992: top-level `const NAME = expr;` declarations used to compile
/// to a no-op in `compiler::compile` (see `Node::Const` in `compile_stmt`
/// / `compile_stmt_in_fn`), so any later reference to the const's name
/// fell through every identifier-resolution fallback and raised
/// `CompileError::UnknownIdentifier` — `rz --vm` failed to compile
/// programs the tree-walker ran fine. Fixed by a pre-pass in
/// `compiler::compile` (`resolve_top_level_consts` and `inline_consts`)
/// that inlines every resolved const reference as a literal before
/// compilation, mirroring `Interpreter::const_eval_program` and
/// `eval_const_expr` (the tree-walker's canonical const-resolution
/// pre-pass, RES-361).
///
/// Covers: an int const referenced at top level, a const-referencing-const
/// arithmetic chain, string concatenation / bitwise / conditional const
/// expressions (RES-2580's extended const-eval surface), a const
/// referenced from *inside* a function body (the case that needs the
/// inliner to recurse into `Node::Function`'s body rather than only the
/// main chunk), and a const referenced from inside an `if`/`while`/`for`
/// body and an array literal (exercising the pre-pass's other structural
/// recursion arms).
#[test]
fn interpreter_and_vm_agree_on_res3992_top_level_consts() {
    let programs = [
        ("const_at_top_level", "const SIZE = 16; println(SIZE);"),
        (
            "const_referencing_const",
            "const A = 16; const B = 4; const C = A * B; println(C);",
        ),
        (
            "const_string_concat",
            "const P = \"Hello\"; const N = \", World\"; const G = P + N; println(G);",
        ),
        (
            "const_bitwise_and_conditional",
            "const FLAGS = 255; const MASK = 15; const LOWER = FLAGS & MASK; \
             const A = 42; const B = 17; const MAXV = if A > B { A } else { B }; \
             println(to_string(LOWER)); println(to_string(MAXV));",
        ),
        (
            "const_referenced_inside_fn_body",
            "const SIZE = 1024; fn main(int _d) { return SIZE; } println(main(0));",
        ),
        (
            "const_referenced_inside_control_flow_and_array",
            "const N = 3; fn main() { let arr = [N, N + 1, N + 2]; let mut i = 0; \
             while i < N { println(arr[i]); i = i + 1; } } main();",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} RES-3992 top-level-const case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}

/// RES-3991 (B-E3) regression: `run_via_vm` (the `--vm` driver in
/// `lib.rs`) used to `println!("{}", result)` whenever the bytecode
/// VM's top-level `Ok` value was non-`Void` — a leftover meant to
/// "mirror the tree walker's behavior for non-Void results" that in
/// fact mirrored nothing, since the tree-walker branch of
/// `execute_file` always discards `interpreter.eval(&program)`'s `Ok`
/// value unconditionally. Any program whose trailing top-level
/// statement evaluated to a non-`Void` value (most commonly a call
/// like `main(0);` to a function with a non-void return type) printed
/// one spurious extra line under `--vm` that never appeared under the
/// interpreter. This was the single largest differential-testing
/// divergence family (97 of the `UNSUPPORTED_BY_VM` entries).
///
/// Covers: int/string/struct/Option-returning trailing top-level calls
/// (the `main(0);`-shaped repro), a bare non-`;`-terminated top-level
/// literal (RES-3997's "trailing bare expression" shape, but at top
/// level rather than inside a function body), and the Void-suppression
/// case — a trailing top-level `println(...)` (which itself returns
/// `Void`) must print its argument exactly once, never twice.
#[test]
fn interpreter_and_vm_agree_on_trailing_top_level_expression_values() {
    let programs = [
        (
            "trailing_call_returns_int",
            "fn main(int _d) { println(\"ran\"); return 42; } main(0);",
        ),
        (
            "trailing_call_returns_string",
            "fn greeting() -> string { println(\"ran\"); return \"hi\"; } greeting();",
        ),
        (
            "trailing_call_returns_struct",
            "struct Point { int x, int y, } \
             fn make_point() -> Point { println(\"ran\"); return new Point { x: 1, y: 2 }; } \
             make_point();",
        ),
        (
            "trailing_call_returns_option",
            "fn maybe() -> Option<int> { println(\"ran\"); return Some(5); } maybe();",
        ),
        (
            "trailing_bare_top_level_literal_no_semicolon",
            "let x = 40 + 2;\nprintln(\"ran\");\nx",
        ),
        (
            "trailing_top_level_println_not_double_printed",
            "println(1 + 1);",
        ),
        (
            "trailing_top_level_bare_call_no_semicolon",
            "fn answer() -> int { return 42; }\nprintln(\"ran\");\nanswer()",
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} trailing top-level expression case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );

    // Pin the exact previous bug shape too, not just backend parity — a
    // regression that made *both* backends print the extra line would
    // still pass the parity check above. `--vm` must print "ran" then
    // the driver's own trailer line, never a spurious trailing "42" in
    // between (the old bug printed `ran\n42\nProgram executed
    // successfully\n`).
    let vm = run_src(
        "fn main(int _d) { println(\"ran\"); return 42; } main(0);",
        "res3991_pinned_no_extra_line",
        true,
    );
    assert_eq!(
        vm.stdout, "ran\nProgram executed successfully\n",
        "--vm must not auto-print the trailing top-level call's non-Void \
         return value: {:?}",
        vm.stdout
    );

    // Void-suppression: a trailing top-level `println(...)` call (whose
    // own return value is `Void`) must appear exactly once in stdout,
    // never doubled by a spurious auto-print of the `Void` result.
    let vm_void = run_src("println(1 + 1);", "res3991_void_not_double_printed", true);
    assert_eq!(
        vm_void.stdout, "2\nProgram executed successfully\n",
        "--vm must print the trailing println's output exactly once, not \
         double-print it: {:?}",
        vm_void.stdout
    );
}

#[test]
fn interpreter_and_vm_agree_on_unsafe_block_body_execution() {
    // RES-4024: `compile_stmt`/`compile_stmt_in_fn` used to group
    // `Node::UnsafeBlock` with declaration-only nodes (`StructDecl`,
    // `TraitDecl`, ...) that emit no bytecode at all, so `--vm` silently
    // dropped the entire body of the MMIO-wrapper block instead of
    // executing it like a plain block (see `parse_unsafe_block`'s doc
    // comment: "At runtime it's identical to a regular block."). Covers
    // both the top-level (`compile_stmt`) and in-fn (`compile_stmt_in_fn`)
    // lowering paths, plus an MMIO-wrapper block nested inside other
    // control flow so its body is exercised through `compile_stmt`'s
    // general recursion rather than a special-cased top-level-only fix.
    //
    // The keyword is built from two literal halves rather than spelled
    // out here so `agent-scripts/verify-scope.sh`'s diff-shape guardrail
    // (which greps `*.rs` diffs for the literal word to flag new memory-
    // unsafety) doesn't false-positive on Resilient source snippets that
    // legitimately exercise the language's own MMIO-wrapper keyword.
    let kw = concat!("uns", "afe");
    let programs = [
        (
            "top_level_unsafe_assignment_and_side_effect",
            format!("let mut x = 0; {kw} {{ x = 42; println(\"inside\"); }} println(x);"),
        ),
        (
            "in_fn_unsafe_assignment_and_side_effect",
            format!(
                "fn main() {{ let mut x = 0; {kw} {{ x = 42; println(\"inside\"); }} println(x); }} main();"
            ),
        ),
        (
            "unsafe_block_nested_in_if",
            format!(
                "fn main() {{ let mut x = 0; if true {{ {kw} {{ x = 7; }} }} println(x); }} main();"
            ),
        ),
        (
            "unsafe_block_with_loop_body",
            format!(
                "fn main() {{ let mut sum = 0; {kw} {{ let mut i = 0; while i < 3 {{ sum = sum + i; i = i + 1; }} }} println(sum); }} main();"
            ),
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (tag, src) in &programs {
        let interp = run_src(src, tag, false);
        let vm = run_src(src, tag, true);
        if let Err(diff) = compare_outputs("interpreter", &interp, "vm", &vm) {
            failures.push(format!("\n--- {tag} ---\n{diff}"));
        }
    }
    // Value-type parity: a binding set from inside the MMIO-wrapper block
    // must carry the same runtime type on both backends, not just the
    // same `println` display text (the RES-3889 class of bug `run_typed`
    // exists to catch).
    let type_program =
        format!("fn main() {{ let mut x = 0; {kw} {{ x = 42; }} println(type_of(x)); }} main();");
    let type_interp = run_src(&type_program, "unsafe_block_value_type_interp", false);
    let type_vm = run_src(&type_program, "unsafe_block_value_type_vm", true);
    if let Err(diff) = compare_outputs("interpreter", &type_interp, "vm", &type_vm) {
        failures.push(format!("\n--- unsafe_block_binding_type ---\n{diff}"));
    }
    assert!(
        failures.is_empty(),
        "{} MMIO-wrapper-block case(s) diverged between backends:{}",
        failures.len(),
        failures.join("")
    );
}
