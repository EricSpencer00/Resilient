use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FeatureStatus {
    Covered,
    Missing,
    Divergent,
}

impl FeatureStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Covered => "covered",
            Self::Missing => "missing",
            Self::Divergent => "divergent",
        }
    }
}

struct FeatureSpec {
    id: &'static str,
    category: &'static str,
    description: &'static str,
    success_detector: Option<fn(&SuccessCase) -> bool>,
    error_detector: Option<fn(&ErrorCase) -> bool>,
}

struct FeatureOutcome {
    spec: &'static FeatureSpec,
    status: FeatureStatus,
    samples: Vec<String>,
    divergent_cases: Vec<String>,
}

struct SuccessCase {
    label: String,
    rust_ast: Value,
    token_parity: bool,
    ast_parity: bool,
    details: Vec<String>,
}

impl SuccessCase {
    fn is_green(&self) -> bool {
        self.token_parity && self.ast_parity
    }
}

struct ErrorCase {
    label: String,
    token_parity: bool,
    parse_error_location: bool,
    details: Vec<String>,
}

impl ErrorCase {
    fn is_green(&self) -> bool {
        self.token_parity && self.parse_error_location
    }
}

struct Report {
    corpus_root: PathBuf,
    success_cases: Vec<SuccessCase>,
    error_cases: Vec<ErrorCase>,
    features: Vec<FeatureOutcome>,
}

impl Report {
    fn has_divergence(&self) -> bool {
        self.success_cases.iter().any(|case| !case.is_green())
            || self.error_cases.iter().any(|case| !case.is_green())
    }

    fn token_parity_ok(&self) -> bool {
        self.success_cases.iter().all(|case| case.token_parity)
            && self.error_cases.iter().all(|case| case.token_parity)
    }

    fn ast_parity_ok(&self) -> bool {
        self.success_cases.iter().all(|case| case.ast_parity)
    }

    fn parse_error_location_ok(&self) -> bool {
        self.error_cases
            .iter()
            .all(|case| case.parse_error_location)
    }

    fn covered_features(&self) -> usize {
        self.features
            .iter()
            .filter(|feature| feature.status == FeatureStatus::Covered)
            .count()
    }

    fn missing_features(&self) -> usize {
        self.features
            .iter()
            .filter(|feature| feature.status == FeatureStatus::Missing)
            .count()
    }

    fn divergent_features(&self) -> usize {
        self.features
            .iter()
            .filter(|feature| feature.status == FeatureStatus::Divergent)
            .count()
    }
}

