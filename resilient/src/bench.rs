//! RES-2613: `bench "name" { body }` plus `rz bench`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::{Node, Parser, Token, imports, stdlib, typechecker};

const DEFAULT_WARMUP_ITERS: usize = 1;
const DEFAULT_RUN_ITERS: usize = 3;

/// Parse a `bench "name" { body };` statement.
pub(crate) fn parse(parser: &mut Parser) -> Node {
    let start_span = parser.span_at_current();
    parser.next_token();

    let name = match &parser.current_token {
        Token::StringLiteral(s) => s.clone(),
        other => {
            let tok = other.clone();
            parser.record_error(format!(
                "Expected string literal for benchmark name, found {}",
                tok
            ));
            parser.next_token();
            return Node::BenchBlock {
                name: String::new(),
                body: Box::new(Node::Block {
                    stmts: vec![],
                    span: start_span,
                }),
                span: start_span,
            };
        }
    };
    parser.next_token();

    if parser.current_token != Token::LeftBrace {
        let tok = parser.current_token.clone();
        parser.record_error(format!("Expected '{{' for bench body, found {}", tok));
        return Node::BenchBlock {
            name,
            body: Box::new(Node::Block {
                stmts: vec![],
                span: start_span,
            }),
            span: start_span,
        };
    }

    let body_block = parser.parse_block_statement();

    Node::BenchBlock {
        name,
        body: Box::new(body_block),
        span: start_span,
    }
}

pub fn dispatch_bench_subcommand(args: &[String]) -> Option<i32> {
    if args.get(1).map(|s| s.as_str()) != Some("bench") {
        return None;
    }

    let mut file: Option<PathBuf> = None;
    let mut baseline_ref: Option<String> = None;
    let mut filter: Option<String> = None;
    let mut summary_json_path: Option<PathBuf> = None;
    let mut warmup_iters = DEFAULT_WARMUP_ITERS;
    let mut run_iters = DEFAULT_RUN_ITERS;

    let mut i = 2;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--help" || arg == "-h" || arg == "help" {
            print_bench_help();
            return Some(0);
        } else if arg == "--baseline" {
            i += 1;
            if i >= args.len() {
                eprintln!("Error: --baseline requires a git ref");
                return Some(2);
            }
            baseline_ref = Some(args[i].clone());
        } else if let Some(value) = arg.strip_prefix("--baseline=") {
            baseline_ref = Some(value.to_string());
        } else if arg == "--filter" {
            i += 1;
            if i >= args.len() {
                eprintln!("Error: --filter requires a substring");
                return Some(2);
            }
            filter = Some(args[i].clone());
        } else if let Some(value) = arg.strip_prefix("--filter=") {
            filter = Some(value.to_string());
        } else if arg == "--summary-json" {
            i += 1;
            if i >= args.len() {
                eprintln!("Error: --summary-json requires a file path");
                return Some(2);
            }
            summary_json_path = Some(PathBuf::from(&args[i]));
        } else if let Some(value) = arg.strip_prefix("--summary-json=") {
            summary_json_path = Some(PathBuf::from(value));
        } else if arg == "--warmup" {
            i += 1;
            if i >= args.len() {
                eprintln!("Error: --warmup requires an integer");
                return Some(2);
            }
            match args[i].parse() {
                Ok(n) => warmup_iters = n,
                Err(_) => {
                    eprintln!("Error: --warmup requires an integer");
                    return Some(2);
                }
            }
        } else if let Some(value) = arg.strip_prefix("--warmup=") {
            match value.parse() {
                Ok(n) => warmup_iters = n,
                Err(_) => {
                    eprintln!("Error: --warmup requires an integer");
                    return Some(2);
                }
            }
        } else if arg == "--runs" {
            i += 1;
            if i >= args.len() {
                eprintln!("Error: --runs requires an integer");
                return Some(2);
            }
            match args[i].parse() {
                Ok(n) => run_iters = n,
                Err(_) => {
                    eprintln!("Error: --runs requires an integer");
                    return Some(2);
                }
            }
        } else if let Some(value) = arg.strip_prefix("--runs=") {
            match value.parse() {
                Ok(n) => run_iters = n,
                Err(_) => {
                    eprintln!("Error: --runs requires an integer");
                    return Some(2);
                }
            }
        } else if file.is_none() && !arg.starts_with('-') {
            file = Some(PathBuf::from(arg));
        } else {
            eprintln!("Error: unexpected argument `{arg}` to bench");
            return Some(2);
        }
        i += 1;
    }

    if run_iters == 0 {
        eprintln!("Error: --runs must be at least 1");
        return Some(2);
    }

    let Some(path) = file else {
        eprintln!("Error: `rz bench <file>` requires a file path");
        return Some(2);
    };

    let loaded = match load_program_from_path(&path) {
        Ok(loaded) => loaded,
        Err(err) => {
            eprintln!("Error: {err}");
            return Some(1);
        }
    };

    let benches = discover_benchmarks(&loaded.program, filter.as_deref());
    if benches.is_empty() {
        eprintln!("No benchmarks found in {}", path.display());
        return Some(1);
    }

    if let Some(dup) = find_duplicate_benchmark_name(&benches) {
        eprintln!("Error: duplicate benchmark name `{dup}`");
        return Some(1);
    }

    let current_results = match run_benchmarks(&loaded, &benches, warmup_iters, run_iters) {
        Ok(results) => results,
        Err(err) => {
            eprintln!("Error: {err}");
            return Some(1);
        }
    };

    let baseline_results = if let Some(reference) = baseline_ref.as_deref() {
        match load_program_from_git_ref(&path, reference) {
            Ok(loaded_baseline) => {
                let baseline_benches =
                    discover_benchmarks(&loaded_baseline.program, filter.as_deref());
                if baseline_benches.is_empty() {
                    eprintln!(
                        "Error: baseline `{reference}` has no benchmarks for {}",
                        path.display()
                    );
                    return Some(1);
                }
                match run_benchmarks(&loaded_baseline, &baseline_benches, warmup_iters, run_iters) {
                    Ok(results) => Some(results),
                    Err(err) => {
                        eprintln!("Error: baseline `{reference}` failed: {err}");
                        return Some(1);
                    }
                }
            }
            Err(err) => {
                eprintln!("Error: could not load baseline `{reference}`: {err}");
                return Some(1);
            }
        }
    } else {
        None
    };

    if let Some(output_path) = summary_json_path.as_deref()
        && let Err(err) = write_summary_json(
            output_path,
            &path,
            &current_results,
            baseline_results.as_ref(),
            warmup_iters,
            run_iters,
            baseline_ref.as_deref(),
        )
    {
        eprintln!("Error: could not write summary JSON: {err}");
        return Some(1);
    }

    print_results(
        &path,
        &current_results,
        baseline_results.as_ref(),
        warmup_iters,
        run_iters,
        baseline_ref.as_deref(),
        summary_json_path.as_deref(),
    );
    Some(0)
}

