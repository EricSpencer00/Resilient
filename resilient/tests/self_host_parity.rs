use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn parse_json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|err| panic!("invalid JSON `{text}`: {err}"))
}

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn corpus_dir(kind: &str) -> PathBuf {
    repo_root()
        .join("self-host")
        .join("parity_corpus")
        .join(kind)
}

fn corpus_files(kind: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(corpus_dir(kind))
        .expect("corpus dir exists")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rz"))
        .collect();
    files.sort();
    files
}

fn temp_path(stem: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "res781_{}_{}_{}.{}",
        std::process::id(),
        stem,
        nanos,
        ext
    ))
}

fn run_rust_dump_tokens(source: &Path) -> String {
    let output = Command::new(bin())
        .arg("--dump-tokens")
        .arg(source)
        .output()
        .expect("spawn rz --dump-tokens");
    assert!(
        output.status.success(),
        "rust token dump failed for {}:\n{}",
        source.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    normalize_rust_tokens(&String::from_utf8_lossy(&output.stdout))
}

fn run_self_host_lexer(source: &Path) -> String {
    let lexer = repo_root().join("self-host/lexer.rz");
    let output = Command::new(bin())
        .arg(lexer)
        .env("SELF_HOST_INPUT", source)
        .output()
        .expect("spawn self-host lexer");
    assert!(
        output.status.success(),
        "self-host lexer failed for {}:\nstdout={}\nstderr={}",
        source.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    normalize_self_host_stream(&String::from_utf8_lossy(&output.stdout))
}

fn run_rust_dump_ast(source: &Path) -> String {
    let output = Command::new(bin())
        .arg("--dump-ast-json")
        .arg(source)
        .output()
        .expect("spawn rz --dump-ast-json");
    assert!(
        output.status.success(),
        "rust AST dump failed for {}:\n{}",
        source.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn run_self_host_parser(tokens: &str, label: &str) -> String {
    let tokens_path = temp_path(label, "tokens.txt");
    fs::write(&tokens_path, tokens).expect("write temp token stream");
    let parser = repo_root().join("self-host/parser.rz");
    let output = Command::new(bin())
        .arg(parser)
        .env("SELF_HOST_TOKENS", &tokens_path)
        .output()
        .expect("spawn self-host parser");
    let _ = fs::remove_file(&tokens_path);
    assert!(
        output.status.success(),
        "self-host parser failed for {label}:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    normalize_self_host_stream(&String::from_utf8_lossy(&output.stdout))
}

fn normalize_self_host_stream(stdout: &str) -> String {
    stdout
        .lines()
        .filter(|line| !line.starts_with("seed="))
        .filter(|line| *line != "Program executed successfully")
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_rust_tokens(stdout: &str) -> String {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(normalize_rust_token_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_rust_token_line(line: &str) -> String {
    let (loc, rest) = line
        .split_once("  ")
        .unwrap_or_else(|| panic!("unexpected token line format: {line}"));
    let (kind, payload_start) = if let Some(idx) = rest.rfind(")(\"") {
        (&rest[..idx + 1], idx + 1)
    } else {
        let idx = rest
            .find("(\"")
            .unwrap_or_else(|| panic!("missing lexeme payload start: {line}"));
        (&rest[..idx], idx)
    };
    let payload_end = rest
        .rfind(')')
        .unwrap_or_else(|| panic!("missing lexeme payload end: {line}"));
    assert!(
        payload_start < payload_end,
        "bad lexeme payload bounds: {line}"
    );
    let mut lexeme = &rest[payload_start + 1..payload_end];
    if lexeme.starts_with('"') && lexeme.ends_with('"') && lexeme.len() >= 2 {
        lexeme = &lexeme[1..lexeme.len() - 1];
    }
    let (line_no, col_no) = loc
        .split_once(':')
        .unwrap_or_else(|| panic!("missing location separator: {line}"));
    let decoded_lexeme = lexeme
        .replace("\\n", "\n")
        .replace("\\\"", "\"")
        .replace("\\\\", "\\");
    let (bucket, rendered_lexeme) = map_rust_token_kind(kind, &decoded_lexeme);
    format!("{bucket} {rendered_lexeme} {line_no} {col_no}")
}

fn map_rust_token_kind(kind: &str, lexeme: &str) -> (&'static str, String) {
    match kind {
        "Function" | "Function(\"fn\")" => ("KW", "fn".to_string()),
        "If" | "If(\"if\")" => ("KW", "if".to_string()),
        "Else" | "Else(\"else\")" => ("KW", "else".to_string()),
        "Return" | "Return(\"return\")" => ("KW", "return".to_string()),
        "Let" | "Let(\"let\")" => ("KW", "let".to_string()),
        "True" | "True(\"true\")" => ("KW", "true".to_string()),
        "False" | "False(\"false\")" => ("KW", "false".to_string()),
        kind if kind.starts_with("Identifier(") => ("IDENT", lexeme.to_string()),
        kind if kind.starts_with("StringLiteral(") => ("STRING", lexeme.to_string()),
        kind if kind.starts_with("IntLiteral(") || kind.starts_with("Integer(") => {
            ("INT", lexeme.to_string())
        }
        "LeftParen" => ("PUNCT", "(".to_string()),
        "RightParen" => ("PUNCT", ")".to_string()),
        "LeftBrace" => ("PUNCT", "{".to_string()),
        "RightBrace" => ("PUNCT", "}".to_string()),
        "Comma" => ("PUNCT", ",".to_string()),
        "Semicolon" => ("PUNCT", ";".to_string()),
        "Plus" => ("OP", "+".to_string()),
        "Greater" | "GreaterThan" => ("OP", ">".to_string()),
        "Arrow" | "Arrow(\"->\")" => ("OP", "->".to_string()),
        "Assign" => ("OP", "=".to_string()),
        "Eof" => ("EOF", String::new()),
        other => panic!("unmapped Rust token kind `{other}` for lexeme `{lexeme}`"),
    }
}

fn first_rust_parse_error(stderr: &str) -> Option<String> {
    stderr.lines().find_map(|line| {
        let mut parts = line.splitn(4, ':');
        let line_no = parts.next()?;
        let col_no = parts.next()?;
        if line_no.parse::<usize>().is_ok() && col_no.parse::<usize>().is_ok() {
            Some(line.trim().to_string())
        } else {
            None
        }
    })
}

fn first_self_host_parse_error(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .find(|line| line.starts_with("parse error:"))
        .map(|line| line.trim().to_string())
}

fn extract_line_col(msg: &str) -> Option<(usize, usize)> {
    let mut parts = msg.splitn(3, ':');
    let line_no = parts.next()?.trim().parse().ok();
    let col_no = parts.next()?.trim().parse().ok();
    if let (Some(line_no), Some(col_no)) = (line_no, col_no) {
        return Some((line_no, col_no));
    }
    let at = msg.rfind(" at ")?;
    let coords = &msg[at + 4..];
    let mut parts = coords.split(':');
    let line_no = parts.next()?.trim().parse().ok()?;
    let col_no = parts.next()?.trim().parse().ok()?;
    Some((line_no, col_no))
}

#[test]
fn self_host_success_corpus_matches_rust_tokens_and_ast() {
    let files = corpus_files("success");
    assert!(
        !files.is_empty(),
        "success parity corpus must include at least one file"
    );

    for source in files {
        let rust_tokens = run_rust_dump_tokens(&source);
        let self_host_tokens = run_self_host_lexer(&source);
        assert_eq!(
            rust_tokens,
            self_host_tokens,
            "token parity mismatch for {}",
            source.display()
        );

        let rust_ast = run_rust_dump_ast(&source);
        let self_host_ast = run_self_host_parser(&self_host_tokens, "success");
        assert!(
            !self_host_ast.contains("parse error:"),
            "unexpected self-host parser diagnostic for {}:\n{}",
            source.display(),
            self_host_ast
        );
        assert_eq!(
            parse_json(&rust_ast),
            parse_json(&self_host_ast),
            "AST parity mismatch for {}",
            source.display()
        );
    }
}

#[test]
fn self_host_error_corpus_matches_rust_tokens_and_parse_failure_location() {
    let files = corpus_files("errors");
    assert!(
        !files.is_empty(),
        "error parity corpus must include at least one file"
    );

    for source in files {
        let rust_tokens = run_rust_dump_tokens(&source);
        let self_host_tokens = run_self_host_lexer(&source);
        assert_eq!(
            rust_tokens,
            self_host_tokens,
            "token parity mismatch for {}",
            source.display()
        );

        let rust = Command::new(bin())
            .arg(&source)
            .output()
            .expect("spawn rz error case");
        assert!(
            !rust.status.success(),
            "Rust frontend unexpectedly accepted {}",
            source.display()
        );
        let rust_stderr = String::from_utf8_lossy(&rust.stderr);
        let rust_error = first_rust_parse_error(&rust_stderr)
            .unwrap_or_else(|| panic!("missing Rust parser diagnostic for {}", source.display()));
        assert!(
            rust_error.contains("Expected '='"),
            "Rust parser should complain about missing '=' for {}:\n{}",
            source.display(),
            rust_stderr
        );

        let self_host_output = run_self_host_parser(&self_host_tokens, "error");
        let self_host_error = first_self_host_parse_error(&self_host_output).unwrap_or_else(|| {
            panic!(
                "missing self-host parser diagnostic for {}:\n{}",
                source.display(),
                self_host_output
            )
        });
        assert!(
            self_host_error.contains("expected operator '='"),
            "self-host parser should complain about missing '=' for {}:\n{}",
            source.display(),
            self_host_output
        );

        assert_eq!(
            extract_line_col(&rust_error),
            extract_line_col(&self_host_error),
            "parse failure location mismatch for {}",
            source.display()
        );
    }
}