const FEATURE_SPECS: &[FeatureSpec] = &[
    FeatureSpec {
        id: "decl.function",
        category: "declarations",
        description: "function declarations",
        success_detector: Some(feature_function_decl),
        error_detector: None,
    },
    FeatureSpec {
        id: "decl.typed_param",
        category: "declarations",
        description: "typed function parameters",
        success_detector: Some(feature_typed_param),
        error_detector: None,
    },
    FeatureSpec {
        id: "decl.return_type",
        category: "declarations",
        description: "explicit function return types",
        success_detector: Some(feature_return_type),
        error_detector: None,
    },
    FeatureSpec {
        id: "stmt.block",
        category: "statements",
        description: "block statements",
        success_detector: Some(feature_block),
        error_detector: None,
    },
    FeatureSpec {
        id: "stmt.expr",
        category: "statements",
        description: "top-level expression statements",
        success_detector: Some(feature_expr_stmt),
        error_detector: None,
    },
    FeatureSpec {
        id: "stmt.let",
        category: "statements",
        description: "let statements",
        success_detector: Some(feature_let_stmt),
        error_detector: None,
    },
    FeatureSpec {
        id: "stmt.return",
        category: "statements",
        description: "return statements",
        success_detector: Some(feature_return_stmt),
        error_detector: None,
    },
    FeatureSpec {
        id: "stmt.if_else",
        category: "statements",
        description: "if / else statements",
        success_detector: Some(feature_if_stmt),
        error_detector: None,
    },
    FeatureSpec {
        id: "expr.call",
        category: "expressions",
        description: "call expressions",
        success_detector: Some(feature_call_expr),
        error_detector: None,
    },
    FeatureSpec {
        id: "expr.identifier",
        category: "expressions",
        description: "identifier expressions",
        success_detector: Some(feature_identifier_expr),
        error_detector: None,
    },
    FeatureSpec {
        id: "expr.int_literal",
        category: "expressions",
        description: "integer literals",
        success_detector: Some(feature_int_literal),
        error_detector: None,
    },
    FeatureSpec {
        id: "expr.string_literal",
        category: "expressions",
        description: "string literals",
        success_detector: Some(feature_string_literal),
        error_detector: None,
    },
    FeatureSpec {
        id: "expr.bool_literal",
        category: "expressions",
        description: "boolean literals",
        success_detector: Some(feature_bool_literal),
        error_detector: None,
    },
    FeatureSpec {
        id: "expr.float_literal",
        category: "expressions",
        description: "floating-point literals",
        success_detector: Some(feature_float_literal),
        error_detector: None,
    },
    FeatureSpec {
        id: "expr.prefix",
        category: "expressions",
        description: "prefix expressions",
        success_detector: Some(feature_prefix_expr),
        error_detector: None,
    },
    FeatureSpec {
        id: "expr.binary_add",
        category: "expressions",
        description: "binary `+` expressions",
        success_detector: Some(feature_binary_add),
        error_detector: None,
    },
    FeatureSpec {
        id: "expr.binary_gt",
        category: "expressions",
        description: "binary `>` expressions",
        success_detector: Some(feature_binary_gt),
        error_detector: None,
    },
    FeatureSpec {
        id: "expr.assignment",
        category: "expressions",
        description: "assignment expressions",
        success_detector: Some(feature_assignment),
        error_detector: None,
    },
    FeatureSpec {
        id: "expr.array_literal",
        category: "expressions",
        description: "array literals",
        success_detector: Some(feature_array_literal),
        error_detector: None,
    },
    FeatureSpec {
        id: "error.parse_location",
        category: "errors",
        description: "parse-failure location parity",
        success_detector: None,
        error_detector: Some(feature_parse_error_location),
    },
];

pub(crate) fn dispatch_self_host_parity_report_subcommand(args: &[String]) -> Option<i32> {
    if args.get(1).map(|arg| arg.as_str()) != Some("self-host-parity-report") {
        return None;
    }
    if is_self_host_parity_report_help_request(args) {
        print_self_host_parity_report_help();
        return Some(0);
    }

    let mut corpus_root: Option<PathBuf> = None;
    let mut json_out: Option<PathBuf> = None;
    let mut i = 2;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--json-out" {
            i += 1;
            if i >= args.len() {
                eprintln!("Error: self-host-parity-report --json-out requires a path");
                return Some(2);
            }
            json_out = Some(PathBuf::from(&args[i]));
        } else if let Some(path) = arg.strip_prefix("--json-out=") {
            json_out = Some(PathBuf::from(path));
        } else if !arg.starts_with('-') && corpus_root.is_none() {
            corpus_root = Some(PathBuf::from(arg));
        } else {
            eprintln!(
                "Error: unexpected argument `{}` to self-host-parity-report",
                arg
            );
            return Some(2);
        }
        i += 1;
    }

    let corpus_root = corpus_root.unwrap_or_else(|| repo_root().join("self-host/parity_corpus"));
    match build_report(&corpus_root) {
        Ok(report) => {
            if let Some(path) = json_out.as_deref()
                && let Err(err) = write_report_json(path, &report)
            {
                eprintln!("Error writing {}: {}", path.display(), err);
                return Some(1);
            }
            print_report(&report, json_out.as_deref());
            Some(if report.has_divergence() { 1 } else { 0 })
        }
        Err(err) => {
            eprintln!("Error: {}", err);
            Some(1)
        }
    }
}

const SELF_HOST_PARITY_REPORT_HELP_TEXT: &str = r#"rz self-host-parity-report — publish self-hosting parity coverage