struct LoadedProgram {
    program: Node,
    std_bindings: Vec<(String, stdlib::StdBinding)>,
    source_label: String,
}

#[derive(Clone, Debug)]
struct BenchmarkCase {
    name: String,
    body: Node,
    line: usize,
    column: usize,
}

#[derive(Clone, Debug)]
struct BenchmarkStats {
    mean_ns: f64,
    median_ns: f64,
    stddev_ns: f64,
    min_ns: f64,
    max_ns: f64,
}

#[derive(Clone, Debug)]
struct BenchmarkResult {
    name: String,
    stats: BenchmarkStats,
}

fn print_bench_help() {
    println!(
        "Usage: rz bench <file> [--baseline <git-ref>] [--summary-json <path>] [--warmup N] [--runs N]"
    );
    println!();
    println!("Discover and run `bench \"name\" {{ ... }}` blocks.");
    println!();
    println!("Options:");
    println!("  --baseline <ref>   Compare mean ns/op against a git ref");
    println!("  --summary-json <path>  Write a stable JSON summary artifact");
    println!("  --warmup <N>       Warmup iterations before timing (default: 1)");
    println!("  --runs <N>         Timed iterations per benchmark (default: 3)");
    println!("  --filter <substr>  Only run benchmarks whose names contain <substr>");
}

fn load_program_from_path(path: &Path) -> Result<LoadedProgram, String> {
    let src =
        fs::read_to_string(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    load_program_from_source(&src, path)
}

fn load_program_from_git_ref(path: &Path, git_ref: &str) -> Result<LoadedProgram, String> {
    let repo_root = git_repo_root(path)?;
    let repo_relative = repo_relative_path(path, &repo_root)?;
    let spec = format!("{git_ref}:{}", repo_relative.display());
    let output = Command::new("git")
        .arg("-C")
        .arg(&repo_root)
        .arg("show")
        .arg(&spec)
        .output()
        .map_err(|e| format!("failed to invoke git: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git show {spec} failed: {stderr}"));
    }
    let src =
        String::from_utf8(output.stdout).map_err(|e| format!("baseline was not UTF-8: {e}"))?;
    load_program_from_source(&src, path)
}

