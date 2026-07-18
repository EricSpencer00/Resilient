//! RES-3987 (D-E1): the round-trip proof for `rz build --target
//! <TRIPLE>`.
//!
//! This is the "does the pipeline actually close" test: compile a
//! `.rz` program with `rz build`, decode the emitted `.rzbc` blob
//! with the exact no_std decoder `resilient-runtime` ships
//! (`resilient_runtime::vm::serde::decode`), run it on the exact
//! embedded VM (`resilient_runtime::vm::Vm`), and check the result
//! against the tree-walking interpreter (the differential oracle
//! `resilient/tests/it/differential.rs` already uses) running the
//! same computation.
//!
//! The embedded subset has no I/O (see `rzbc_emit.rs`'s scope docs —
//! `println` lowers to `Op::CallBuiltin`, which isn't in the
//! `Instr` subset), so the interpreter reference run uses a sibling
//! source string that's identical except the trailing bare
//! expression is wrapped in `println(...)`. Both strings encode the
//! same arithmetic; only the observation mechanism differs (a
//! decoded/executed `.rzbc` blob's returned `Value` vs. stdout).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "res_rzbc_roundtrip_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

/// Run `rz build --target <target> <src> -o <out>`, returning
/// `(exit_code, stderr)`.
fn run_build(src: &Path, out: &Path, target: &str) -> (Option<i32>, String) {
    let output = Command::new(bin())
        .args(["build", "--target", target])
        .arg(src)
        .arg("-o")
        .arg(out)
        .output()
        .expect("spawn rz build");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Run the tree-walking interpreter (no flags — the default,
/// oracle backend per `differential.rs`) on `src` and return stdout.
fn run_interpreter(src: &Path) -> String {
    let output = Command::new(bin())
        .arg(src)
        .output()
        .expect("spawn rz (interpreter)");
    assert_eq!(
        output.status.code(),
        Some(0),
        "interpreter run should succeed; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Decode a `.rzbc` blob and run it on `resilient_runtime::vm::Vm`,
/// returning the runtime `Value`.
fn run_embedded_blob(blob: &[u8]) -> resilient_runtime::vm::Value {
    let mut instrs = [resilient_runtime::vm::Instr::Return; 64];
    let count = resilient_runtime::vm::serde::decode(blob, &mut instrs)
        .expect("decode should succeed on a blob `rz build` just emitted");
    let mut vm = resilient_runtime::vm::Vm::<32, 8>::new();
    vm.run(&instrs[..count])
        .expect("embedded VM should run the decoded program without error")
}

/// Core proof: `rz build` → `resilient_runtime::vm::serde::decode` →
/// `resilient_runtime::vm::Vm::run` produces the same value as the
/// tree-walking interpreter evaluating the equivalent (println'd)
/// source, for a small Int program with a `while` loop, comparisons,
/// and locals — exactly the no_std-representable subset this bridge
/// targets.
#[test]
fn build_decode_run_matches_interpreter_for_loop_program() {
    let dir = tmp_dir("loop");

    // i = 0, 2, 4 (stops at 6): sum = 0 + 2 + 4 = 6.
    let embedded_src = dir.join("sum_loop.rz");
    std::fs::write(
        &embedded_src,
        "let mut i: Int = 0;\n\
         let mut sum: Int = 0;\n\
         while i < 5 {\n\
         \x20   sum = sum + i;\n\
         \x20   i = i + 2;\n\
         }\n\
         sum;\n",
    )
    .unwrap();

    let interpreter_src = dir.join("sum_loop_print.rz");
    std::fs::write(
        &interpreter_src,
        "let mut i: Int = 0;\n\
         let mut sum: Int = 0;\n\
         while i < 5 {\n\
         \x20   sum = sum + i;\n\
         \x20   i = i + 2;\n\
         }\n\
         println(sum);\n",
    )
    .unwrap();

    let out = dir.join("sum_loop.rzbc");
    let (code, stderr) = run_build(&embedded_src, &out, "thumbv7em-none-eabihf");
    assert_eq!(code, Some(0), "rz build should succeed; stderr={stderr}");

    let blob = std::fs::read(&out).expect("rz build should have written the .rzbc file");
    assert!(
        blob.starts_with(b"RZBC"),
        "blob should start with the RZBC magic"
    );

    let embedded_result = run_embedded_blob(&blob);
    assert_eq!(embedded_result, resilient_runtime::vm::Value::Int(6));

    let interpreter_stdout = run_interpreter(&interpreter_src);
    assert_eq!(
        interpreter_stdout.lines().next(),
        Some("6"),
        "interpreter reference run should print the same value the embedded VM computed; \
         full stdout={interpreter_stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Same proof for a straight-line (no loop) arithmetic program, to
/// cover the non-control-flow half of the supported subset
/// independently of the jump-target math.
#[test]
fn build_decode_run_matches_interpreter_for_straight_line_arithmetic() {
    let dir = tmp_dir("straight");

    // (2 + 3) * 4 - 1 == 19
    let embedded_src = dir.join("arith.rz");
    std::fs::write(&embedded_src, "let x: Int = (2 + 3) * 4 - 1;\nx;\n").unwrap();

    let interpreter_src = dir.join("arith_print.rz");
    std::fs::write(
        &interpreter_src,
        "let x: Int = (2 + 3) * 4 - 1;\nprintln(x);\n",
    )
    .unwrap();

    let out = dir.join("arith.rzbc");
    let (code, stderr) = run_build(&embedded_src, &out, "riscv32imac-unknown-none-elf");
    assert_eq!(code, Some(0), "rz build should succeed; stderr={stderr}");

    let blob = std::fs::read(&out).unwrap();
    let embedded_result = run_embedded_blob(&blob);
    assert_eq!(embedded_result, resilient_runtime::vm::Value::Int(19));

    let interpreter_stdout = run_interpreter(&interpreter_src);
    assert_eq!(interpreter_stdout.lines().next(), Some("19"));

    let _ = std::fs::remove_dir_all(&dir);
}

/// `rz build` must reject (not silently miscompile) a program that
/// uses a construct outside the embedded subset — here, `println`
/// itself, which lowers to `Op::CallBuiltin` plus a `Value::String`
/// constant, neither of which `resilient_runtime::vm::Instr` has.
#[test]
fn build_rejects_unsupported_construct_with_clear_diagnostic() {
    let dir = tmp_dir("rejected");
    let src = dir.join("prints.rz");
    std::fs::write(&src, "println(\"hello\");\n").unwrap();
    let out = dir.join("prints.rzbc");

    let (code, stderr) = run_build(&src, &out, "thumbv7em-none-eabihf");
    assert_eq!(code, Some(1), "unsupported construct should be exit 1");
    assert!(
        stderr.contains("not supported for embedded target"),
        "expected a clear unsupported-construct diagnostic; got: {stderr}"
    );
    assert!(
        !out.exists(),
        "a rejected build must not leave a partial/broken .rzbc file behind"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// RES-4077 (D-E1 fn-support): `rz build` now accepts a program
/// containing top-level `fn` declarations — this test used to be
/// `build_rejects_fn_declarations` (reject with exit 1), and is
/// rewritten to prove the full positive pipeline instead: build →
/// decode the function-table `.rzbc` format with
/// `resilient_runtime::vm::serde::decode_program` → execute on
/// `Vm::run_with_functions` → match the tree-walking interpreter.
#[test]
fn build_decode_run_matches_interpreter_for_fn_declarations() {
    let dir = tmp_dir("fn_decl");
    let src = dir.join("with_fn.rz");
    std::fs::write(
        &src,
        "fn add(int a, int b) -> int {\n    return a + b;\n}\nadd(1, 2);\n",
    )
    .unwrap();
    let out = dir.join("with_fn.rzbc");

    let (code, stderr) = run_build(&src, &out, "thumbv7em-none-eabihf");
    assert_eq!(
        code,
        Some(0),
        "fn declarations should now build for embedded targets; stderr={stderr}"
    );

    let blob = std::fs::read(&out).expect("rz build should have written the .rzbc file");
    assert!(blob.starts_with(b"RZBC"));

    let mut out_main = [resilient_runtime::vm::Instr::Return; 32];
    let mut out_func_meta = [resilient_runtime::vm::serde::DecodedFunctionMeta {
        offset: 0,
        len: 0,
        arity: 0,
        local_count: 0,
        postcheck: None,
        fails_variant: None,
    }; 8];
    let mut out_func_code = [resilient_runtime::vm::Instr::Return; 64];
    let mut out_try_handlers = [resilient_runtime::vm::TryHandlerEntry::EMPTY; 1];
    let counts = resilient_runtime::vm::serde::decode_program(
        &blob,
        &mut out_main,
        &mut out_func_meta,
        &mut out_func_code,
        &mut out_try_handlers,
    )
    .expect("blob should decode as the function-table .rzbc format");
    assert_eq!(counts.func_count, 1, "one fn declaration → one table entry");

    let mut functions = [resilient_runtime::vm::FunctionDef {
        code: &[][..],
        arity: 0,
        local_count: 0,
        postcheck: None,
        fails_variant: None,
    }; 8];
    for (slot, meta) in functions
        .iter_mut()
        .zip(out_func_meta.iter())
        .take(counts.func_count)
    {
        *slot = resilient_runtime::vm::FunctionDef {
            code: &out_func_code[meta.offset as usize..(meta.offset + meta.len) as usize],
            arity: meta.arity,
            local_count: meta.local_count,
            postcheck: meta.postcheck,
            fails_variant: meta.fails_variant,
        };
    }
    let mut vm = resilient_runtime::vm::Vm::<32, 8, 4>::new();
    let embedded_result = vm
        .run_with_functions(
            &functions[..counts.func_count],
            &out_main[..counts.main_len],
        )
        .expect("embedded VM should run the decoded fn-calling program");
    assert_eq!(embedded_result, resilient_runtime::vm::Value::Int(3));

    let interpreter_src = dir.join("with_fn_print.rz");
    std::fs::write(
        &interpreter_src,
        "fn add(int a, int b) -> int {\n    return a + b;\n}\nprintln(add(1, 2));\n",
    )
    .unwrap();
    let interpreter_stdout = run_interpreter(&interpreter_src);
    assert_eq!(interpreter_stdout.lines().next(), Some("3"));

    let _ = std::fs::remove_dir_all(&dir);
}

/// RES-4075 (fn-support tail): decode a fn-table blob and run it —
/// the same dance as `build_decode_run_matches_interpreter_for_fn_declarations`,
/// shared by the TailCall/Pop round-trip tests below.
fn run_fn_blob(blob: &[u8]) -> resilient_runtime::vm::Value {
    let mut out_main = [resilient_runtime::vm::Instr::Return; 32];
    let mut out_func_meta = [resilient_runtime::vm::serde::DecodedFunctionMeta {
        offset: 0,
        len: 0,
        arity: 0,
        local_count: 0,
        postcheck: None,
        fails_variant: None,
    }; 8];
    let mut out_func_code = [resilient_runtime::vm::Instr::Return; 64];
    let mut out_try_handlers = [resilient_runtime::vm::TryHandlerEntry::EMPTY; 1];
    let counts = resilient_runtime::vm::serde::decode_program(
        blob,
        &mut out_main,
        &mut out_func_meta,
        &mut out_func_code,
        &mut out_try_handlers,
    )
    .expect("blob should decode as the function-table .rzbc format");

    let mut functions = [resilient_runtime::vm::FunctionDef {
        code: &[][..],
        arity: 0,
        local_count: 0,
        postcheck: None,
        fails_variant: None,
    }; 8];
    for (slot, meta) in functions
        .iter_mut()
        .zip(out_func_meta.iter())
        .take(counts.func_count)
    {
        *slot = resilient_runtime::vm::FunctionDef {
            code: &out_func_code[meta.offset as usize..(meta.offset + meta.len) as usize],
            arity: meta.arity,
            local_count: meta.local_count,
            postcheck: meta.postcheck,
            fails_variant: meta.fails_variant,
        };
    }
    let mut vm = resilient_runtime::vm::Vm::<32, 8, 4>::new();
    vm.run_with_functions(
        &functions[..counts.func_count],
        &out_main[..counts.main_len],
    )
    .expect("embedded VM should run the decoded program")
}

/// RES-4083 (D-E1 tail): the `try`/`fails` counterpart of
/// [`run_fn_blob`] — decodes the try-handler table too and runs on
/// [`resilient_runtime::vm::Vm::run_with_tries`].
fn run_fn_blob_with_tries(blob: &[u8]) -> resilient_runtime::vm::Value {
    let mut out_main = [resilient_runtime::vm::Instr::Return; 32];
    let mut out_func_meta = [resilient_runtime::vm::serde::DecodedFunctionMeta {
        offset: 0,
        len: 0,
        arity: 0,
        local_count: 0,
        postcheck: None,
        fails_variant: None,
    }; 8];
    let mut out_func_code = [resilient_runtime::vm::Instr::Return; 64];
    let mut out_try_handlers = [resilient_runtime::vm::TryHandlerEntry::EMPTY; 4];
    let counts = resilient_runtime::vm::serde::decode_program(
        blob,
        &mut out_main,
        &mut out_func_meta,
        &mut out_func_code,
        &mut out_try_handlers,
    )
    .expect("blob should decode as the function-table .rzbc format");

    let mut functions = [resilient_runtime::vm::FunctionDef {
        code: &[][..],
        arity: 0,
        local_count: 0,
        postcheck: None,
        fails_variant: None,
    }; 8];
    for (slot, meta) in functions
        .iter_mut()
        .zip(out_func_meta.iter())
        .take(counts.func_count)
    {
        *slot = resilient_runtime::vm::FunctionDef {
            code: &out_func_code[meta.offset as usize..(meta.offset + meta.len) as usize],
            arity: meta.arity,
            local_count: meta.local_count,
            postcheck: meta.postcheck,
            fails_variant: meta.fails_variant,
        };
    }
    let mut vm = resilient_runtime::vm::Vm::<32, 8, 4, 2>::new();
    vm.run_with_tries(
        &functions[..counts.func_count],
        &out_try_handlers[..counts.try_count],
        &out_main[..counts.main_len],
    )
    .expect("embedded VM should run the decoded try/fails program")
}

/// RES-4083 (D-E1 tail): `rz build` now accepts a program declaring
/// `fails` and using `try { } catch Variant { }` — the whole
/// parser -> typechecker -> compiler -> `rzbc_emit` -> embedded `Vm`
/// pipeline for checked-failure dispatch. The embedded VM
/// deterministically injects the declared checked failure on any
/// call made inside an active `try` (mirroring the host interpreter's
/// `RES-775` behavior), so `catch Timeout` always fires here.
#[test]
fn build_decode_run_matches_interpreter_for_fails_try_catch() {
    let dir = tmp_dir("fails_try");
    let src_body = "fn read_sensor(int addr) fails Timeout {\n    \
                     return addr;\n\
                     }\n\n\
                     fn caller(int addr) -> int {\n    \
                     if addr > 0 {\n        \
                     try {\n            \
                     let v = read_sensor(addr);\n            \
                     return v;\n        \
                     } catch Timeout {\n            \
                     return -1;\n        \
                     }\n    \
                     }\n    \
                     return -2;\n\
                     }\n\n";

    let embedded_src = dir.join("fails_try.rz");
    std::fs::write(&embedded_src, format!("{src_body}caller(42);\n")).unwrap();
    let out = dir.join("fails_try.rzbc");

    let (code, stderr) = run_build(&embedded_src, &out, "thumbv7em-none-eabihf");
    assert_eq!(
        code,
        Some(0),
        "fails/try/catch should build for embedded targets; stderr={stderr}"
    );

    let blob = std::fs::read(&out).expect("rz build should have written the .rzbc file");
    assert_eq!(
        run_fn_blob_with_tries(&blob),
        resilient_runtime::vm::Value::Int(-1),
        "the embedded VM deterministically injects the declared checked failure inside `try`, \
         so `catch Timeout` should fire"
    );

    let interpreter_src = dir.join("fails_try_print.rz");
    std::fs::write(
        &interpreter_src,
        format!("{src_body}println(caller(42));\n"),
    )
    .unwrap();
    let interpreter_stdout = run_interpreter(&interpreter_src);
    assert_eq!(
        interpreter_stdout.lines().next(),
        Some("-1"),
        "the tree-walking interpreter also injects the checked failure inside `try` (RES-775), \
         so both backends should agree on -1; full stdout={interpreter_stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// RES-4075: a tail-recursive fn — the host peephole rewrites the
/// self-recursive `Call; ReturnFromCall` into `TailCall`, which now
/// translates and runs in O(1) call-frame space (depth 50 with only
/// 4 frame slots).
#[test]
fn build_compiles_tail_recursive_fn_and_embedded_vm_runs_it() {
    let dir = tmp_dir("tail_rec");
    let src = dir.join("countdown.rz");
    std::fs::write(
        &src,
        "fn countdown(int n) -> int {\n    if n < 1 {\n        return 0;\n    }\n    return countdown(n - 1);\n}\ncountdown(50);\n",
    )
    .unwrap();
    let out = dir.join("countdown.rzbc");

    let (code, stderr) = run_build(&src, &out, "thumbv7em-none-eabihf");
    assert_eq!(
        code,
        Some(0),
        "tail-recursive fn should build for embedded targets; stderr={stderr}"
    );

    let blob = std::fs::read(&out).unwrap();
    assert_eq!(run_fn_blob(&blob), resilient_runtime::vm::Value::Int(0));

    let _ = std::fs::remove_dir_all(&dir);
}

/// RES-4075: a program with a discarded call-statement result — the
/// compiler emits `Op::Pop` after `f(1);`, which now translates.
#[test]
fn build_compiles_discarded_call_statement_and_embedded_vm_runs_it() {
    let dir = tmp_dir("pop_stmt");
    let src = dir.join("pop.rz");
    std::fs::write(
        &src,
        "fn f(int a) -> int {\n    return a + 1;\n}\nf(1);\nf(41);\n",
    )
    .unwrap();
    let out = dir.join("pop.rzbc");

    let (code, stderr) = run_build(&src, &out, "riscv32imac-unknown-none-elf");
    assert_eq!(
        code,
        Some(0),
        "discarded call statements should build; stderr={stderr}"
    );

    let blob = std::fs::read(&out).unwrap();
    assert_eq!(run_fn_blob(&blob), resilient_runtime::vm::Value::Int(42));

    let interpreter_src = dir.join("pop_print.rz");
    std::fs::write(
        &interpreter_src,
        "fn f(int a) -> int {\n    return a + 1;\n}\nf(1);\nprintln(f(41));\n",
    )
    .unwrap();
    let interpreter_stdout = run_interpreter(&interpreter_src);
    assert_eq!(interpreter_stdout.lines().next(), Some("42"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_missing_target_is_usage_error() {
    let dir = tmp_dir("missing_target");
    let src = dir.join("x.rz");
    std::fs::write(&src, "1;\n").unwrap();

    let output = Command::new(bin())
        .arg("build")
        .arg(&src)
        .output()
        .expect("spawn rz build");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--target"),
        "expected a --target usage hint; got: {stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