USAGE:
    rz self-host-parity-report [DIR] [--json-out PATH]

INPUT:
    DIR defaults to self-host/parity_corpus.
    The report compares Rust and self-host lexer/parser output for that corpus.

OUTPUT:
    Prints a coverage and divergence summary to stdout.
    With --json-out, also writes a stable JSON report artifact.

FLAGS:
        --json-out PATH    Write the machine-readable report to PATH

EXAMPLES:
    rz self-host-parity-report
    rz self-host-parity-report self-host/parity_corpus --json-out parity.json

Run `rz --help` for global flags and other subcommands.
"#;

fn is_self_host_parity_report_help_request(args: &[String]) -> bool {
    matches!(
        args.get(2).map(String::as_str),
        Some("--help" | "-h" | "help")
    )
}

fn print_self_host_parity_report_help() {
    print!("{}", SELF_HOST_PARITY_REPORT_HELP_TEXT);
}

fn build_report(corpus_root: &Path) -> Result<Report, String> {
    let exe = env::current_exe().map_err(|err| format!("resolve current executable: {err}"))?;
    let success_files = corpus_files(corpus_root, "success")?;
    let error_files = corpus_files(corpus_root, "errors")?;
    let mut success_cases = Vec::with_capacity(success_files.len());
    let mut error_cases = Vec::with_capacity(error_files.len());

    for source in success_files {
        success_cases.push(run_success_case(&exe, corpus_root, &source)?);
    }
    for source in error_files {
        error_cases.push(run_error_case(&exe, corpus_root, &source)?);
    }

    let mut features = Vec::with_capacity(FEATURE_SPECS.len());
    for spec in FEATURE_SPECS {
        let mut samples = Vec::new();
        let mut divergent_cases = Vec::new();

        if let Some(detector) = spec.success_detector {
            for case in &success_cases {
                if detector(case) {
                    samples.push(case.label.clone());
                    if !case.is_green() {
                        divergent_cases.push(case.label.clone());
                    }
                }
            }
        }
        if let Some(detector) = spec.error_detector {
            for case in &error_cases {
                if detector(case) {
                    samples.push(case.label.clone());
                    if !case.is_green() {
                        divergent_cases.push(case.label.clone());
                    }
                }
            }
        }

        let status = if samples.is_empty() {
            FeatureStatus::Missing
        } else if divergent_cases.is_empty() {
            FeatureStatus::Covered
        } else {
            FeatureStatus::Divergent
        };

        features.push(FeatureOutcome {
            spec,
            status,
            samples,
            divergent_cases,
        });
    }

    Ok(Report {
        corpus_root: corpus_root.to_path_buf(),
        success_cases,
        error_cases,
        features,
    })
}

fn run_success_case(exe: &Path, corpus_root: &Path, source: &Path) -> Result<SuccessCase, String> {
    let label = relative_case_label(corpus_root, source)?;
    let src =
        fs::read_to_string(source).map_err(|err| format!("read {}: {err}", source.display()))?;
    let rust_tokens = normalize_rust_tokens(&crate::dump_tokens_string(&src));
    let self_host_tokens = run_self_host_lexer(exe, source)?;
    let rust_ast = crate::dump_ast_json_value(&src)
        .map_err(|errs| format!("Rust AST dump failed for {}: {}", label, errs.join(" | ")))?;
    let self_host_ast_text = run_self_host_parser(exe, &self_host_tokens, &label)?;
    let mut details = Vec::new();

    let token_parity = rust_tokens == self_host_tokens;
    if !token_parity {
        details.push("token parity mismatch".to_string());
    }

    let self_host_ast: Value = if self_host_ast_text.contains("parse error:") {
        details.push("self-host parser returned a parse error on a success case".to_string());
        Value::Null
    } else {
        serde_json::from_str(&self_host_ast_text).map_err(|err| {
            format!(
                "self-host AST JSON invalid for {}: {} (stdout: {})",
                label, err, self_host_ast_text
            )
        })?
    };

    let ast_parity = self_host_ast != Value::Null && rust_ast == self_host_ast;
    if !ast_parity {
        details.push("AST parity mismatch".to_string());
    }

    Ok(SuccessCase {
        label,
        rust_ast,
        token_parity,
        ast_parity,
        details,
    })
}