fn load_program_from_source(src: &str, source_path: &Path) -> Result<LoadedProgram, String> {
    let (mut program, parse_errs) = crate::parse(src);
    if !parse_errs.is_empty() {
        return Err(parse_errs.join("\n"));
    }

    let base_dir = source_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut loaded: HashSet<PathBuf> = HashSet::new();
    if let Ok(canon) = fs::canonicalize(source_path) {
        loaded.insert(canon);
    }
    let mut std_imports = Vec::new();
    imports::expand_uses_with_std(&mut program, &base_dir, &mut loaded, &mut std_imports)
        .map_err(|e| format!("import error in {}: {e}", source_path.display()))?;

    let _ = crate::named_args::lower_program(&mut program);
    crate::default_params::lower_program(&mut program);
    crate::newtypes::lower_program(&mut program);
    crate::macros::lower_program(&mut program);

    let mut std_bindings = Vec::new();
    for import in &std_imports {
        let bindings = stdlib::resolve_std_import(&import.module, import.alias.as_deref())
            .map_err(|e| format!("import error in {}: {e}", source_path.display()))?;
        std_bindings.extend(bindings);
    }

    typechecker::TypeChecker::new()
        .check_program_with_source(&program, source_path.to_string_lossy().as_ref())
        .map_err(|e| format!("type error in {}: {e}", source_path.display()))?;

    Ok(LoadedProgram {
        program,
        std_bindings,
        source_label: source_path.display().to_string(),
    })
}

fn discover_benchmarks(program: &Node, filter: Option<&str>) -> Vec<BenchmarkCase> {
    let mut benches = Vec::new();
    let Node::Program(stmts) = program else {
        return benches;
    };
    for stmt in stmts {
        if let Node::BenchBlock {
            name, body, span, ..
        } = &stmt.node
        {
            if let Some(filter) = filter
                && !name.contains(filter)
            {
                continue;
            }
            benches.push(BenchmarkCase {
                name: name.clone(),
                body: body.as_ref().clone(),
                line: span.start.line,
                column: span.start.column,
            });
        }
    }
    benches
}

fn find_duplicate_benchmark_name(benches: &[BenchmarkCase]) -> Option<String> {
    let mut seen = HashSet::new();
    for bench in benches {
        if !seen.insert(bench.name.as_str()) {
            return Some(bench.name.clone());
        }
    }
    None
}

fn run_benchmarks(
    loaded: &LoadedProgram,
    benches: &[BenchmarkCase],
    warmup_iters: usize,
    run_iters: usize,
) -> Result<Vec<BenchmarkResult>, String> {
    let mut results = Vec::with_capacity(benches.len());
    for bench in benches {
        for _ in 0..warmup_iters {
            crate::execute_benchmark_body(
                &loaded.program,
                &bench.body,
                &loaded.std_bindings,
                &loaded.source_label,
            )
            .map_err(|e| {
                format!(
                    "benchmark `{}` failed during warmup at {}:{}: {}",
                    bench.name, bench.line, bench.column, e
                )
            })?;
        }

        let mut samples_ns = Vec::with_capacity(run_iters);
        for _ in 0..run_iters {
            let started = Instant::now();
            crate::execute_benchmark_body(
                &loaded.program,
                &bench.body,
                &loaded.std_bindings,
                &loaded.source_label,
            )
            .map_err(|e| {
                format!(
                    "benchmark `{}` failed at {}:{}: {}",
                    bench.name, bench.line, bench.column, e
                )
            })?;
            samples_ns.push(started.elapsed().as_secs_f64() * 1_000_000_000.0);
        }

        results.push(BenchmarkResult {
            name: bench.name.clone(),
            stats: compute_stats(&samples_ns),
        });
    }
    Ok(results)
}

