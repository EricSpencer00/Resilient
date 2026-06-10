//! `rz test` subcommand — discover and run `fn test_*()` functions.
//!
//! Test discovery walks the parsed AST for top-level `Function` nodes
//! whose name starts with `test_` and that take zero parameters.  Each
//! test runs in an isolated `Interpreter` scope with stdlib bindings
//! injected (so `use std::testing; testing::assert_eq(...)` works).
//! A runtime error or assertion failure counts as a test failure.
//!
//! All logic lives in this file; `lib.rs` contributes only a `mod`
//! declaration and a dispatch call in `run_cli()`.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{Interpreter, Node, imports, output_sink, stdlib};

/// Entry point called from `run_cli()`.  Returns `Some(exit_code)` when
/// the first CLI arg is `"test"`, `None` otherwise (fall through).
pub fn dispatch_test_subcommand(args: &[String]) -> Option<i32> {
    if args.get(1).map(|s| s.as_str()) != Some("test") {
        return None;
    }

    let mut target: Option<String> = None;
    let mut filter: Option<String> = None;
    let mut i = 2;
    while i < args.len() {
        let a = &args[i];
        if a == "--filter" {
            i += 1;
            if i >= args.len() {
                eprintln!("Error: --filter requires a substring argument");
                return Some(2);
            }
            filter = Some(args[i].clone());
        } else if let Some(f) = a.strip_prefix("--filter=") {
            filter = Some(f.to_string());
        } else if a == "--help" || a == "-h" || a == "help" {
            print_test_help();
            return Some(0);
        } else if target.is_none() {
            target = Some(a.clone());
        } else {
            eprintln!("Error: unexpected argument `{a}` to test");
            return Some(2);
        }
        i += 1;
    }

    let paths = match resolve_target(target.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error: {e}");
            return Some(2);
        }
    };

    if paths.is_empty() {
        eprintln!("No .rz files found");
        return Some(1);
    }

    let mut total = 0usize;
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut failure_details: Vec<String> = Vec::new();

    for path in &paths {
        match run_tests_in_file(path, filter.as_deref()) {
            Ok(result) => {
                total += result.total;
                passed += result.passed;
                failed += result.failed;
                failure_details.extend(result.failure_details);
            }
            Err(e) => {
                eprintln!("Error processing {}: {e}", path.display());
                failed += 1;
                total += 1;
            }
        }
    }

    // Print failure details at the end for easy scanning.
    if !failure_details.is_empty() {
        eprintln!();
        eprintln!("failures:");
        for detail in &failure_details {
            eprintln!("{detail}");
        }
    }

    println!();
    println!(
        "{total} test{}: {passed} passed, {failed} failed",
        if total == 1 { "" } else { "s" }
    );

    if failed > 0 { Some(1) } else { Some(0) }
}

// ── helpers ────────────────────────────────────────────────────────────

fn print_test_help() {
    println!("Usage: rz test [<file|dir>] [--filter <substring>]");
    println!();
    println!("Discover and run fn test_*() functions in .rz files.");
    println!();
    println!("Options:");
    println!("  <file>              Run tests in a single .rz file");
    println!("  <dir>               Discover all .rz files recursively");
    println!("  (no argument)       Discover from the current directory");
    println!("  --filter <substr>   Only run tests whose name contains <substr>");
}

/// Resolve the CLI target into a list of `.rz` file paths.
fn resolve_target(target: Option<&str>) -> Result<Vec<PathBuf>, String> {
    let path = match target {
        Some(t) => PathBuf::from(t),
        None => {
            std::env::current_dir().map_err(|e| format!("cannot read current directory: {e}"))?
        }
    };

    if path.is_file() {
        return Ok(vec![path]);
    }

    if path.is_dir() {
        let mut files = Vec::new();
        collect_rz_files(&path, &mut files)?;
        files.sort();
        return Ok(files);
    }

    Err(format!("{} is not a file or directory", path.display()))
}

fn collect_rz_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries =
        fs::read_dir(dir).map_err(|e| format!("cannot read directory {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("directory entry error: {e}"))?;
        let p = entry.path();
        if p.is_dir() {
            collect_rz_files(&p, out)?;
        } else if p.extension().and_then(|s| s.to_str()) == Some("rz") {
            out.push(p);
        }
    }
    Ok(())
}