fn run_error_case(exe: &Path, corpus_root: &Path, source: &Path) -> Result<ErrorCase, String> {
    let label = relative_case_label(corpus_root, source)?;
    let src =
        fs::read_to_string(source).map_err(|err| format!("read {}: {err}", source.display()))?;
    let rust_tokens = normalize_rust_tokens(&crate::dump_tokens_string(&src));
    let self_host_tokens = run_self_host_lexer(exe, source)?;
    let token_parity = rust_tokens == self_host_tokens;
    let mut details = Vec::new();
    if !token_parity {
        details.push("token parity mismatch".to_string());
    }

    let (_, rust_errors) = crate::parse(&src);
    let rust_error = rust_errors
        .iter()
        .find(|err| crate::parse_error_location(err).0 > 0)
        .cloned()
        .ok_or_else(|| format!("missing Rust parse diagnostic for {}", label))?;
    let self_host_output = run_self_host_parser(exe, &self_host_tokens, &label)?;
    let self_host_error = first_self_host_parse_error(&self_host_output)
        .ok_or_else(|| format!("missing self-host parse diagnostic for {}", label))?;
    let rust_coords = crate::parse_error_location(&rust_error);
    let self_host_coords = extract_line_col(&self_host_error)
        .ok_or_else(|| format!("missing self-host error location for {}", label))?;
    let parse_error_location = (rust_coords.0 as usize, rust_coords.1 as usize) == self_host_coords;
    if !parse_error_location {
        details.push(format!(
            "parse-error location mismatch (rust {}:{}, self-host {}:{})",
            rust_coords.0, rust_coords.1, self_host_coords.0, self_host_coords.1
        ));
    }

    Ok(ErrorCase {
        label,
        token_parity,
        parse_error_location,
        details,
    })
}

fn print_report(report: &Report, json_out: Option<&Path>) {
    println!("self-host parity report");
    println!("corpus.root={}", report.corpus_root.display());
    println!("cases.success={}", report.success_cases.len());
    println!("cases.errors={}", report.error_cases.len());
    println!(
        "artifacts.token_parity={}",
        bool_status(report.token_parity_ok())
    );
    println!(
        "artifacts.ast_parity={}",
        bool_status(report.ast_parity_ok())
    );
    println!(
        "artifacts.parse_error_location={}",
        bool_status(report.parse_error_location_ok())
    );
    println!("features.covered={}", report.covered_features());
    println!("features.missing={}", report.missing_features());
    println!("features.divergent={}", report.divergent_features());
    if let Some(path) = json_out {
        println!("artifact.report_json={}", path.display());
    }

    println!();
    println!("status     feature                       samples");
    println!("---------------------------------------------------------------");
    for feature in &report.features {
        let samples = if feature.samples.is_empty() {
            "-".to_string()
        } else {
            feature.samples.join(", ")
        };
        println!(
            "{:<10} {:<29} {}",
            feature.status.as_str(),
            feature.spec.id,
            samples
        );
    }

    if report.has_divergence() {
        println!();
        println!("divergences:");
        for case in &report.success_cases {
            if !case.is_green() {
                println!("{}: {}", case.label, case.details.join("; "));
            }
        }
        for case in &report.error_cases {
            if !case.is_green() {
                println!("{}: {}", case.label, case.details.join("; "));
            }
        }
    }
}

fn write_report_json(path: &Path, report: &Report) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|err| format!("create {}: {err}", parent.display()))?;
    }
    let rendered = serde_json::to_string_pretty(&report_to_json(report))
        .map_err(|err| format!("serialize JSON report: {err}"))?;
    fs::write(path, rendered).map_err(|err| format!("write {}: {err}", path.display()))
}

