//! RES-2613: Benchmark framework — `bench "name" { body }` blocks.
//!
//! ## Syntax
//!
//! ```text
//! bench "fibonacci" {
//!     fibonacci(30);
//! }
//!
//! bench "sort 1000 elements" {
//!     array_sort([3, 1, 2]);
//! }
//! ```
//!
//! ## Runtime behaviour
//!
//! `bench` blocks are **silently skipped** during normal `rz file.rz` execution.
//! Run `rz bench file.rz` to collect and time them.
//!
//! ## `rz bench` output
//!
//! ```text
//! benchmark "fibonacci"         100 iters   mean: 1.23 ms   min: 1.10 ms   max: 1.45 ms
//! benchmark "sort 1000 elements"  100 iters   mean: 0.42 ms   min: 0.38 ms   max: 0.55 ms
//! ```

use crate::{Node, Parser, Token};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Parser helper
// ---------------------------------------------------------------------------

/// Parse `bench "name" { body }` and return a `Node::BenchBlock`.
pub(crate) fn parse_bench_block(parser: &mut Parser) -> Node {
    let span = parser.span_at_current();
    parser.next_token(); // consume `bench`

    let name = match &parser.current_token.clone() {
        Token::StringLiteral(s) => {
            let s = s.clone();
            parser.next_token(); // consume name string
            s
        }
        other => {
            parser.record_error(format!(
                "bench: expected string literal for name, got {}",
                other
            ));
            String::from("unnamed")
        }
    };

    // parse_block_statement handles the `{` … `}` delimiters itself.
    let body = if parser.current_token == Token::LeftBrace {
        parser.parse_block_statement()
    } else {
        parser.record_error("bench: expected `{` after name".to_string());
        Node::Block {
            stmts: Vec::new(),
            span,
        }
    };

    Node::BenchBlock {
        name,
        body: Box::new(body),
        span,
    }
}

// ---------------------------------------------------------------------------
// Bench subcommand dispatch
// ---------------------------------------------------------------------------

/// Handles `rz bench <file>`. Returns `Some(exit_code)` when the subcommand
/// was matched (whether it succeeded or failed), `None` to fall through.
pub(crate) fn dispatch_bench_subcommand(args: &[String]) -> Option<i32> {
    if args.get(1).map(|s| s.as_str()) != Some("bench") {
        return None;
    }

    let file = match args.get(2) {
        Some(f) => f.clone(),
        None => {
            eprintln!("usage: rz bench <file.rz> [--iters N]");
            return Some(1);
        }
    };

    let iters: u64 = args
        .windows(2)
        .find(|w| w[0] == "--iters")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(100);

    let src = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("bench: cannot read {file}: {e}");
            return Some(1);
        }
    };

    let (program, _errors) = crate::parse(&src);
    let blocks = collect_bench_blocks(&program);

    if blocks.is_empty() {
        println!("No bench blocks found in {file}.");
        return Some(0);
    }

    println!(
        "{:<40}  {:>8}  {:>12}  {:>12}  {:>12}",
        "benchmark", "iters", "mean", "min", "max"
    );
    println!("{}", "-".repeat(90));

    for (name, body) in &blocks {
        let times = run_bench(body, iters);
        let mean = mean_duration(&times);
        let min = times.iter().min().copied().unwrap_or_default();
        let max = times.iter().max().copied().unwrap_or_default();

        println!(
            "{:<40}  {:>8}  {:>12}  {:>12}  {:>12}",
            format!("{:?}", name),
            iters,
            fmt_duration(mean),
            fmt_duration(min),
            fmt_duration(max),
        );
    }

    Some(0)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Collect all `Node::BenchBlock { name, body }` from the program's top-level
/// statements (bench blocks at non-top-level are silently ignored).
fn collect_bench_blocks(program: &Node) -> Vec<(String, Node)> {
    let Node::Program(stmts) = program else {
        return Vec::new();
    };
    stmts
        .iter()
        .filter_map(|s| {
            if let Node::BenchBlock { name, body, .. } = &s.node {
                Some((name.clone(), *body.clone()))
            } else {
                None
            }
        })
        .collect()
}

/// Run `body` `iters` times and return per-iteration durations.
fn run_bench(body: &Node, iters: u64) -> Vec<Duration> {
    use crate::Interpreter;

    let mut times = Vec::with_capacity(iters as usize);
    for _ in 0..iters {
        let mut interp = Interpreter::new();
        let start = Instant::now();
        let _ = interp.eval(body);
        times.push(start.elapsed());
    }
    times
}

fn mean_duration(times: &[Duration]) -> Duration {
    if times.is_empty() {
        return Duration::ZERO;
    }
    times.iter().sum::<Duration>() / times.len() as u32
}

fn fmt_duration(d: Duration) -> String {
    let nanos = d.as_nanos();
    if nanos < 1_000 {
        format!("{nanos} ns")
    } else if nanos < 1_000_000 {
        format!("{:.2} µs", nanos as f64 / 1_000.0)
    } else if nanos < 1_000_000_000 {
        format!("{:.2} ms", nanos as f64 / 1_000_000.0)
    } else {
        format!("{:.2} s", d.as_secs_f64())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> String {
        let r = run_program(src);
        assert!(r.ok, "program failed: {:?}", r.errors);
        r.stdout
    }

    #[test]
    fn bench_block_is_skipped_in_normal_execution() {
        let out = run(r#"
println("before");
bench "noop" {
    println("inside bench");
}
println("after");
"#);
        assert!(out.contains("before"), "got: {out:?}");
        assert!(out.contains("after"), "got: {out:?}");
        assert!(
            !out.contains("inside bench"),
            "bench block should be skipped: {out:?}"
        );
    }

    #[test]
    fn multiple_bench_blocks_are_all_skipped() {
        let out = run(r#"
let x = 1;
bench "first" { let y = 2; }
bench "second" { let z = 3; }
println(to_string(x));
"#);
        assert!(out.contains("1"), "got: {out:?}");
    }
}