fn compute_stats(samples_ns: &[f64]) -> BenchmarkStats {
    debug_assert!(!samples_ns.is_empty());

    let mut sorted = samples_ns.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let mean_ns = samples_ns.iter().sum::<f64>() / samples_ns.len() as f64;
    let median_ns = if sorted.len() % 2 == 1 {
        sorted[sorted.len() / 2]
    } else {
        let hi = sorted.len() / 2;
        (sorted[hi - 1] + sorted[hi]) / 2.0
    };
    let variance = samples_ns
        .iter()
        .map(|sample| {
            let delta = sample - mean_ns;
            delta * delta
        })
        .sum::<f64>()
        / samples_ns.len() as f64;

    BenchmarkStats {
        mean_ns,
        median_ns,
        stddev_ns: variance.sqrt(),
        min_ns: sorted[0],
        max_ns: sorted[sorted.len() - 1],
    }
}

fn print_results(
    source_path: &Path,
    current: &[BenchmarkResult],
    baseline: Option<&Vec<BenchmarkResult>>,
    warmup_iters: usize,
    run_iters: usize,
    baseline_ref: Option<&str>,
    summary_json_path: Option<&Path>,
) {
    println!("Benchmark results (warmup: {warmup_iters}, runs: {run_iters})");

    let baseline_by_name: HashMap<&str, &BenchmarkResult> = baseline
        .into_iter()
        .flat_map(|results| results.iter())
        .map(|result| (result.name.as_str(), result))
        .collect();

    if let Some(reference) = baseline_ref {
        println!("baseline: {reference}");
    }

    println!(
        "{:<24} {:>12} {:>12} {:>12} {:>12} {:>12} {:>14} {:>10}",
        "Benchmark", "mean ns/op", "median", "stddev", "min", "max", "baseline", "delta"
    );

    for result in current {
        let (baseline_mean, delta) = if let Some(base) = baseline_by_name.get(result.name.as_str())
        {
            let delta = if base.stats.mean_ns == 0.0 {
                None
            } else {
                Some(((result.stats.mean_ns - base.stats.mean_ns) / base.stats.mean_ns) * 100.0)
            };
            (format_ns(base.stats.mean_ns), delta)
        } else {
            ("-".to_string(), None)
        };
        let delta_text = match delta {
            Some(value) => format!("{value:+.1}%"),
            None => "-".to_string(),
        };
        println!(
            "{:<24} {:>12} {:>12} {:>12} {:>12} {:>12} {:>14} {:>10}",
            truncate_name(&result.name, 24),
            format_ns(result.stats.mean_ns),
            format_ns(result.stats.median_ns),
            format_ns(result.stats.stddev_ns),
            format_ns(result.stats.min_ns),
            format_ns(result.stats.max_ns),
            baseline_mean,
            delta_text,
        );
    }

    println!();
    println!("summary.source={}", source_path.display());
    println!("summary.benchmarks={}", current.len());
    println!("summary.warmup_iters={warmup_iters}");
    println!("summary.run_iters={run_iters}");
    if let Some(reference) = baseline_ref {
        println!("summary.baseline_ref={reference}");
    }
    if let Some(path) = summary_json_path {
        println!("artifact.summary_json={}", path.display());
    }
}

fn write_summary_json(
    output_path: &Path,
    source_path: &Path,
    current: &[BenchmarkResult],
    baseline: Option<&Vec<BenchmarkResult>>,
    warmup_iters: usize,
    run_iters: usize,
    baseline_ref: Option<&str>,
) -> Result<(), String> {
    let baseline_by_name: HashMap<&str, &BenchmarkResult> = baseline
        .into_iter()
        .flat_map(|results| results.iter())
        .map(|result| (result.name.as_str(), result))
        .collect();

    let benches: Vec<_> = current
        .iter()
        .map(|result| {
            let (baseline_mean_ns, delta_pct) = if let Some(base) =
                baseline_by_name.get(result.name.as_str())
            {
                let delta_pct = if base.stats.mean_ns == 0.0 {
                    None
                } else {
                    Some(((result.stats.mean_ns - base.stats.mean_ns) / base.stats.mean_ns) * 100.0)
                };
                (Some(base.stats.mean_ns), delta_pct)
            } else {
                (None, None)
            };
            serde_json::json!({
                "name": result.name,
                "mean_ns": result.stats.mean_ns,
                "median_ns": result.stats.median_ns,
                "stddev_ns": result.stats.stddev_ns,
                "min_ns": result.stats.min_ns,
                "max_ns": result.stats.max_ns,
                "baseline_mean_ns": baseline_mean_ns,
                "delta_pct": delta_pct,
            })
        })
        .collect();

    let summary = serde_json::json!({
        "schema_version": 1,
        "source": source_path.display().to_string(),
        "warmup_iters": warmup_iters,
        "run_iters": run_iters,
        "benchmark_count": current.len(),
        "baseline_ref": baseline_ref,
        "benchmarks": benches,
    });

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(&summary)
        .map_err(|e| format!("could not serialize summary JSON: {e}"))?;
    fs::write(output_path, text)
        .map_err(|e| format!("could not write {}: {e}", output_path.display()))?;
    Ok(())
}