fn report_to_json(report: &Report) -> Value {
    json!({
        "schema_version": 1,
        "corpus_root": report.corpus_root.display().to_string(),
        "success_case_count": report.success_cases.len(),
        "error_case_count": report.error_cases.len(),
        "artifact_summary": {
            "token_parity": bool_status(report.token_parity_ok()),
            "ast_parity": bool_status(report.ast_parity_ok()),
            "parse_error_location": bool_status(report.parse_error_location_ok()),
        },
        "feature_summary": {
            "covered": report.covered_features(),
            "missing": report.missing_features(),
            "divergent": report.divergent_features(),
        },
        "features": report.features.iter().map(|feature| {
            json!({
                "id": feature.spec.id,
                "category": feature.spec.category,
                "description": feature.spec.description,
                "status": feature.status.as_str(),
                "samples": feature.samples,
                "divergent_cases": feature.divergent_cases,
            })
        }).collect::<Vec<_>>(),
        "cases": report.success_cases.iter().map(|case| {
            json!({
                "file": case.label,
                "kind": "success",
                "token_parity": bool_status(case.token_parity),
                "ast_parity": bool_status(case.ast_parity),
                "details": case.details,
            })
        }).chain(report.error_cases.iter().map(|case| {
            json!({
                "file": case.label,
                "kind": "error",
                "token_parity": bool_status(case.token_parity),
                "parse_error_location": bool_status(case.parse_error_location),
                "details": case.details,
            })
        })).collect::<Vec<_>>(),
    })
}

fn relative_case_label(corpus_root: &Path, source: &Path) -> Result<String, String> {
    let rel = source
        .strip_prefix(corpus_root)
        .map_err(|_| format!("{} is outside {}", source.display(), corpus_root.display()))?;
    Ok(rel.display().to_string())
}

fn corpus_files(corpus_root: &Path, kind: &str) -> Result<Vec<PathBuf>, String> {
    let dir = corpus_root.join(kind);
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|err| format!("read {}: {err}", dir.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("read {} entry: {err}", dir.display()))?
        .into_iter()
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rz"))
        .collect();
    files.sort();
    Ok(files)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn temp_tokens_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "res2992_{}_{}_{}.tokens.txt",
        std::process::id(),
        label.replace('/', "_"),
        nanos
    ))
}

