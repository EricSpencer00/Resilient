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
    // RES-3991: `--vm` auto-prints the trailing top-level non-Void return value;
    // the interpreter discards it (the `use_vm` branch of `execute_file` in
    // `lib.rs` prints `result` when non-`Void`; the tree-walker branch never
    // does). One extra trailing numeric line under `--vm`.
    "acos.rz",
    "acosh.rz",
    "alignment_helpers.rz",
    "array_argminmax.rz",
    "array_binary_search.rz",
    "array_chunking.rz",
    "array_cumulative.rz",
    "array_polymorphic_pos.rz",
    "array_set_helpers.rz",
    "array_slicing.rz",
    "array_sort_extra.rz",
    "array_stats.rz",
    "as_cast.rz",
    "asin.rz",
    "asinh.rz",
    "assoc_type_device_driver.rz",
    "assoc_type_simple.rz",
    "atan.rz",
    "atanh.rz",
    "bit_counting.rz",
    "bit_manipulation.rz",
    "break_continue.rz",
    "builtin_methods.rz",
    "bytes_and_or_not.rz",
    "bytes_concat.rz",
    "bytes_conversions.rz",
    "bytes_eq.rz",
    "bytes_fill_reverse.rz",
    "bytes_helpers.rz",
    "bytes_slicing.rz",
    "bytes_xor.rz",
    "cbrt.rz",
    "cfg_attr_demo.rz",
    "cluster_single_leader_bad.rz",
    "cluster_single_leader_ok.rz",
    "compound_assignment.rz",
    "compound_lvalue.rz",
    "copysign.rz",
    "cosh.rz",
    "embedded_uart_driver.rz",
    "enum_payload_exhaust_missing.rz",
    "exp2.rz",
    "first_class_fn_anon.rz",
    "float_classify.rz",
    "for_tuple_binding.rz",
    "hash_builtins.rz",
    "hashmap_len.rz",
    "hashmap_values.rz",
    "hypot.rz",
    "if_expression.rz",
    "int_rotate.rz",
    "interactive_greeter.rz",
    "is_ascii_family.rz",
    "iter_helpers.rz",
    "linear_closure_capture_error.rz",
    "linear_closure_demo.rz",
    "linear_demo.rz",
    "linear_double_use.rz",
    "linear_effect_io_accepted.rz",
    "log10.rz",
    "log2.rz",
    "loop_keyword.rz",
    "map_contains_key.rz",
    "map_entries_merge.rz",
    "map_values.rz",
    "match_struct_nonexhaustive.rz",
    "negative_indices.rz",
    "pipe_operator.rz",
    "polymorphic_types.rz",
    "precision_math.rz",
    "pure_effect_demo.rz",
    "range_patterns.rz",
    "result_patterns.rz",
    "rounding.rz",
    "scientific_notation.rz",
    "set_difference.rz",
    "set_intersection.rz",
    "set_is_disjoint.rz",
    "set_is_subset.rz",
    "set_is_superset.rz",
    "set_result_option.rz",
    "set_symmetric_difference.rz",
    "set_union.rz",
    "sinh.rz",
    "state_topology.rz",
    "string_array_misc.rz",
    "string_repetition.rz",
    "string_slicing.rz",
    "sum_types_constructor.rz",
    "sum_types_match.rz",
    "tanh.rz",
    "to_degrees.rz",
    "to_radians.rz",
    "tuple_pattern.rz",
    "underscore_numerics.rz",
    "unix_time.rz",
    // RES-3992: VM bytecode compiler "unknown identifier" / "unknown function" —
    // closures/consts captured across scopes, and static/namespaced/tuple-
    // struct-constructor calls the compiler doesn't resolve to a callable.
    "actor_deadlock.rz",
    "actor_ping_pong.rz",
    "actor_spawn_send.rz",
    "const_eval.rz",
    "const_eval_ext.rz",
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
    "static_assert.rz",
    "tuple_struct.rz",
    // RES-3993: VM bytecode compiler "unsupported construct" (Match, WhileStatement,
    // ReturnStatement, indirect calls, non-arithmetic operators, and an
    // <other> catch-all), plus the capability-gated volatile-MMIO block
    // (see `unsafe_block_smoke.rz`) whose statements the VM silently drops.
    "array_contains.rz",
    "array_sorted_invariant.rz",
    "bench_simple.rz",
    "break_with_value.rz",
    "comprehension_demo.rz",
    "defer_stmt.rz",
    "edge_closure_capture.rz",
    "edge_if_let_pattern.rz",
    "edge_while_let.rz",
    "if_let.rz",
    "match_block_arms.rz",
    "null_coalescing_operator.rz",
    "option_find.rz",
    "optional_chaining.rz",
    "quantifier_assert.rz",
    "quantifier_exists.rz",
    "quantifier_forall.rz",
    "showcase_quantifiers.rz",
    "unsafe_block_smoke.rz",
    "while_let.rz",
    // RES-3994: VM runtime type-mismatch: CallMethod receiver-shape gaps, struct-
    // update/field-lookup mismatches, `impl Add` operator-overload not
    // consulted by `Op::Add`, plus misc completeness (no TCO for mutual
    // recursion, `contains()` missing an (Array, T) overload).
    "array_functional.rz",
    "array_method_chains.rz",
    "display_trait.rz",
    "error_chaining.rz",
    "error_handling_patterns.rz",
    "mutual_tco.rz",
    "operator_overload.rz",
    "option_enum_roundtrip.rz",
    "primitive_impl.rz",
    "range_values.rz",
    "string_builder.rz",
    "struct_update_syntax.rz",
    "struct_update_trailing.rz",
    "transaction_commit.rz",
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
    // RES-3997: VM silently computes a *different* value (not an error) — a bare
    // statement-expression call's return value isn't popped off the VM's eval
    // stack and leaks into the next arithmetic op. Deterministic across runs
    // (not fault-injection nondeterminism); see `lock_ordering.rz` for the
    // isolated repro.
    "audit_log_required.rz",
    "bounded_blocking.rz",
    "degraded_mode.rz",
    "lock_ordering.rz",
    "priority_inheritance.rz",
    "res1111_block_scope.rz",
    "secret_erasure.rz",
    // RES-3998: VM equality (`==`) silently evaluates to `false` for equal Option /
    // Result / Set values (no error — the if/else just takes the wrong
    // branch), so the expected output line is silently missing.
    "char_equality_in_compounds.rz",
    "compound_equality.rz",
    "option_equality.rz",
    "result_equality.rz",
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
    // this is 412 of 581 examples (169 denylisted); the threshold below
    // is a conservative floor, not the exact figure, so unrelated
    // example-corpus growth doesn't make this test flaky.
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
/// `Range` is deliberately **not** included here: `type_of(1..5)`
/// reports `"range"` on the interpreter and `"array"` under `--vm` —
/// a real divergence caught by exactly this mechanism, tracked under
/// RES-4000 rather than asserted on here (asserting on it would just
/// make this test red instead of documenting the gap).
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
