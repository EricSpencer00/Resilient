//! RES-2613: `bench "name" { body }` — benchmark framework.
//!
//! Provides syntax and runtime support for defining and running benchmarks.
//! Bench blocks are silently skipped during normal execution (like `#[cfg(test)]`)
//! but discovered and timed by the `rz bench` subcommand.
//!
//! ## Syntax
//!
//! ```text
//! bench "name" { statements }
//! ```
//!
//! The name must be a string literal. The body is a sequence of statements
//! executed once per iteration.
//!
//! ## Feature isolation
//!
//! All logic lives here. Core files (`lib.rs`, `typechecker.rs`,
//! `lexer_logos.rs`) have only the minimal extension-point entries:
//! one `Token::Bench` variant, one keyword mapping, one
//! `Node::BenchBlock` variant, one `parse_statement` dispatch arm,
//! and one `<EXTENSION_PASSES>` call.

use crate::{Node, Parser, Token};

/// Parse a `bench "name" { body };` statement.
///
/// Called from `Parser::parse_statement` when the current token is
/// `Token::Bench`. Consumes the entire statement including the trailing block.
pub(crate) fn parse(parser: &mut Parser) -> Node {
    let start_span = parser.span_at_current();
    parser.next_token(); // skip `bench`

    // Parse the name (must be a string literal)
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
    parser.next_token(); // skip the name

    // Expect a left brace
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

    // Parse the block body using the standard block parser
    let body_block = parser.parse_block_statement();

    Node::BenchBlock {
        name,
        body: Box::new(body_block),
        span: start_span,
    }
}

/// Scan the program for all benchmark blocks and return their metadata.
///
/// Called by the `rz bench` subcommand to discover benchmarks before running them.
#[allow(dead_code)]
pub(crate) fn discover_benchmarks(program: &Node) -> Vec<BenchmarkMetadata> {
    let mut benches = Vec::new();
    discover_benchmarks_recursive(program, &mut benches);
    benches
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BenchmarkMetadata {
    pub name: String,
    pub line: usize,
    pub column: usize,
}

#[allow(dead_code)]
fn discover_benchmarks_recursive(node: &Node, benches: &mut Vec<BenchmarkMetadata>) {
    match node {
        Node::Program(stmts) => {
            for stmt in stmts {
                discover_benchmarks_recursive(&stmt.node, benches);
            }
        }
        Node::BenchBlock { name, span, .. } => {
            benches.push(BenchmarkMetadata {
                name: name.clone(),
                line: span.start.line,
                column: span.start.column,
            });
        }
        _ => {
            // For now, don't recurse into other node types to find nested benches
            // (benches at top level only). This can be extended in future tickets.
        }
    }
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
                            // Should have at least 3 statements (the three inside)
                            assert!(
                                stmts.len() >= 3,
                                "expected at least 3 statements, got {}",
                                stmts.len()
                            );
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
    fn discover_bench_finds_single() {
        let src = r#"bench "simple" { int x = 1; }"#;
        let (program, _) = parse(src);
        let benches = super::discover_benchmarks(&program);
        assert_eq!(benches.len(), 1);
        assert_eq!(benches[0].name, "simple");
    }

    #[test]
    fn discover_bench_finds_multiple() {
        let src = r#"
            bench "first" { int a = 1; }
            bench "second" { int b = 2; }
            bench "third" { int c = 3; }
        "#;
        let (program, _) = parse(src);
        let benches = super::discover_benchmarks(&program);
        assert_eq!(benches.len(), 3);
        assert_eq!(benches[0].name, "first");
        assert_eq!(benches[1].name, "second");
        assert_eq!(benches[2].name, "third");
    }
}