fn run_self_host_lexer(exe: &Path, source: &Path) -> Result<String, String> {
    let lexer = repo_root().join("self-host/lexer.rz");
    let output = Command::new(exe)
        .arg(&lexer)
        .env("SELF_HOST_INPUT", source)
        .output()
        .map_err(|err| format!("spawn self-host lexer: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "self-host lexer failed for {}:\nstdout={}\nstderr={}",
            source.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(normalize_self_host_stream(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn run_self_host_parser(exe: &Path, tokens: &str, label: &str) -> Result<String, String> {
    let tokens_path = temp_tokens_path(label);
    fs::write(&tokens_path, tokens)
        .map_err(|err| format!("write {}: {err}", tokens_path.display()))?;
    let parser = repo_root().join("self-host/parser.rz");
    let output = Command::new(exe)
        .arg(&parser)
        .env("SELF_HOST_TOKENS", &tokens_path)
        .output()
        .map_err(|err| format!("spawn self-host parser: {err}"))?;
    let _ = fs::remove_file(&tokens_path);
    if !output.status.success() {
        return Err(format!(
            "self-host parser failed for {}:\nstdout={}\nstderr={}",
            label,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(normalize_self_host_stream(&String::from_utf8_lossy(
        &output.stdout,
    )))
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

fn first_self_host_parse_error(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .find(|line| line.starts_with("parse error:"))
        .map(|line| line.trim().to_string())
}

fn extract_line_col(msg: &str) -> Option<(usize, usize)> {
    let at = msg.rfind(" at ")?;
    let coords = &msg[at + 4..];
    let mut parts = coords.split(':');
    let line_no = parts.next()?.trim().parse().ok()?;
    let col_no = parts.next()?.trim().parse().ok()?;
    Some((line_no, col_no))
}

fn bool_status(ok: bool) -> &'static str {
    if ok { "pass" } else { "fail" }
}

fn json_any_object<F>(value: &Value, predicate: &F) -> bool
where
    F: Fn(&serde_json::Map<String, Value>) -> bool,
{
    match value {
        Value::Object(map) => {
            predicate(map) || map.values().any(|child| json_any_object(child, predicate))
        }
        Value::Array(items) => items.iter().any(|child| json_any_object(child, predicate)),
        _ => false,
    }
}

fn json_contains_type(value: &Value, wanted: &str) -> bool {
    json_any_object(value, &|map| {
        map.get("type").and_then(Value::as_str) == Some(wanted)
    })
}

fn json_contains_binary_op(value: &Value, wanted: &str) -> bool {
    json_any_object(value, &|map| {
        map.get("type").and_then(Value::as_str) == Some("Binary")
            && map.get("op").and_then(Value::as_str) == Some(wanted)
    })
}

fn json_contains_function_with_params(value: &Value) -> bool {
    json_any_object(value, &|map| {
        map.get("type").and_then(Value::as_str) == Some("Function")
            && map
                .get("params")
                .and_then(Value::as_array)
                .is_some_and(|params| !params.is_empty())
    })
}

fn json_contains_function_with_non_void_return(value: &Value) -> bool {
    json_any_object(value, &|map| {
        map.get("type").and_then(Value::as_str) == Some("Function")
            && map
                .get("returns")
                .and_then(Value::as_str)
                .is_some_and(|ret| ret != "void")
    })
}

fn feature_function_decl(case: &SuccessCase) -> bool {
    json_contains_type(&case.rust_ast, "Function")
}

fn feature_typed_param(case: &SuccessCase) -> bool {
    json_contains_function_with_params(&case.rust_ast)
}

fn feature_return_type(case: &SuccessCase) -> bool {
    json_contains_function_with_non_void_return(&case.rust_ast)
}

fn feature_block(case: &SuccessCase) -> bool {
    json_contains_type(&case.rust_ast, "Block")
}

fn feature_expr_stmt(case: &SuccessCase) -> bool {
    json_contains_type(&case.rust_ast, "ExprStmt")
}

fn feature_let_stmt(case: &SuccessCase) -> bool {
    json_contains_type(&case.rust_ast, "Let")
}

fn feature_return_stmt(case: &SuccessCase) -> bool {
    json_contains_type(&case.rust_ast, "Return")
}

fn feature_if_stmt(case: &SuccessCase) -> bool {
    json_contains_type(&case.rust_ast, "If")
}

fn feature_call_expr(case: &SuccessCase) -> bool {
    json_contains_type(&case.rust_ast, "Call")
}

fn feature_identifier_expr(case: &SuccessCase) -> bool {
    json_contains_type(&case.rust_ast, "Identifier")
}

fn feature_int_literal(case: &SuccessCase) -> bool {
    json_contains_type(&case.rust_ast, "Int")
}

fn feature_string_literal(case: &SuccessCase) -> bool {
    json_contains_type(&case.rust_ast, "String")
}

fn feature_bool_literal(case: &SuccessCase) -> bool {
    json_contains_type(&case.rust_ast, "Bool")
}

fn feature_float_literal(case: &SuccessCase) -> bool {
    json_contains_type(&case.rust_ast, "Float")
}

fn feature_prefix_expr(case: &SuccessCase) -> bool {
    json_contains_type(&case.rust_ast, "Prefix")
}

fn feature_binary_add(case: &SuccessCase) -> bool {
    json_contains_binary_op(&case.rust_ast, "+")
}

fn feature_binary_gt(case: &SuccessCase) -> bool {
    json_contains_binary_op(&case.rust_ast, ">")
}

fn feature_assignment(case: &SuccessCase) -> bool {
    json_contains_binary_op(&case.rust_ast, "=")
}

fn feature_array_literal(case: &SuccessCase) -> bool {
    json_contains_type(&case.rust_ast, "Array")
}

fn feature_parse_error_location(_case: &ErrorCase) -> bool {
    true
}