fn format_ns(value: f64) -> String {
    format!("{value:.0}")
}

fn truncate_name(name: &str, width: usize) -> String {
    if name.chars().count() <= width {
        return name.to_string();
    }
    let mut out = String::new();
    for ch in name.chars().take(width.saturating_sub(1)) {
        out.push(ch);
    }
    out.push('~');
    out
}

fn git_repo_root(path: &Path) -> Result<PathBuf, String> {
    let start_dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };
    let output = Command::new("git")
        .arg("-C")
        .arg(&start_dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| format!("failed to invoke git: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git rev-parse failed: {stderr}"));
    }
    let root =
        String::from_utf8(output.stdout).map_err(|e| format!("git output was not UTF-8: {e}"))?;
    Ok(PathBuf::from(root.trim()))
}

fn repo_relative_path(path: &Path, repo_root: &Path) -> Result<PathBuf, String> {
    let absolute = fs::canonicalize(path)
        .map_err(|e| format!("could not canonicalize {}: {e}", path.display()))?;
    absolute
        .strip_prefix(repo_root)
        .map(Path::to_path_buf)
        .map_err(|_| {
            format!(
                "{} is not inside git repo {}",
                absolute.display(),
                repo_root.display()
            )
        })
}

#[cfg(test)]
mod tests {
    use crate::parse;

    #[test]
    fn parse_bench_basic() {
        let src = r#"bench "fibonacci" { fibonacci(30); }"#;
        let (program, errors) = parse(src);
        assert!(errors.is_empty(), "unexpected parse errors: {:?}", errors);
        match &program {
            crate::Node::Program(stmts) => {
                assert_eq!(stmts.len(), 1);
                match &stmts[0].node {
                    crate::Node::BenchBlock { name, .. } => {
                        assert_eq!(name, "fibonacci");
                    }
                    other => panic!("expected BenchBlock, got {:?}", other),
                }
            }
            _ => panic!("expected Program"),
        }
    }

    #[test]
    fn parse_bench_with_multiple_statements() {
        let src = r#"
            bench "multi" {
                int x = 5;
                int y = 10;
                println(x + y);
            }
        "#;
        let (program, errors) = parse(src);
        assert!(errors.is_empty(), "unexpected parse errors: {:?}", errors);
        match &program {
            crate::Node::Program(stmts) => {
                assert_eq!(stmts.len(), 1);
                match &stmts[0].node {
                    crate::Node::BenchBlock { name, body, .. } => {
                        assert_eq!(name, "multi");
                        if let crate::Node::Block { stmts, .. } = &**body {
                            assert!(stmts.len() >= 3);
                        } else {
                            panic!("expected Block body");
                        }
                    }
                    other => panic!("expected BenchBlock, got {:?}", other),
                }
            }
            _ => panic!("expected Program"),
        }
    }

    #[test]
    fn discover_bench_finds_multiple() {
        let src = r#"
            bench "first" { int a = 1; }
            bench "second" { int b = 2; }
            bench "third" { int c = 3; }
        "#;
        let (program, _) = parse(src);
        let benches = super::discover_benchmarks(&program, None);
        assert_eq!(benches.len(), 3);
        assert_eq!(benches[0].name, "first");
        assert_eq!(benches[1].name, "second");
        assert_eq!(benches[2].name, "third");
    }

    #[test]
    fn compute_stats_reports_mean_median_and_bounds() {
        let stats = super::compute_stats(&[10.0, 20.0, 30.0, 40.0]);
        assert_eq!(stats.mean_ns, 25.0);
        assert_eq!(stats.median_ns, 25.0);
        assert_eq!(stats.min_ns, 10.0);
        assert_eq!(stats.max_ns, 40.0);
        assert!(stats.stddev_ns > 0.0);
    }
}