struct FileTestResult {
    total: usize,
    passed: usize,
    failed: usize,
    failure_details: Vec<String>,
}

/// Parse one `.rz` file, discover `fn test_*()` functions, and run each
/// in its own `Interpreter`.
fn run_tests_in_file(path: &Path, filter: Option<&str>) -> Result<FileTestResult, String> {
    let src =
        fs::read_to_string(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let (mut program, parse_errs) = crate::parse(&src);

    if !parse_errs.is_empty() {
        let joined = parse_errs.join("\n");
        return Err(format!("parse errors in {}:\n{joined}", path.display()));
    }

    // Resolve `use` imports (especially `use std::testing;`).
    let base_dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut loaded: HashSet<PathBuf> = HashSet::new();
    if let Ok(canon) = fs::canonicalize(path) {
        loaded.insert(canon);
    }
    let mut std_imports = Vec::new();
    if let Err(e) =
        imports::expand_uses_with_std(&mut program, &base_dir, &mut loaded, &mut std_imports)
    {
        return Err(format!("import error in {}: {e}", path.display()));
    }

    let mut std_bindings = Vec::new();
    for si in &std_imports {
        match stdlib::resolve_std_import(&si.module, si.alias.as_deref()) {
            Ok(bindings) => std_bindings.extend(bindings),
            Err(e) => return Err(format!("import error in {}: {e}", path.display())),
        }
    }

    // Re-run lowering passes that `parse()` already ran on the original
    // source but that need a second pass after `expand_uses` spliced in
    // imported definitions.
    let _ = crate::named_args::lower_program(&mut program);
    crate::default_params::lower_program(&mut program);
    crate::newtypes::lower_program(&mut program);
    crate::macros::lower_program(&mut program);

    // Discover test functions.
    let test_names = discover_tests(&program, filter);

    let file_display = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_else(|| path.to_str().unwrap_or("?"));

    let mut result = FileTestResult {
        total: 0,
        passed: 0,
        failed: 0,
        failure_details: Vec::new(),
    };

    for test_name in &test_names {
        result.total += 1;
        match run_single_test(&program, &std_bindings, test_name) {
            Ok(()) => {
                println!("test {test_name} ... ok");
                result.passed += 1;
            }
            Err(e) => {
                println!("test {test_name} ... FAIL");
                let detail = format!("  {file_display}: {test_name}: {e}");
                result.failure_details.push(detail);
                result.failed += 1;
            }
        }
    }

    Ok(result)
}

/// Walk the top-level AST and collect names of `fn test_*()`
/// (zero-parameter functions whose name starts with `test_`).
fn discover_tests(program: &Node, filter: Option<&str>) -> Vec<String> {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return Vec::new(),
    };

    let mut names = Vec::new();
    for stmt in stmts {
        if let Node::Function {
            name, parameters, ..
        } = &stmt.node
            && name.starts_with("test_")
            && parameters.is_empty()
        {
            if let Some(f) = filter
                && !name.contains(f)
            {
                continue;
            }
            names.push(name.clone());
        }
    }
    names
}

/// Run a single test function by name.  Sets up a fresh `Interpreter`,
/// evaluates the entire program (which hoists all `fn` defs), then
/// calls the named test function with zero arguments.
fn run_single_test(
    program: &Node,
    std_bindings: &[(String, stdlib::StdBinding)],
    test_name: &str,
) -> Result<(), String> {
    // Capture stdout so test println!s don't leak into the harness output.
    let (eval_result, _captured) = output_sink::with_captured_output(|| {
        let mut interp = Interpreter::new();
        stdlib::inject_std_bindings(std_bindings, &interp.env);

        // Evaluate the program to hoist all function definitions.
        interp.eval(program)?;

        // Look up the test function and call it.
        let func = interp
            .env
            .get(test_name)
            .ok_or_else(|| format!("test function `{test_name}` not found after evaluation"))?;
        interp.apply_function(&func, Vec::new())?;
        Ok(())
    });
    eval_result
}
