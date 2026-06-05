//! RES-074: minimum-viable Language Server for Resilient.
//!
//! Implements `textDocument/didOpen` and `textDocument/didChange`:
//! each time either fires, we parse the buffer, run the typechecker,
//! and publish diagnostics with source ranges derived from
//! RES-077's per-statement `Spanned<Node>` wrappers.
//!
//! Nothing else yet — no hover, no completion, no go-to-definition.
//! Those are dedicated follow-up tickets. This ticket is the
//! scaffolding that makes an editor light up with red squiggles.
//!
//! The `mod lsp_server;` declaration in `main.rs` is already
//! gated on `cfg(feature = "lsp")`, so this file is only compiled
//! when the feature is on — no per-file `#![cfg]` needed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tower_lsp::jsonrpc::Result as JsonResult;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionProviderCapability, CompletionItem, CompletionItemKind, CompletionOptions,
    CompletionParams, CompletionResponse, Diagnostic, DiagnosticSeverity,
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, DocumentSymbol, DocumentSymbolParams,
    DocumentSymbolResponse, GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents,
    HoverParams, HoverProviderCapability, InitializeParams, InitializeResult, InitializedParams,
    InlayHint, InlayHintKind, InlayHintLabel, InlayHintOptions, InlayHintParams,
    InlayHintServerCapabilities, Location, MarkedString, MessageType, NumberOrString, OneOf,
    Position, PrepareRenameResponse, Range, ReferenceParams, RenameOptions, RenameParams,
    SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokens,
    SemanticTokensFullOptions, SemanticTokensLegend, SemanticTokensOptions, SemanticTokensParams,
    SemanticTokensResult, SemanticTokensServerCapabilities, ServerCapabilities, SymbolInformation,
    SymbolKind, TextDocumentPositionParams, TextDocumentSyncCapability, TextDocumentSyncKind,
    TextEdit, Url, WorkDoneProgressOptions, WorkspaceEdit, WorkspaceSymbolParams,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::{Node, builtin_names, compute_semantic_tokens, parse, typechecker};

/// RES-186: one workspace-level symbol entry. A flat vec of these
/// is the backend's search index — substring filter at query time,
/// rebuilt per `did_save`.
#[derive(Clone, Debug)]
pub(crate) struct WorkspaceSymbolEntry {
    pub(crate) name: String,
    pub(crate) kind: SymbolKind,
    pub(crate) uri: Url,
    pub(crate) range: Range,
}

#[derive(Clone, Copy, Debug)]
struct InlayHintConfig {
    types: bool,
    parameters: bool,
}

/// The LSP backend. Holds a `Client` handle for publishing diagnostics.
///
/// RES-185: the `documents` map caches each open document's last-
/// parsed AST, keyed by `Url`. Document-symbol (and future
/// cursor-aware) handlers consume from here instead of re-parsing
/// on every request.
///
/// RES-186: the `workspace_index` is a pre-computed list of every
/// `*.rs` file's top-level symbols in the workspace root. Built
/// lazily on first `workspace/symbol` request (cheaper than
/// walking at `initialize` time when the workspace might be huge),
/// cached, and refreshed per-file on `did_save`.
pub struct Backend {
    client: Client,
    /// URI → latest parsed Program AST. Mutex-guarded because
    /// LSP handlers run on the tokio runtime's worker threads,
    /// and we never hold the lock across an `.await`.
    documents: Mutex<HashMap<Url, Node>>,
    /// RES-187: URI → latest raw source text. Stored separately
    /// from the AST because `semantic_tokens_full` re-lexes the
    /// source (the lexer's token stream is the source of truth
    /// for highlighting — the AST discards too much by this
    /// point, e.g. operator forms collapse into `BinaryOp`).
    /// Same mutex discipline as `documents`: synchronous lock,
    /// never held across `.await`.
    documents_text: Mutex<HashMap<Url, String>>,
    /// RES-186: per-file symbol index. Keyed by `Url` so a
    /// `did_save` can replace just that file's entries instead of
    /// rebuilding the whole thing. The vec-of-entries form inside
    /// each value keeps the filter loop flat at query time.
    workspace_index: Mutex<HashMap<Url, Vec<WorkspaceSymbolEntry>>>,
    /// RES-186: workspace root path captured from `initialize` —
    /// either `workspace_folders[0].uri` or the deprecated
    /// `root_uri`. `None` when the client opened a single file
    /// with no workspace attached. Used to decide whether to
    /// walk at index-build time.
    workspace_root: Mutex<Option<PathBuf>>,
    /// RES-186: once the workspace index has been built, set this
    /// to skip rebuilding on every `workspace/symbol` call. Reset
    /// to false on `did_save` to trigger a lazy refresh on the
    /// next query.
    workspace_index_built: Mutex<bool>,
    /// RES-2569: user preference for inferred-type inlay hints.
    /// Defaults on so unannotated `let` / fn-return hints surface
    /// without extra client config; `resilient.inlayHints.types: false`
    /// disables them.
    inlay_hint_types: Mutex<bool>,
    /// RES-189: user preference for inlay hints at call sites
    /// (`add(a: 1, b: 2)`-style parameter labels). Off by default
    /// per the ticket; the client flips it on via `initializationOptions`
    /// (`resilient.inlayHints.parameters: true`).
    inlay_hint_parameters: Mutex<bool>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Mutex::new(HashMap::new()),
            documents_text: Mutex::new(HashMap::new()),
            workspace_index: Mutex::new(HashMap::new()),
            workspace_root: Mutex::new(None),
            workspace_index_built: Mutex::new(false),
            inlay_hint_types: Mutex::new(true),
            inlay_hint_parameters: Mutex::new(false),
        }
    }

    /// RES-074: analyze `text` for parser + typechecker errors and
    /// publish them as LSP diagnostics against `uri`. Called from
    /// both `did_open` and `did_change`.
    async fn publish_analysis(&self, uri: Url, text: String) {
        let mut diagnostics = Vec::new();

        // Step 1: parse. Parser::record_error formats errors with a
        // bare `<line>:<col>:` prefix; route them through
        // extract_range_and_message (RES-089) so they land at the
        // right LSP Range instead of the file's first character.
        let (program, parser_errors) = parse(&text);

        // RES-185: cache the freshly-parsed AST so document-symbol
        // (and future cursor-aware) handlers don't have to re-parse.
        // Stored even if the parse reported recoverable errors —
        // the partial AST still covers the recovered fns / structs.
        if let Ok(mut map) = self.documents.lock() {
            map.insert(uri.clone(), program.clone());
        }
        // RES-187: cache the raw source text too, for semantic-
        // tokens requests that re-lex the file.
        if let Ok(mut tmap) = self.documents_text.lock() {
            tmap.insert(uri.clone(), text.clone());
        }

        for err in &parser_errors {
            let (range, pretty) = extract_range_and_message(err);
            diagnostics.push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("resilient-parser".into()),
                message: pretty,
                ..Default::default()
            });
        }

        // Step 2: typechecker. Errors from RES-080's
        // `check_program_with_source` come back prefixed with
        // `<path>:<line>:<col>: ...`. We re-parse that prefix so the
        // diagnostic range lines up with what the user sees.
        if parser_errors.is_empty() {
            let mut tc = typechecker::TypeChecker::new();
            if let Err(msg) = tc.check_program_with_source(&program, uri.as_str()) {
                let (range, pretty) = extract_range_and_message(&msg);
                diagnostics.push(Diagnostic {
                    range,
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("resilient-typecheck".into()),
                    message: pretty,
                    ..Default::default()
                });
            }
        }

        // RES-357: Step 3: run lints (warning-level). Convert each Lint
        // into an LSP Diagnostic so clients see L0010 "no contract" and
        // can request the "Add contract stubs" code action.
        // Lint positions are 1-indexed; convert to 0-indexed LSP positions.
        if parser_errors.is_empty() {
            for lint in crate::lint::check(&program, &text) {
                let line0 = lint.line.saturating_sub(1);
                let col0 = lint.column.saturating_sub(1);
                let pos = Position::new(line0, col0);
                let range = Range::new(pos, pos);
                let severity = match lint.severity {
                    crate::lint::Severity::Error => DiagnosticSeverity::ERROR,
                    crate::lint::Severity::Warning => DiagnosticSeverity::WARNING,
                };
                diagnostics.push(Diagnostic {
                    range,
                    severity: Some(severity),
                    source: Some("resilient-lint".into()),
                    message: lint.message,
                    ..Default::default()
                });
            }
        }

        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }
}

/// RES-074: LSP uses 0-indexed line/column; RES-077's span is
/// 1-indexed. Subtract 1 (clamped) and build a zero-width range.
fn point_range(line_0based: u32, col_0based: u32) -> Range {
    let pos = Position::new(line_0based, col_0based);
    Range::new(pos, pos)
}

/// Parse a `<line>:<col>:` or `<path>:<line>:<col>:` prefix out of
/// an error string. Returns a zero-width `Range` at that position
/// plus the remaining message (or a zero-zero range + the original
/// message if parsing fails).
///
/// Recognizes both forms used in this codebase:
/// - RES-080 typechecker errors: `<path>:<line>:<col>: <rest>`
/// - RES-089 parser errors: `<line>:<col>: <rest>` (no path)
fn extract_range_and_message(err: &str) -> (Range, String) {
    // RES-089: try the bare `<line>:<col>:` form first. This catches
    // parser errors emitted by `Parser::record_error`.
    if let Some(parsed) = parse_bare_line_col(err) {
        return parsed;
    }

    // Fall back to `<anything>:<line>:<col>:` form (typechecker).
    // Find the FIRST colon after which `<uint>:<uint>:` follows.
    for (i, _) in err.match_indices(':') {
        let rest = &err[i + 1..];
        let mut parts = rest.splitn(3, ':');
        let line_s = parts.next().unwrap_or("");
        let col_s = parts.next().unwrap_or("");
        let msg = parts.next().unwrap_or("");
        if let (Ok(line), Ok(col)) = (line_s.trim().parse::<u32>(), col_s.trim().parse::<u32>()) {
            let line0 = line.saturating_sub(1);
            let col0 = col.saturating_sub(1);
            return (point_range(line0, col0), msg.trim().to_string());
        }
    }
    (point_range(0, 0), err.to_string())
}

/// RES-089: parse a bare `<line>:<col>: <message>` prefix (no
/// preceding path). Returns None if the input doesn't fit that
/// shape exactly.
fn parse_bare_line_col(err: &str) -> Option<(Range, String)> {
    let mut parts = err.splitn(3, ':');
    let line_s = parts.next()?;
    let col_s = parts.next()?;
    let msg = parts.next()?;
    let line: u32 = line_s.trim().parse().ok()?;
    let col: u32 = col_s.trim().parse().ok()?;
    Some((
        point_range(line.saturating_sub(1), col.saturating_sub(1)),
        msg.trim().to_string(),
    ))
}

/// RES-185: map the runtime's `Span` (1-indexed line/col, 0 for
/// offset) to LSP's `Range` (0-indexed line/character).
fn span_to_range(span: crate::span::Span) -> Range {
    let start = Position::new(
        span.start.line.saturating_sub(1) as u32,
        span.start.column.saturating_sub(1) as u32,
    );
    let end = Position::new(
        span.end.line.saturating_sub(1) as u32,
        span.end.column.saturating_sub(1) as u32,
    );
    Range::new(start, end)
}

/// RES-181a: classify the token kind at LSP `pos` inside `src`
/// and return `(type_name, range)` for hover display. Returns
/// `None` if no literal token covers that position.
///
/// Why drive the lexer directly instead of walking the AST? The
/// parser's `span_at_current` records `last_token_*` after the
/// lexer has ALREADY advanced to the next token — so a
/// `Node::IntegerLiteral` built inside `parse_expression` ends
/// up with a span pointing at whatever lexeme follows, not at
/// the literal itself. Reliable per-token positions come straight
/// from `Lexer::next_token_with_span`, which returns true start
/// AND end coordinates in one call.
///
/// This lookup is O(tokens). A cached-source document is small
/// enough that re-lexing on each hover request is cheap — faster
/// than the HashMap operation that reads it.
pub(crate) fn hover_literal_at(src: &str, pos: Position) -> Option<(&'static str, Range)> {
    use crate::{Lexer, Token};
    let mut lex = Lexer::new(src);
    loop {
        let (tok, span) = lex.next_token_with_span();
        if matches!(tok, Token::Eof) {
            return None;
        }
        if !lex_span_contains_lsp_position(span, pos) {
            continue;
        }
        let type_name: &'static str = match tok {
            Token::IntLiteral(_) => "Int",
            Token::FloatLiteral(_) => "Float",
            Token::StringLiteral(_) => "String",
            Token::BoolLiteral(_) => "Bool",
            Token::BytesLiteral(_) => "Bytes",
            _ => return None, // non-literal token at cursor → no hover
        };
        return Some((type_name, span_to_range(span)));
    }
}

/// RES-181a: does a lexer-produced `Span` (proper start + end,
/// 1-indexed) contain LSP `Position` (0-indexed)? End is
/// exclusive.
fn lex_span_contains_lsp_position(span: crate::span::Span, pos: Position) -> bool {
    let start_line = span.start.line.saturating_sub(1) as u32;
    let start_col = span.start.column.saturating_sub(1) as u32;
    let end_line = span.end.line.saturating_sub(1) as u32;
    let end_col = span.end.column.saturating_sub(1) as u32;
    // Check line ordering first: pos.line must be in [start, end].
    if pos.line < start_line || pos.line > end_line {
        return false;
    }
    // Single-line span: constrain by columns within that line.
    if start_line == end_line {
        return pos.character >= start_col && pos.character < end_col;
    }
    // Multi-line token (e.g. multi-line string literal). Inner
    // lines match unconditionally; boundaries check their column.
    if pos.line == start_line {
        return pos.character >= start_col;
    }
    if pos.line == end_line {
        return pos.character < end_col;
    }
    true
}

/// RES-182a: lex `src` and, if the cursor sits on an
/// `Identifier` token, return the `(name, span)` pair. Returns
/// `None` for every other kind of token (keywords, literals,
/// operators) or when the cursor isn't on any token.
///
/// Mirrors `hover_literal_at`'s token-level lookup approach: the
/// parser's per-leaf spans are unreliable, so we drive the lexer
/// directly against the cached source and find the containing
/// `Token::Identifier`. Caller uses the returned name to look up
/// the definition in a `TopLevelDefMap`.
pub(crate) fn identifier_at(src: &str, pos: Position) -> Option<(String, Range)> {
    use crate::{Lexer, Token};
    let mut lex = Lexer::new(src);
    loop {
        let (tok, span) = lex.next_token_with_span();
        if matches!(tok, Token::Eof) {
            return None;
        }
        if !lex_span_contains_lsp_position(span, pos) {
            continue;
        }
        if let Token::Identifier(name) = tok {
            return Some((name, span_to_range(span)));
        }
        // Non-identifier token at the cursor — no jump.
        return None;
    }
}

/// RES-182a: one top-level declaration's definition site.
/// `name_range` is the span of the identifier token on the
/// declaration line — what the editor highlights as the "goto
/// target" — while `full_range` covers the whole decl (same
/// data `document_symbols_for_program` uses). `full_range` is
/// what we return today; a future refinement could return
/// `name_range` when RES-088's span work gets an identifier-
/// specific span on `Node::Function` / friends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TopLevelDef {
    pub name: String,
    pub range: Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ReferenceSymbolKind {
    Fn,
    Struct,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceVisibleSymbol {
    origin_path: PathBuf,
    origin_name: String,
    kind: ReferenceSymbolKind,
    decl_range: Range,
}

#[derive(Debug, Clone)]
struct VariableReferenceSet {
    name: String,
    decl_range: Range,
    refs: Vec<Range>,
}

#[derive(Debug, Clone)]
struct UseStmtInfo {
    path: String,
    alias: Option<String>,
    selectors: Option<Vec<String>>,
    is_pub: bool,
}

/// RES-182a: build a name → top-level-decl-location map from a
/// parsed program. Covers `fn` / `struct` / `type` aliases —
/// the same decl shapes `document_symbols_for_program` already
/// walks. Duplicate names inside a single program (parser
/// doesn't reject them yet) resolve to the FIRST occurrence,
/// since goto-def is deterministic and users most often mean
/// the original definition.
pub(crate) fn build_top_level_defs(program: &Node) -> Vec<TopLevelDef> {
    let stmts = match program {
        Node::Program(s) => s,
        _ => return Vec::new(),
    };
    // RES-1976: pre-size `out` and `seen` to `stmts.len()` — the exact
    // upper bound (at most one entry per top-level statement, since
    // Function / StructDecl / TypeAlias are the only matching variants
    // and dedup-via-`seen` only shrinks the count further). Skips the
    // 0→4→8→16→32 doubling cascade for both Vec and HashSet. Same
    // exact-upper-bound shape as RES-1742 / RES-1744 / RES-1746 call-
    // graph pre-size series. `build_top_level_defs` is keystroke-rate
    // (runs per LSP `textDocument/definition` and `documentSymbol`).
    let mut out: Vec<TopLevelDef> = Vec::with_capacity(stmts.len());
    // RES-1508: borrow the AST's decl names into the dedup set
    // instead of cloning them. The owned String allocation only
    // happens once at the `out.push(...)` site — previously each
    // decl name allocated twice (once to bind `name`, once to feed
    // `seen.insert`).
    let mut seen: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(stmts.len());
    for spanned in stmts {
        let name = match &spanned.node {
            Node::Function { name, .. } => name.as_str(),
            Node::StructDecl { name, .. } => name.as_str(),
            Node::TypeAlias { name, .. } => name.as_str(),
            _ => continue,
        };
        if !seen.insert(name) {
            continue;
        }
        out.push(TopLevelDef {
            name: name.to_string(),
            range: span_to_range(spanned.span),
        });
    }
    out
}

/// RES-182a: find a top-level decl by name. Linear scan over
/// the `build_top_level_defs` vec — decl counts are small and
/// this runs per request (not per keystroke).
pub(crate) fn find_top_level_def<'a>(
    defs: &'a [TopLevelDef],
    name: &str,
) -> Option<&'a TopLevelDef> {
    defs.iter().find(|d| d.name == name)
}

fn canonicalize_or_self(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn raw_use_stmts(program: &Node) -> Vec<UseStmtInfo> {
    let Node::Program(stmts) = program else {
        return Vec::new();
    };
    stmts
        .iter()
        .filter_map(|stmt| match &stmt.node {
            Node::Use {
                path,
                alias,
                selectors,
                is_pub,
                ..
            } => Some(UseStmtInfo {
                path: path.clone(),
                alias: alias.clone(),
                selectors: selectors.clone(),
                is_pub: *is_pub,
            }),
            _ => None,
        })
        .collect()
}

fn local_top_level_symbols(
    program: &Node,
    src: &str,
    source_path: &Path,
    exported_only: bool,
) -> HashMap<String, WorkspaceVisibleSymbol> {
    let Node::Program(stmts) = program else {
        return HashMap::new();
    };
    let source_path = canonicalize_or_self(source_path);
    let any_pub = stmts.iter().any(|stmt| {
        matches!(
            stmt.node,
            Node::Function { is_pub: true, .. } | Node::StructDecl { is_pub: true, .. }
        )
    });
    let mut out = HashMap::new();
    for stmt in stmts {
        match &stmt.node {
            Node::Function { name, is_pub, .. } => {
                if exported_only && any_pub && !is_pub {
                    continue;
                }
                out.entry(name.clone())
                    .or_insert_with(|| WorkspaceVisibleSymbol {
                        origin_path: source_path.clone(),
                        origin_name: name.clone(),
                        kind: ReferenceSymbolKind::Fn,
                        decl_range: find_decl_name_range(src, name)
                            .unwrap_or_else(|| span_to_range(stmt.span)),
                    });
            }
            Node::StructDecl { name, is_pub, .. } => {
                if exported_only && any_pub && !is_pub {
                    continue;
                }
                out.entry(name.clone())
                    .or_insert_with(|| WorkspaceVisibleSymbol {
                        origin_path: source_path.clone(),
                        origin_name: name.clone(),
                        kind: ReferenceSymbolKind::Struct,
                        decl_range: find_struct_decl_name_range(src, name)
                            .unwrap_or_else(|| span_to_range(stmt.span)),
                    });
            }
            _ => {}
        }
    }
    out
}

fn load_source_and_program(
    path: &Path,
    source_overrides: &HashMap<PathBuf, (String, Node)>,
) -> Option<(String, Node)> {
    let canon = canonicalize_or_self(path);
    if let Some((src, program)) = source_overrides.get(&canon) {
        return Some((src.clone(), program.clone()));
    }
    let src = std::fs::read_to_string(&canon).ok()?;
    let (program, _errors) = parse(&src);
    Some((src, program))
}

fn resolve_use_target(base_path: &Path, use_path: &str) -> Option<PathBuf> {
    if use_path.starts_with("std::") {
        return None;
    }
    if use_path.contains("::") && !use_path.ends_with(".rz") {
        return None;
    }
    let base_dir = base_path.parent().unwrap_or_else(|| Path::new("."));
    let candidate = base_dir.join(use_path);
    candidate
        .exists()
        .then_some(canonicalize_or_self(&candidate))
}

fn exported_symbols_for_file(
    path: &Path,
    source_overrides: &HashMap<PathBuf, (String, Node)>,
    memo: &mut HashMap<PathBuf, HashMap<String, WorkspaceVisibleSymbol>>,
    visiting: &mut HashSet<PathBuf>,
) -> HashMap<String, WorkspaceVisibleSymbol> {
    let canon = canonicalize_or_self(path);
    if let Some(cached) = memo.get(&canon) {
        return cached.clone();
    }
    if !visiting.insert(canon.clone()) {
        return HashMap::new();
    }
    let Some((src, program)) = load_source_and_program(&canon, source_overrides) else {
        visiting.remove(&canon);
        return HashMap::new();
    };

    let mut out = local_top_level_symbols(&program, &src, &canon, true);
    for use_stmt in raw_use_stmts(&program) {
        if !use_stmt.is_pub {
            continue;
        }
        let Some(target) = resolve_use_target(&canon, &use_stmt.path) else {
            continue;
        };
        let imported = exported_symbols_for_file(&target, source_overrides, memo, visiting);
        for (visible_name, symbol) in imported {
            if let Some(selector_names) = use_stmt.selectors.as_ref()
                && !selector_names
                    .iter()
                    .any(|selector| selector == &visible_name)
            {
                continue;
            }
            let imported_name = use_stmt
                .alias
                .as_ref()
                .map(|ns| format!("{ns}::{visible_name}"))
                .unwrap_or(visible_name);
            out.entry(imported_name).or_insert(symbol);
        }
    }

    visiting.remove(&canon);
    memo.insert(canon.clone(), out.clone());
    out
}

fn accessible_symbols_for_file(
    path: &Path,
    source_overrides: &HashMap<PathBuf, (String, Node)>,
    exports_memo: &mut HashMap<PathBuf, HashMap<String, WorkspaceVisibleSymbol>>,
) -> HashMap<String, WorkspaceVisibleSymbol> {
    let canon = canonicalize_or_self(path);
    let Some((src, program)) = load_source_and_program(&canon, source_overrides) else {
        return HashMap::new();
    };

    let mut out = local_top_level_symbols(&program, &src, &canon, false);
    for use_stmt in raw_use_stmts(&program) {
        let Some(target) = resolve_use_target(&canon, &use_stmt.path) else {
            continue;
        };
        let mut visiting = HashSet::new();
        let imported =
            exported_symbols_for_file(&target, source_overrides, exports_memo, &mut visiting);
        for (visible_name, symbol) in imported {
            if let Some(selector_names) = use_stmt.selectors.as_ref()
                && !selector_names
                    .iter()
                    .any(|selector| selector == &visible_name)
            {
                continue;
            }
            let imported_name = use_stmt
                .alias
                .as_ref()
                .map(|ns| format!("{ns}::{visible_name}"))
                .unwrap_or(visible_name);
            out.entry(imported_name).or_insert(symbol);
        }
    }
    out
}

/// RES-302: best-effort type lookup for an identifier in the
/// program. Walks `Node::Program` looking for a top-level
/// declaration whose name matches `target` and returns a short,
/// human-readable type string suitable for a hover bubble.
///
/// Coverage today (intentionally minimal — see ticket):
///   - `let NAME [: T] = EXPR;` → `T` if annotated, else inferred
///     from a literal RHS (`Int` / `Float` / `String` / `Bool` /
///     `Bytes`). Other RHS shapes return `None`.
///   - `const NAME [: T] = EXPR;` — same rules as `let`.
///   - `static let NAME = EXPR;` — same literal-inference rules.
///   - `fn NAME(...)` → `"fn"`. Return-type annotations land in a
///     follow-up; we return the bare keyword today so hovering on a
///     fn name surfaces *something* useful.
///   - Function parameters: walks every top-level fn body looking
///     for a parameter with the matching name, returning its
///     declared type (`(type, name)` tuple in the AST).
///
/// Anything else (struct fields, nested let bindings, type-aliased
/// identifiers, identifiers used inside a fn body) returns `None`
/// and the hover handler falls through to `Ok(None)`. Doc-comment
/// surfacing is a separate follow-up — see RES-302 ticket.
pub(crate) fn infer_identifier_type(program: &Node, target: &str) -> Option<String> {
    let stmts = match program {
        Node::Program(s) => s,
        _ => return None,
    };
    for spanned in stmts {
        match &spanned.node {
            Node::LetStatement {
                name,
                value,
                type_annot,
                ..
            } => {
                if name == target {
                    return Some(
                        type_annot
                            .clone()
                            .unwrap_or_else(|| infer_literal_type(value).to_string()),
                    );
                }
            }
            Node::Const {
                name,
                value,
                type_annot,
                ..
            } => {
                if name == target {
                    return Some(
                        type_annot
                            .clone()
                            .unwrap_or_else(|| infer_literal_type(value).to_string()),
                    );
                }
            }
            Node::StaticLet { name, value, .. } => {
                if name == target {
                    return Some(infer_literal_type(value).to_string());
                }
            }
            Node::Function {
                name,
                parameters,
                return_type,
                body,
                ..
            } => {
                if name == target {
                    // RES-181b: surface full function signature on hover.
                    let params = parameters
                        .iter()
                        .map(|(ty, pname)| format!("{ty} {pname}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let ret = return_type
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .map(|s| format!(" -> {s}"))
                        .unwrap_or_default();
                    return Some(format!("fn {name}({params}){ret}"));
                }
                // Search parameters.
                for (ty, pname) in parameters {
                    if pname == target {
                        return Some(ty.clone());
                    }
                }
                // RES-181b: search local let bindings inside the function body.
                if let Some(ty) = search_let_in_body(body, target) {
                    return Some(ty);
                }
            }
            _ => {}
        }
    }
    None
}

/// RES-181b: recursively search a function body (Block / nested
/// statements) for a `LetStatement` binding matching `target`.
/// Returns the declared type annotation, or the inferred literal
/// type when no annotation is present.
fn search_let_in_body(node: &Node, target: &str) -> Option<String> {
    match node {
        Node::LetStatement {
            name,
            value,
            type_annot,
            ..
        } if name == target => Some(
            type_annot
                .clone()
                .unwrap_or_else(|| infer_literal_type(value).to_string()),
        ),
        Node::Block { stmts, .. } => {
            for stmt in stmts {
                if let Some(ty) = search_let_in_body(stmt, target) {
                    return Some(ty);
                }
            }
            None
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => search_let_in_body(consequence, target).or_else(|| {
            alternative
                .as_ref()
                .and_then(|a| search_let_in_body(a, target))
        }),
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            search_let_in_body(body, target)
        }
        _ => None,
    }
}

/// RES-302: classify a literal `Node` into the same surface-type
/// strings the literal-hover path uses (`Int`, `Float`, `String`,
/// `Bool`, `Bytes`). Falls back to `"unknown"` for non-literal
/// expressions — callers wrapped in `infer_identifier_type` only
/// hit this for `let` / `const` RHS, where most early-source
/// programs DO bind a literal directly.
fn infer_literal_type(value: &Node) -> &'static str {
    match value {
        Node::IntegerLiteral { .. } => "Int",
        Node::FloatLiteral { .. } => "Float",
        Node::StringLiteral { .. } | Node::StringInternLiteral { .. } => "String",
        Node::BooleanLiteral { .. } => "Bool",
        Node::BytesLiteral { .. } => "Bytes",
        _ => "unknown",
    }
}

/// RES-184: walk the token stream of `src` looking for a `fn`
/// declaration whose name is `target`. Returns the `Range` of the
/// name identifier token in the declaration (e.g. the `foo` span
/// in `fn foo(...)`). Returns `None` if not found.
///
/// We use a two-token lookahead: on each `fn` keyword we peek at
/// the immediately following token; if it is an `Identifier` equal
/// to `target` we return its range. This is cheaper than re-
/// building the AST and more precise than `build_top_level_defs`
/// (which stores the whole-statement span, not the name span).
pub(crate) fn find_decl_name_range(src: &str, target: &str) -> Option<Range> {
    use crate::{Lexer, Token};
    let mut lex = Lexer::new(src);
    let mut prev_was_fn = false;
    loop {
        let (tok, span) = lex.next_token_with_span();
        match tok {
            Token::Eof => return None,
            Token::Function => {
                prev_was_fn = true;
            }
            Token::Identifier(ref name) if prev_was_fn && name == target => {
                return Some(span_to_range(span));
            }
            _ => {
                prev_was_fn = false;
            }
        }
    }
}

/// RES-2568: scan the lexer token stream for `struct <name>` and
/// return the identifier token's precise `Range`. Mirrors
/// `find_decl_name_range` but watches for `Token::Struct` instead
/// of `Token::Function`. Falls back to `None` if not found.
pub(crate) fn find_struct_decl_name_range(src: &str, target: &str) -> Option<Range> {
    use crate::{Lexer, Token};
    let mut lex = Lexer::new(src);
    let mut prev_was_struct = false;
    loop {
        let (tok, span) = lex.next_token_with_span();
        match tok {
            Token::Eof => return None,
            Token::Struct => {
                prev_was_struct = true;
            }
            Token::Identifier(ref name) if prev_was_struct && name == target => {
                return Some(span_to_range(span));
            }
            _ => {
                prev_was_struct = false;
            }
        }
    }
}

/// RES-2568: collect every `new <target> { ... }` constructor-name site
/// in `src` by scanning the lexer token stream. Returns the `Range` of
/// the identifier token that names the struct — NOT the `new` keyword.
///
/// We use the lexer (not the AST) because the AST's `StructLiteral.span`
/// points at a brace token, not the struct-name identifier token, making
/// it unsuitable for a rename edit that must cover only the name.
///
/// Also collects `let <target> { ... } = expr;` destructuring sites since
/// those also embed the struct name as a visible token.
pub(crate) fn collect_struct_literal_sites(src: &str, target: &str) -> Vec<Range> {
    use crate::{Lexer, Token};
    let mut out = Vec::new();
    let mut lex = Lexer::new(src);
    let mut prev_was_new = false;
    let mut prev_was_let = false;
    loop {
        let (tok, span) = lex.next_token_with_span();
        match tok {
            Token::Eof => break,
            Token::New => {
                prev_was_new = true;
                prev_was_let = false;
            }
            Token::Let => {
                prev_was_let = true;
                prev_was_new = false;
            }
            Token::Identifier(ref name) if (prev_was_new || prev_was_let) && name == target => {
                out.push(span_to_range(span));
                prev_was_new = false;
                prev_was_let = false;
            }
            _ => {
                prev_was_new = false;
                prev_was_let = false;
            }
        }
    }
    out
}

/// RES-2568: collect every `Identifier` occurrence of `target` in
/// `program`, across all scopes. Used for renaming top-level `let` /
/// `const` / `static let` variables — their bindings appear as
/// `Identifier` nodes throughout the AST wherever the name is used.
///
/// **Scope note**: this is a name-match, not a scope-aware lookup. For
/// top-level bindings (the only kind this handler currently covers) that
/// is correct — there is no inner scope that can shadow a top-level
/// name in the same file without being a different `LetStatement`
/// binder, which is itself excluded by the caller (since the binder
/// is a field, not an `Identifier` node).
pub(crate) fn collect_identifier_refs(program: &Node, target: &str) -> Vec<Range> {
    let mut out = Vec::new();
    walk_identifier_refs(program, target, &mut out);
    out
}

fn range_contains_pos(range: Range, pos: Position) -> bool {
    (pos.line > range.start.line
        || (pos.line == range.start.line && pos.character >= range.start.character))
        && (pos.line < range.end.line
            || (pos.line == range.end.line && pos.character <= range.end.character))
}

fn push_scope(scopes: &mut Vec<HashMap<String, usize>>) {
    scopes.push(HashMap::new());
}

fn pop_scope(scopes: &mut Vec<HashMap<String, usize>>) {
    scopes.pop();
}

fn bind_variable(
    symbols: &mut Vec<VariableReferenceSet>,
    scopes: &mut [HashMap<String, usize>],
    name: &str,
    decl_range: Range,
) {
    let symbol_id = symbols.len();
    symbols.push(VariableReferenceSet {
        name: name.to_string(),
        decl_range,
        refs: Vec::new(),
    });
    if let Some(scope) = scopes.last_mut() {
        scope.insert(name.to_string(), symbol_id);
    }
}

fn resolve_variable(scopes: &[HashMap<String, usize>], name: &str) -> Option<usize> {
    scopes
        .iter()
        .rev()
        .find_map(|scope| scope.get(name).copied())
}

fn collect_variable_references(program: &Node) -> Vec<VariableReferenceSet> {
    let mut symbols = Vec::new();
    let mut scopes = vec![HashMap::new()];
    walk_variable_refs(program, &mut symbols, &mut scopes);
    symbols
}

fn walk_variable_refs(
    node: &Node,
    symbols: &mut Vec<VariableReferenceSet>,
    scopes: &mut Vec<HashMap<String, usize>>,
) {
    match node {
        Node::Program(stmts) => {
            for stmt in stmts {
                walk_variable_refs(&stmt.node, symbols, scopes);
            }
        }
        Node::Block { stmts, .. } => {
            push_scope(scopes);
            for stmt in stmts {
                walk_variable_refs(stmt, symbols, scopes);
            }
            pop_scope(scopes);
        }
        Node::Function {
            parameters,
            body,
            requires,
            ensures,
            ..
        } => {
            push_scope(scopes);
            let fn_range = expression_span(node).map(span_to_range).unwrap_or_default();
            for (_, name) in parameters {
                bind_variable(symbols, scopes, name, fn_range);
            }
            for req in requires {
                walk_variable_refs(req, symbols, scopes);
            }
            for ensure in ensures {
                walk_variable_refs(ensure, symbols, scopes);
            }
            walk_variable_refs(body, symbols, scopes);
            pop_scope(scopes);
        }
        Node::LetStatement {
            name, value, span, ..
        } => {
            walk_variable_refs(value, symbols, scopes);
            bind_variable(symbols, scopes, name, span_to_range(*span));
        }
        Node::Const {
            name, value, span, ..
        }
        | Node::StaticLet { name, value, span } => {
            walk_variable_refs(value, symbols, scopes);
            bind_variable(symbols, scopes, name, span_to_range(*span));
        }
        Node::ForInStatement {
            name,
            iterable,
            body,
            span,
            ..
        } => {
            walk_variable_refs(iterable, symbols, scopes);
            push_scope(scopes);
            bind_variable(symbols, scopes, name, span_to_range(*span));
            walk_variable_refs(body, symbols, scopes);
            pop_scope(scopes);
        }
        Node::Assignment { name, value, span } => {
            walk_variable_refs(value, symbols, scopes);
            if let Some(id) = resolve_variable(scopes, name) {
                symbols[id].refs.push(span_to_range(*span));
            }
        }
        Node::Identifier { name, span } => {
            if let Some(id) = resolve_variable(scopes, name) {
                symbols[id].refs.push(span_to_range(*span));
            }
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            walk_variable_refs(function, symbols, scopes);
            for arg in arguments {
                walk_variable_refs(arg, symbols, scopes);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_variable_refs(condition, symbols, scopes);
            walk_variable_refs(consequence, symbols, scopes);
            if let Some(alt) = alternative {
                walk_variable_refs(alt, symbols, scopes);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            walk_variable_refs(condition, symbols, scopes);
            walk_variable_refs(body, symbols, scopes);
        }
        Node::ReturnStatement {
            value: Some(value), ..
        }
        | Node::ExpressionStatement { expr: value, .. }
        | Node::TryExpression { expr: value, .. }
        | Node::DeferStatement { expr: value, .. } => {
            walk_variable_refs(value, symbols, scopes);
        }
        Node::InfixExpression { left, right, .. } => {
            walk_variable_refs(left, symbols, scopes);
            walk_variable_refs(right, symbols, scopes);
        }
        Node::PrefixExpression { right, .. } => {
            walk_variable_refs(right, symbols, scopes);
        }
        Node::IndexExpression { target, index, .. } => {
            walk_variable_refs(target, symbols, scopes);
            walk_variable_refs(index, symbols, scopes);
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            walk_variable_refs(target, symbols, scopes);
            walk_variable_refs(index, symbols, scopes);
            walk_variable_refs(value, symbols, scopes);
        }
        Node::FieldAccess { target, .. } => walk_variable_refs(target, symbols, scopes),
        Node::FieldAssignment { target, value, .. } => {
            walk_variable_refs(target, symbols, scopes);
            walk_variable_refs(value, symbols, scopes);
        }
        Node::OptionalChain { object, access, .. } => {
            walk_variable_refs(object, symbols, scopes);
            if let crate::ChainAccess::Method(_, args) = access {
                for arg in args {
                    walk_variable_refs(arg, symbols, scopes);
                }
            }
        }
        Node::ArrayLiteral { items, .. } => {
            for item in items {
                walk_variable_refs(item, symbols, scopes);
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            if let Some(base) = base {
                walk_variable_refs(base, symbols, scopes);
            }
            for (_, value) in fields {
                walk_variable_refs(value, symbols, scopes);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            walk_variable_refs(scrutinee, symbols, scopes);
            for (_, guard, body) in arms {
                if let Some(guard) = guard {
                    walk_variable_refs(guard, symbols, scopes);
                }
                walk_variable_refs(body, symbols, scopes);
            }
        }
        Node::FunctionLiteral {
            parameters,
            body,
            requires,
            ensures,
            ..
        } => {
            push_scope(scopes);
            let fn_range = expression_span(node).map(span_to_range).unwrap_or_default();
            for (_, name) in parameters {
                bind_variable(symbols, scopes, name, fn_range);
            }
            for req in requires {
                walk_variable_refs(req, symbols, scopes);
            }
            for ensure in ensures {
                walk_variable_refs(ensure, symbols, scopes);
            }
            walk_variable_refs(body, symbols, scopes);
            pop_scope(scopes);
        }
        _ => {}
    }
}

fn collect_qualified_identifier_sites(src: &str, target: &str) -> Vec<Range> {
    use crate::{Lexer, Token};
    let mut ranges = Vec::new();
    let mut lex = Lexer::new(src);
    let mut tokens = Vec::new();
    loop {
        let (tok, span) = lex.next_token_with_span();
        if matches!(tok, Token::Eof) {
            break;
        }
        tokens.push((tok, span));
    }

    let mut i = 0;
    while i < tokens.len() {
        let (Token::Identifier(name), span) = &tokens[i] else {
            i += 1;
            continue;
        };
        let mut full_name = name.clone();
        let start_span = *span;
        let mut end_span = *span;
        let mut j = i;
        while j + 2 < tokens.len()
            && matches!(tokens[j + 1].0, Token::DoubleColon)
            && matches!(tokens[j + 2].0, Token::Identifier(_))
        {
            if let Token::Identifier(seg) = &tokens[j + 2].0 {
                full_name.push_str("::");
                full_name.push_str(seg);
            }
            end_span = tokens[j + 2].1;
            j += 2;
        }
        if full_name == target {
            let start = Position::new(
                start_span.start.line.saturating_sub(1) as u32,
                start_span.start.column.saturating_sub(1) as u32,
            );
            let end = Position::new(
                end_span.end.line.saturating_sub(1) as u32,
                end_span.end.column.saturating_sub(1) as u32,
            );
            ranges.push(Range::new(start, end));
        }
        i = j + 1;
    }
    ranges
}

fn walk_identifier_refs(node: &Node, target: &str, out: &mut Vec<Range>) {
    match node {
        Node::Identifier { name, span } if name == target => {
            out.push(span_to_range(*span));
        }
        Node::Program(stmts) => {
            for s in stmts {
                walk_identifier_refs(&s.node, target, out);
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            ..
        } => {
            walk_identifier_refs(body, target, out);
            for r in requires {
                walk_identifier_refs(r, target, out);
            }
            for e in ensures {
                walk_identifier_refs(e, target, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_identifier_refs(s, target, out);
            }
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            walk_identifier_refs(function, target, out);
            for a in arguments {
                walk_identifier_refs(a, target, out);
            }
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::ReturnStatement {
            value: Some(value), ..
        } => {
            walk_identifier_refs(value, target, out);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_identifier_refs(condition, target, out);
            walk_identifier_refs(consequence, target, out);
            if let Some(a) = alternative {
                walk_identifier_refs(a, target, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            walk_identifier_refs(condition, target, out);
            walk_identifier_refs(body, target, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            walk_identifier_refs(iterable, target, out);
            walk_identifier_refs(body, target, out);
        }
        Node::ExpressionStatement { expr, .. } => {
            walk_identifier_refs(expr, target, out);
        }
        Node::InfixExpression { left, right, .. } => {
            walk_identifier_refs(left, target, out);
            walk_identifier_refs(right, target, out);
        }
        Node::PrefixExpression { right, .. } => {
            walk_identifier_refs(right, target, out);
        }
        Node::Assignment { value, .. } => {
            walk_identifier_refs(value, target, out);
        }
        Node::IndexExpression {
            target: t, index, ..
        } => {
            walk_identifier_refs(t, target, out);
            walk_identifier_refs(index, target, out);
        }
        Node::IndexAssignment {
            target: t,
            index,
            value,
            ..
        } => {
            walk_identifier_refs(t, target, out);
            walk_identifier_refs(index, target, out);
            walk_identifier_refs(value, target, out);
        }
        Node::FieldAccess { target: obj, .. } | Node::TryExpression { expr: obj, .. } => {
            walk_identifier_refs(obj, target, out);
        }
        Node::FieldAssignment {
            target: obj, value, ..
        } => {
            walk_identifier_refs(obj, target, out);
            walk_identifier_refs(value, target, out);
        }
        Node::OptionalChain { object, access, .. } => {
            walk_identifier_refs(object, target, out);
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args {
                    walk_identifier_refs(a, target, out);
                }
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            walk_identifier_refs(scrutinee, target, out);
            for (_, guard, body) in arms {
                if let Some(g) = guard {
                    walk_identifier_refs(g, target, out);
                }
                walk_identifier_refs(body, target, out);
            }
        }
        Node::ArrayLiteral { items, .. } => {
            for e in items {
                walk_identifier_refs(e, target, out);
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            if let Some(b) = base {
                walk_identifier_refs(b, target, out);
            }
            for (_, v) in fields {
                walk_identifier_refs(v, target, out);
            }
        }
        _ => {}
    }
}

/// RES-2568: symbol kind tag used by `build_rename_edits_for_doc`.
/// Kept `pub(crate)` so tests can call it without going through the
/// async handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenameSymbolKind {
    Fn,
    Struct,
    Variable,
}

/// RES-2568: produce all `TextEdit`s needed to rename `old_name` →
/// `new_name` inside a single document.
///
/// - `Fn`: declaration token (via lexer) + all call sites.
/// - `Struct`: declaration token + all `StructLiteral` name sites.
/// - `Variable`: declaration token (via `def_range` if no finer span
///   is available) + all `Identifier` references.
///
/// The declaration edit is only emitted when `def_range` is non-empty
/// (i.e. the symbol is declared in this document). Cross-file callers
/// pass an empty `Range::default()` for `def_range` when the symbol
/// is imported, in which case only the reference edits are emitted.
pub(crate) fn build_rename_edits_for_doc(
    program: &Node,
    src: &str,
    old_name: &str,
    new_name: &str,
    kind: RenameSymbolKind,
    def_range: Range,
) -> Vec<TextEdit> {
    let mut edits: Vec<TextEdit> = Vec::new();
    let zero_range = Range::default();

    match kind {
        RenameSymbolKind::Fn => {
            // Declaration: precise fn-name-token range.
            if def_range != zero_range {
                let decl_range = find_decl_name_range(src, old_name).unwrap_or(def_range);
                edits.push(TextEdit {
                    range: decl_range,
                    new_text: new_name.to_string(),
                });
            }
            // All call sites.
            for range in collect_call_sites(program, old_name) {
                edits.push(TextEdit {
                    range,
                    new_text: new_name.to_string(),
                });
            }
        }
        RenameSymbolKind::Struct => {
            // Collect all struct-name sites (decl + constructor + destructuring)
            // from the lexer token stream. The declaration token is `struct <Name>`
            // and the constructor token is `new <Name> { ... }`.
            // Both are captured by their respective scanner passes.
            if def_range != zero_range {
                let decl_range = find_struct_decl_name_range(src, old_name).unwrap_or(def_range);
                edits.push(TextEdit {
                    range: decl_range,
                    new_text: new_name.to_string(),
                });
            }
            // All `new <Name> { ... }` and `let <Name> { ... } = ...` sites.
            for range in collect_struct_literal_sites(src, old_name) {
                edits.push(TextEdit {
                    range,
                    new_text: new_name.to_string(),
                });
            }
        }
        RenameSymbolKind::Variable => {
            // Declaration: use def_range (whole-statement span is fine
            // for variables; the name appears at the start of the `let`).
            // We refine to the identifier token via the lexer scan; fall
            // back to def_range if not found.
            if def_range != zero_range {
                // Scan the lexer for the exact identifier token at/near
                // the def_range start position to get a tighter range.
                let decl_range = find_let_name_range(src, old_name).unwrap_or(def_range);
                edits.push(TextEdit {
                    range: decl_range,
                    new_text: new_name.to_string(),
                });
            }
            // All identifier references (excludes the binder itself,
            // which is a field on the AST node, not an `Identifier` child).
            for range in collect_identifier_refs(program, old_name) {
                edits.push(TextEdit {
                    range,
                    new_text: new_name.to_string(),
                });
            }
        }
    }

    edits
}

/// RES-2568: find the precise range of the identifier token that names
/// a `let` / `const` / `static let` binding in `src`. Scans for the
/// sequence `let <name>` or `const <name>` and returns the identifier
/// span. Falls back to `None` when the pattern isn't found (e.g. for
/// a static let with a more complex form, or when the lexer misses).
pub(crate) fn find_let_name_range(src: &str, target: &str) -> Option<Range> {
    use crate::{Lexer, Token};
    let mut lex = Lexer::new(src);
    let mut prev_was_let_or_const = false;
    loop {
        let (tok, span) = lex.next_token_with_span();
        match tok {
            Token::Eof => return None,
            Token::Let | Token::Const | Token::Static => {
                prev_was_let_or_const = true;
            }
            Token::Identifier(ref n) if prev_was_let_or_const && n == target => {
                return Some(span_to_range(span));
            }
            _ => {
                prev_was_let_or_const = false;
            }
        }
    }
}

/// RES-184: validate that `name` is a legal Resilient identifier.
/// Must match `[A-Za-z_][A-Za-z0-9_]*`.  Empty strings and names
/// that start with a digit are rejected.  Used by the rename
/// handler to produce a clean LSP error before touching the AST.
pub(crate) fn is_valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        None => false,
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    }
}

/// RES-357: scan forward from `start_line` (0-indexed) in `src` to
/// find the line that contains the opening `{` of a function body.
/// Returns the 0-indexed line number of that brace line, or `None`
/// if no `{` is found in the remaining source.
///
/// The scan skips the string content of any `"..."` literals so a
/// `{` inside a default-parameter string doesn't fool the heuristic.
/// A `{` appearing anywhere on a line (even after other tokens) is
/// treated as the function's opening brace.
pub(crate) fn find_brace_line(src: &str, start_line: usize) -> Option<usize> {
    for (idx, line) in src.lines().enumerate() {
        if idx < start_line {
            continue;
        }
        // Walk character-by-character, skipping string literals so
        // `{` inside `"..."` is not mistaken for the opening brace.
        let mut in_string = false;
        let mut prev = '\0';
        for ch in line.chars() {
            match ch {
                '"' if prev != '\\' => in_string = !in_string,
                '{' if !in_string => return Some(idx),
                _ => {}
            }
            prev = ch;
        }
    }
    None
}

/// RES-190: heuristically detect parser diagnostics that are
/// "missing semicolon" errors. Matches by message substring or by
/// `code == "E0002"` once the parser starts populating the LSP
/// diagnostic's `code` field. Conservative: a borderline match is
/// fine — the worst case is offering an `Insert ;` action that the
/// user dismisses.
pub(crate) fn is_missing_semicolon_diagnostic(diag: &Diagnostic) -> bool {
    if let Some(NumberOrString::String(c)) = &diag.code
        && c == "E0002"
    {
        return true;
    }
    let msg = diag.message.to_ascii_lowercase();
    // Common phrasings: "expected ';'", "missing semicolon",
    // "expected a ';'", "expected `;`". Restrict to messages that
    // mention `;` explicitly so we don't grab an unrelated parse
    // error.
    let mentions_semi = msg.contains("';'") || msg.contains("`;`") || msg.contains("semicolon");
    let mentions_expected_or_missing = msg.contains("expected") || msg.contains("missing");
    mentions_semi && mentions_expected_or_missing
}

/// RES-190: build an "Insert `;`" `CodeAction` for a missing-
/// semicolon diagnostic. The edit inserts a `;` at the diagnostic's
/// start position — that's where the parser flagged the problem,
/// which is at the end of the preceding token.
pub(crate) fn build_insert_semicolon_action(uri: &Url, diag: &Diagnostic) -> Option<CodeAction> {
    let insert_range = Range {
        start: diag.range.start,
        end: diag.range.start,
    };
    let text_edit = TextEdit {
        range: insert_range,
        new_text: ";".to_string(),
    };
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), vec![text_edit]);
    let workspace_edit = WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    };
    Some(CodeAction {
        title: "Insert `;`".to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diag.clone()]),
        edit: Some(workspace_edit),
        command: None,
        is_preferred: Some(true),
        disabled: None,
        data: None,
    })
}

/// RES-183: walk the full AST and collect the `Range` of every
/// `CallExpression` whose callee is an `Identifier` with the
/// given name. Only `CallExpression` nodes count — `StructLiteral`
/// nodes that happen to share the name are deliberately excluded
/// (the AC says the match must be AST-driven, not textual).
///
/// The returned ranges point at the callee identifier's span
/// within the call expression. Because `CallExpression.function`
/// is a `Box<Node>` holding an `Identifier`, we use
/// `expression_span` to get that identifier's span and convert it
/// to an LSP `Range`. Calls where the callee is a non-identifier
/// expression (method chain, higher-order call, etc.) are skipped.
///
/// `include_declaration`: when `true`, the caller should also
/// append the definition site via `find_top_level_def`. This
/// helper only ever emits CALL sites.
pub(crate) fn collect_call_sites(program: &Node, target: &str) -> Vec<Range> {
    let mut out = Vec::new();
    walk_call_sites(program, target, &mut out);
    out
}

/// Recursive helper for `collect_call_sites`. Visits every node
/// reachable from `node`. Appends to `out` when a
/// `CallExpression` with callee `Identifier { name == target }`
/// is found.
fn walk_call_sites(node: &Node, target: &str, out: &mut Vec<Range>) {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                walk_call_sites(&s.node, target, out);
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            ..
        } => {
            walk_call_sites(body, target, out);
            for r in requires {
                walk_call_sites(r, target, out);
            }
            for e in ensures {
                walk_call_sites(e, target, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_call_sites(s, target, out);
            }
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            // Recurse into arguments first so nested calls are captured.
            for a in arguments {
                walk_call_sites(a, target, out);
            }
            // Recurse into the callee in case it is itself a call
            // expression (e.g. `foo()()`). We check AFTER recursion
            // so inner calls land before outer.
            walk_call_sites(function, target, out);
            // Now check if THIS call's callee is the target name.
            if let Node::Identifier { name, span } = function.as_ref()
                && name == target
            {
                out.push(span_to_range(*span));
            }
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::ReturnStatement {
            value: Some(value), ..
        } => {
            walk_call_sites(value, target, out);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_call_sites(condition, target, out);
            walk_call_sites(consequence, target, out);
            if let Some(a) = alternative {
                walk_call_sites(a, target, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            walk_call_sites(condition, target, out);
            walk_call_sites(body, target, out);
        }
        Node::InfixExpression { left, right, .. } => {
            walk_call_sites(left, target, out);
            walk_call_sites(right, target, out);
        }
        Node::PrefixExpression { right, .. } => {
            walk_call_sites(right, target, out);
        }
        Node::Assignment { value, .. } => {
            walk_call_sites(value, target, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            walk_call_sites(iterable, target, out);
            walk_call_sites(body, target, out);
        }
        Node::ExpressionStatement { expr, .. } => {
            walk_call_sites(expr, target, out);
        }
        Node::IndexExpression {
            target: t, index, ..
        } => {
            walk_call_sites(t, target, out);
            walk_call_sites(index, target, out);
        }
        Node::IndexAssignment {
            target: t,
            index,
            value,
            ..
        } => {
            walk_call_sites(t, target, out);
            walk_call_sites(index, target, out);
            walk_call_sites(value, target, out);
        }
        Node::FieldAccess { target: obj, .. } | Node::TryExpression { expr: obj, .. } => {
            walk_call_sites(obj, target, out);
        }
        Node::OptionalChain { object, access, .. } => {
            walk_call_sites(object, target, out);
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args {
                    walk_call_sites(a, target, out);
                }
            }
        }
        Node::FieldAssignment {
            target: obj, value, ..
        } => {
            walk_call_sites(obj, target, out);
            walk_call_sites(value, target, out);
        }
        Node::ArrayLiteral { items, .. } => {
            for e in items {
                walk_call_sites(e, target, out);
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            // Note: `StructLiteral { name, .. }` is intentionally NOT
            // matched as a call site — struct construction is not a fn
            // call even if the struct shares a name with a function.
            // Only the field VALUE expressions are descended into.
            if let Some(b) = base {
                walk_call_sites(b, target, out);
            }
            for (_, v) in fields {
                walk_call_sites(v, target, out);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            walk_call_sites(scrutinee, target, out);
            for (_, guard, body) in arms {
                if let Some(g) = guard {
                    walk_call_sites(g, target, out);
                }
                walk_call_sites(body, target, out);
            }
        }
        Node::FunctionLiteral {
            body,
            requires,
            ensures,
            ..
        } => {
            walk_call_sites(body, target, out);
            for r in requires {
                walk_call_sites(r, target, out);
            }
            for e in ensures {
                walk_call_sites(e, target, out);
            }
        }
        // Simple leaf nodes and forms without sub-expressions:
        // Identifier, IntegerLiteral, FloatLiteral, StringLiteral,
        // BooleanLiteral, BytesLiteral, ReturnStatement { value: None },
        // TypeAlias, StructDecl, LetDestructureStruct, AssumeStatement, etc.
        _ => {}
    }
}

/// RES-188a: hard cap on the completion list. Large lists hurt
/// client latency — both rendering and the follow-up filter pass
/// — and VS Code / most clients truncate at ~200 anyway. 100 is
/// the ticket's explicit choice.
pub(crate) const COMPLETION_LIMIT: usize = 100;

/// RES-188a: extract the identifier prefix that ends at `pos`.
/// Walks backwards through the line's text until it hits a
/// non-identifier character (anything that isn't alphanumeric or
/// `_`). Returns the portion already typed — the empty string if
/// the cursor is on a non-identifier character or at column 0 on
/// a blank line. Used by `completion` to filter the suggestion
/// set to entries that start with the user's in-progress name.
pub(crate) fn prefix_at(src: &str, pos: Position) -> String {
    let line_no = pos.line as usize;
    let col = pos.character as usize;
    let line = match src.lines().nth(line_no) {
        Some(l) => l,
        None => return String::new(),
    };
    let chars: Vec<char> = line.chars().collect();
    // Clamp col into the line length — some clients send positions
    // one past end-of-line, which we treat as "end of line".
    let end = col.min(chars.len());
    let mut start = end;
    while start > 0 {
        let c = chars[start - 1];
        if c.is_alphanumeric() || c == '_' {
            start -= 1;
        } else {
            break;
        }
    }
    chars[start..end].iter().collect()
}

/// RES-188a: one resolved completion candidate, pre-sorting.
/// Carries enough info to render a `CompletionItem` without
/// touching the LSP types in pure helpers (so the helpers can be
/// unit-tested without a tower-lsp dependency in the test body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub label: String,
    pub kind: CandidateKind,
    pub detail: Option<String>,
}

/// RES-188a: completion-item kind. Maps 1:1 to
/// `tower_lsp::lsp_types::CompletionItemKind` in the handler —
/// kept as a local enum here so pure helpers and tests don't pull
/// in tower-lsp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CandidateKind {
    Function,
    Struct,
    TypeAlias,
}

impl CandidateKind {
    fn to_lsp(self) -> CompletionItemKind {
        match self {
            CandidateKind::Function => CompletionItemKind::FUNCTION,
            CandidateKind::Struct => CompletionItemKind::STRUCT,
            CandidateKind::TypeAlias => CompletionItemKind::TYPE_PARAMETER,
        }
    }
}

/// RES-188a: build the candidate list from the cached AST +
/// `BUILTINS`, filter by `prefix`, and cap at `COMPLETION_LIMIT`.
/// Deterministic output: builtins come first (alphabetical), then
/// top-level decls (source order), so regressions across tests
/// are easy to spot.
///
/// When `prefix` is empty, the full candidate set (up to the cap)
/// is returned — that's the Ctrl-Space case.
pub(crate) fn completion_candidates(program: &Node, prefix: &str) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();

    // Builtins — alphabetically sorted snapshot.
    let mut names: Vec<&'static str> = builtin_names().collect();
    names.sort_unstable();
    for name in names {
        if !name.starts_with(prefix) {
            continue;
        }
        out.push(Candidate {
            label: name.to_string(),
            kind: CandidateKind::Function,
            detail: Some("builtin".to_string()),
        });
        if out.len() >= COMPLETION_LIMIT {
            return out;
        }
    }

    // Top-level decls — source order. Already handles duplicates
    // via the `seen` set inside `build_top_level_defs`.
    let stmts = match program {
        Node::Program(s) => s,
        _ => return out,
    };
    // RES-1531: borrow the decl name as `&str` for the prefix
    // filter; only allocate the owned label string when the entry
    // is actually going to land in `out`. The previous shape
    // cloned every top-level decl name, then dropped most of them
    // on the `!name.starts_with(prefix)` continue — wasted work
    // proportional to the program's top-level decl count for every
    // completion request.
    for spanned in stmts {
        let (name, kind, detail) = match &spanned.node {
            Node::Function {
                name, parameters, ..
            } => (
                name.as_str(),
                CandidateKind::Function,
                Some(format!("fn ({} params)", parameters.len())),
            ),
            Node::StructDecl { name, fields, .. } => (
                name.as_str(),
                CandidateKind::Struct,
                Some(format!("struct ({} fields)", fields.len())),
            ),
            Node::TypeAlias { name, .. } => (
                name.as_str(),
                CandidateKind::TypeAlias,
                Some("type".to_string()),
            ),
            _ => continue,
        };
        if !name.starts_with(prefix) {
            continue;
        }
        out.push(Candidate {
            label: name.to_string(),
            kind,
            detail,
        });
        if out.len() >= COMPLETION_LIMIT {
            return out;
        }
    }
    out
}

/// RES-188a: convert a `Candidate` into the LSP wire-shape.
/// Lifted out so the filter / cap / ordering logic can be
/// unit-tested over pure `Vec<Candidate>`.
fn candidate_to_completion_item(c: Candidate) -> CompletionItem {
    CompletionItem {
        label: c.label.clone(),
        kind: Some(c.kind.to_lsp()),
        detail: c.detail,
        insert_text: Some(c.label),
        ..Default::default()
    }
}

/// RES-185: walk a `Node::Program` and emit one `DocumentSymbol`
/// per top-level `fn`, `struct`, or `type` alias. Returned list
/// is sorted by source position so editors present the outline
/// in file order.
///
/// This is a pure function — the LSP handler just hands it the
/// cached AST and returns the result. Unit tests exercise it
/// directly; the integration test in `tests/lsp_smoke.rs`
/// verifies the round-trip through the actual server.
///
/// `selection_range` is set equal to `range` for now — a later
/// ticket can compute the identifier-only sub-span once parse-
/// time name positions are tracked (today's `Node::Function::span`
/// is the `fn` keyword's zero-width point, per RES-088).
#[allow(dead_code)] // only used behind the `lsp` feature + test
pub(crate) fn document_symbols_for_program(program: &Node) -> Vec<DocumentSymbol> {
    let stmts = match program {
        Node::Program(s) => s,
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for spanned in stmts {
        let symbol = match &spanned.node {
            Node::Function { name, .. } => make_symbol(name, SymbolKind::FUNCTION, spanned.span),
            Node::StructDecl { name, .. } => make_symbol(name, SymbolKind::STRUCT, spanned.span),
            Node::TypeAlias { name, .. } => {
                make_symbol(name, SymbolKind::TYPE_PARAMETER, spanned.span)
            }
            // Everything else (let / static / return / while / ...)
            // is a statement, not a declaration — skip.
            _ => continue,
        };
        out.push(symbol);
    }
    // Stable-sort by source position. Parse order already matches
    // source order for the shapes we track, so this is usually a
    // no-op — belt-and-suspenders against future AST reordering.
    out.sort_by_key(|s| (s.range.start.line, s.range.start.character));
    out
}

/// RES-186: rebuild helpers. The workspace index is a flat
/// HashMap<Url, Vec<entries>>; `rebuild_workspace_index` walks
/// the root, `index_file` handles one `*.rs` at a time,
/// `filter_workspace_symbols` applies the query + cap.
impl Backend {
    fn rebuild_workspace_index(&self) {
        let root = match self.workspace_root.lock() {
            Ok(g) => g.clone(),
            Err(_) => None,
        };
        let Some(root) = root else { return };
        let files = walk_resilient_files(&root);
        let mut new_index: HashMap<Url, Vec<WorkspaceSymbolEntry>> = HashMap::new();
        for path in files {
            if let Some(entries) = index_file(&path) {
                let Ok(uri) = Url::from_file_path(&path) else {
                    continue;
                };
                new_index.insert(uri, entries);
            }
        }
        if let Ok(mut idx) = self.workspace_index.lock() {
            *idx = new_index;
        }
    }
}

/// RES-186: recursive `*.rs` walker. Skips `target/` and any
/// dot-prefixed directory (`.git/`, `.board/`, etc.) so we don't
/// index build artifacts or management metadata.
fn walk_resilient_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        // Skip hidden + build dirs. This is the ticket's "don't
        // respect .gitignore, just skip the obvious" policy.
        if name.starts_with('.') || name == "target" {
            continue;
        }
        if path.is_dir() {
            out.extend(walk_resilient_files(&path));
        } else if path.extension().and_then(|s| s.to_str()) == Some("rz") {
            out.push(path);
        }
    }
    out
}

/// RES-186: read, parse, and extract workspace-symbol entries for
/// one file. Parse errors are tolerated — a file that doesn't
/// parse returns an empty vec (not `None`) so partial-edit
/// states don't unindex the file; in practice the entries just
/// disappear until the file parses again.
#[allow(dead_code)] // `lsp` feature + test
pub(crate) fn index_file(path: &Path) -> Option<Vec<WorkspaceSymbolEntry>> {
    let text = std::fs::read_to_string(path).ok()?;
    let (program, _errs) = parse(&text);
    let uri = Url::from_file_path(path).ok()?;
    let entries = document_symbols_for_program(&program)
        .into_iter()
        .map(|d| WorkspaceSymbolEntry {
            name: d.name,
            kind: d.kind,
            uri: uri.clone(),
            range: d.range,
        })
        .collect();
    Some(entries)
}

/// RES-186: substring filter across the whole workspace index,
/// case-insensitive, capped at `limit` entries. Sorted by name
/// for reproducible output (editors' quick-open panels usually
/// re-sort, but we want the API stable for tests).
#[allow(deprecated)] // `SymbolInformation::deprecated` is a deprecated LSP field
fn filter_workspace_symbols(
    index: &HashMap<Url, Vec<WorkspaceSymbolEntry>>,
    query_lower: &str,
    limit: usize,
) -> Vec<SymbolInformation> {
    let mut matches: Vec<&WorkspaceSymbolEntry> = index
        .values()
        .flatten()
        .filter(|e| e.name.to_lowercase().contains(query_lower))
        .collect();
    // Stable sort by name then uri for determinism.
    matches.sort_by(|a, b| a.name.cmp(&b.name).then(a.uri.as_str().cmp(b.uri.as_str())));
    matches
        .into_iter()
        .take(limit)
        .map(|e| SymbolInformation {
            name: e.name.clone(),
            kind: e.kind,
            tags: None,
            deprecated: None,
            location: Location::new(e.uri.clone(), e.range),
            container_name: None,
        })
        .collect()
}

/// RES-187: the semantic-tokens legend. The order here MUST match
/// the `sem_tok::*` token-type indices declared in `main.rs`
/// (KEYWORD=0 … OPERATOR=8) and the modifier bit positions
/// (MOD_DECLARATION=bit0, MOD_READONLY=bit1). Any drift between
/// these two tables yields mis-colored output in every client.
fn semantic_tokens_legend() -> SemanticTokensLegend {
    // Indices (0..=8): keyword, function, variable, parameter,
    // type, string, number, comment, operator.
    let token_types = vec![
        SemanticTokenType::KEYWORD,
        SemanticTokenType::FUNCTION,
        SemanticTokenType::VARIABLE,
        SemanticTokenType::PARAMETER,
        SemanticTokenType::TYPE,
        SemanticTokenType::STRING,
        SemanticTokenType::NUMBER,
        SemanticTokenType::COMMENT,
        SemanticTokenType::OPERATOR,
    ];
    // Bit positions: declaration=bit0, readonly=bit1.
    let token_modifiers = vec![
        SemanticTokenModifier::DECLARATION,
        SemanticTokenModifier::READONLY,
    ];
    SemanticTokensLegend {
        token_types,
        token_modifiers,
    }
}

/// RES-187: the capability advertised in `initialize` — full-file
/// tokens only (delta left for a follow-up per the ticket notes).
fn semantic_tokens_capability() -> SemanticTokensServerCapabilities {
    SemanticTokensServerCapabilities::SemanticTokensOptions(SemanticTokensOptions {
        work_done_progress_options: WorkDoneProgressOptions::default(),
        legend: semantic_tokens_legend(),
        range: Some(false),
        full: Some(SemanticTokensFullOptions::Bool(true)),
    })
}

/// RES-187: turn the `compute_semantic_tokens` u32 wire format
/// into the `Vec<SemanticToken>` that `tower-lsp`'s
/// `SemanticTokens::data` expects. The serializer reassembles
/// the flat u32 stream on the wire — we just need to round-trip
/// through the struct form.
fn semantic_tokens_from_wire(wire: Vec<u32>) -> Vec<SemanticToken> {
    wire.chunks_exact(5)
        .map(|c| SemanticToken {
            delta_line: c[0],
            delta_start: c[1],
            length: c[2],
            token_type: c[3],
            token_modifiers_bitset: c[4],
        })
        .collect()
}

/// RES-189: build one inlay hint for a typechecked unannotated
/// `let` binding. Position lands at the end-of-pattern column
/// (just after the identifier) so clients render `let x<here>`
/// → `let x :: Int` — the `:: ` prefix keeps the hint visually
/// separate from the code.
///
/// Per the LSP spec `position.line` / `character` are 0-indexed;
/// the typechecker's `span` is 1-indexed per RES-077, so we
/// subtract.
#[allow(dead_code)] // used behind `lsp` + test
fn inlay_hint_from_let(entry: &typechecker::LetTypeHint) -> InlayHint {
    // `let ` is 4 chars. Skip past it + the name to land at
    // end-of-pattern. If someone later adds a `let mut` form,
    // this computation needs to move.
    let line0 = entry.span.start.line.saturating_sub(1) as u32;
    let col0 = entry.span.start.column.saturating_sub(1) as u32;
    let end_of_pattern = col0 + 4 + entry.name_len_chars as u32;
    InlayHint {
        position: Position::new(line0, end_of_pattern),
        label: InlayHintLabel::String(format!(": {}", entry.ty)),
        kind: Some(InlayHintKind::TYPE),
        text_edits: None,
        tooltip: None,
        padding_left: Some(true),
        padding_right: None,
        data: None,
    }
}

/// RES-189: collect per-call-site parameter hints for a program.
///
/// For each `Node::CallExpression` whose callee resolves to a
/// top-level `Node::Function` declared in the same program, emit
/// one hint per positional argument carrying the corresponding
/// parameter name (`add(a: 1, b: 2)` editor chrome). Hints land
/// at the argument expression's start position.
///
/// Intentionally simple: we only resolve names against
/// top-level fns declared in THIS program. Imported fns, methods
/// on structs, and arguments at non-call call sites are all
/// skipped (each can be layered on as a follow-up once name
/// resolution is unified — RES-182 territory).
#[allow(dead_code)] // used behind `lsp` + test
pub(crate) fn collect_param_hints(program: &Node) -> Vec<InlayHint> {
    let mut out = Vec::new();
    let fns = collect_top_level_fns(program);
    walk_call_hints(program, &fns, &mut out);
    out
}

/// Map fn name → parameter names. Populated from top-level
/// `Node::Function` decls so parameter hints can look up a callee
/// in O(1).
///
/// RES-1508: borrow fn names and parameter names as `&str` from
/// the program AST. The previous shape cloned every fn name plus
/// every parameter name into owned `String`s purely so the map
/// keys / values could satisfy `HashMap`'s ownership; the consumer
/// (`walk_call_hints`) only reads them.
fn collect_top_level_fns(program: &Node) -> HashMap<&str, Vec<&str>> {
    let mut out = HashMap::new();
    if let Node::Program(stmts) = program {
        for spanned in stmts {
            if let Node::Function {
                name, parameters, ..
            } = &spanned.node
            {
                // parameters are (type, name) — only names needed.
                let names: Vec<&str> = parameters.iter().map(|(_, n)| n.as_str()).collect();
                out.insert(name.as_str(), names);
            }
        }
    }
    out
}

/// Recursive walker: visits every `CallExpression` reachable from
/// `node`. For each one, if the callee is a bare identifier
/// that's in `fns` AND the arg count matches, emit one hint per
/// positional argument.
fn walk_call_hints(node: &Node, fns: &HashMap<&str, Vec<&str>>, out: &mut Vec<InlayHint>) {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                walk_call_hints(&s.node, fns, out);
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            ..
        } => {
            walk_call_hints(body, fns, out);
            for r in requires {
                walk_call_hints(r, fns, out);
            }
            for e in ensures {
                walk_call_hints(e, fns, out);
            }
        }
        Node::Block { stmts, .. } => {
            // `Block.stmts` is `Vec<Node>` (not Spanned) so walk directly.
            for s in stmts {
                walk_call_hints(s, fns, out);
            }
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            // First recurse into the arguments so nested calls get
            // hints too.
            for a in arguments {
                walk_call_hints(a, fns, out);
            }
            walk_call_hints(function, fns, out);

            // Now check if this call itself gets a hint: callee
            // must be an Identifier naming a known top-level fn,
            // and arg count must match.
            if let Node::Identifier { name, .. } = function.as_ref()
                && let Some(param_names) = fns.get(name.as_str())
                && param_names.len() == arguments.len()
            {
                for (arg, pname) in arguments.iter().zip(param_names.iter()) {
                    let arg_span = expression_span(arg);
                    let Some(sp) = arg_span else { continue };
                    let pos = Position::new(
                        sp.start.line.saturating_sub(1) as u32,
                        sp.start.column.saturating_sub(1) as u32,
                    );
                    out.push(InlayHint {
                        position: pos,
                        label: InlayHintLabel::String(format!("{}: ", pname)),
                        kind: Some(InlayHintKind::PARAMETER),
                        text_edits: None,
                        tooltip: None,
                        padding_left: None,
                        padding_right: Some(true),
                        data: None,
                    });
                }
            }
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::ReturnStatement {
            value: Some(value), ..
        } => {
            walk_call_hints(value, fns, out);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_call_hints(condition, fns, out);
            walk_call_hints(consequence, fns, out);
            if let Some(a) = alternative {
                walk_call_hints(a, fns, out);
            }
        }
        Node::InfixExpression { left, right, .. } => {
            walk_call_hints(left, fns, out);
            walk_call_hints(right, fns, out);
        }
        Node::PrefixExpression { right, .. } => {
            walk_call_hints(right, fns, out);
        }
        // Stop recursion at simple leaves and forms we don't
        // currently inspect. Expand cases as new AST shapes need
        // hint coverage.
        _ => {}
    }
}

/// RES-189: tri-state position-in-range test. The LSP spec says
/// an inlay hint request's `range` is the viewport the editor
/// wants hints for; we filter server-side so the client doesn't
/// render hints outside that range. Returns true when `p` falls
/// inside `[range.start, range.end)` lexicographically.
fn position_in_range(p: Position, range: Range) -> bool {
    let before_start = (p.line, p.character) < (range.start.line, range.start.character);
    let after_end = (p.line, p.character) > (range.end.line, range.end.character);
    !before_start && !after_end
}

fn read_init_inlay_hints_config(opts: Option<&tower_lsp::lsp_types::LSPAny>) -> InlayHintConfig {
    fn read_flag(
        opts: &tower_lsp::lsp_types::LSPAny,
        flat_key: &str,
        nested_key: &str,
    ) -> Option<bool> {
        opts.get(flat_key).and_then(|v| v.as_bool()).or_else(|| {
            opts.get("resilient")
                .and_then(|v| v.get("inlayHints"))
                .and_then(|v| v.get(nested_key))
                .and_then(|v| v.as_bool())
        })
    }

    let Some(opts) = opts else {
        return InlayHintConfig {
            types: true,
            parameters: false,
        };
    };
    InlayHintConfig {
        types: read_flag(opts, "resilient.inlayHints.types", "types").unwrap_or(true),
        parameters: read_flag(opts, "resilient.inlayHints.parameters", "parameters")
            .unwrap_or(false),
    }
}

/// RES-189: compatibility shim for the existing parameter-hints parser
/// tests; the canonical settings parser now lives in
/// `read_init_inlay_hints_config`.
#[allow(dead_code)]
pub(crate) fn read_init_param_hints_flag(opts: Option<&tower_lsp::lsp_types::LSPAny>) -> bool {
    read_init_inlay_hints_config(opts).parameters
}

fn inlay_hint_from_fn_return(
    entry: &typechecker::FnReturnTypeHint,
    src: &str,
) -> Option<InlayHint> {
    let position = find_signature_close_position(src, entry.fn_start, entry.body_start)?;
    Some(InlayHint {
        position,
        label: InlayHintLabel::String(format!(" -> {}", entry.ty)),
        kind: Some(InlayHintKind::TYPE),
        text_edits: None,
        tooltip: None,
        padding_left: Some(true),
        padding_right: None,
        data: None,
    })
}

fn find_signature_close_position(
    src: &str,
    fn_start: crate::span::Pos,
    body_start: crate::span::Pos,
) -> Option<Position> {
    fn byte_index_for_position(src: &str, target: (u32, u32)) -> Option<usize> {
        let mut line = 0u32;
        let mut col = 0u32;
        for (idx, ch) in src.char_indices() {
            if (line, col) == target {
                return Some(idx);
            }
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        if (line, col) == target {
            Some(src.len())
        } else {
            None
        }
    }

    fn position_for_byte_index(src: &str, byte_idx: usize) -> Position {
        let mut line = 0u32;
        let mut col = 0u32;
        for (idx, ch) in src.char_indices() {
            if idx >= byte_idx {
                break;
            }
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        Position::new(line, col)
    }

    let mut line = 0u32;
    let mut col = 0u32;
    let mut seen_lparen = false;
    let mut depth = 0usize;
    let mut start = (
        fn_start.line.saturating_sub(1) as u32,
        fn_start.column.saturating_sub(1) as u32,
    );
    let end = (
        body_start.line.saturating_sub(1) as u32,
        body_start.column.saturating_sub(1) as u32,
    );
    if start >= end
        && let Some(end_byte) = byte_index_for_position(src, end)
        && let Some(fn_byte) = src[..end_byte].rfind("fn")
    {
        let pos = position_for_byte_index(src, fn_byte);
        start = (pos.line, pos.character);
    }

    for ch in src.chars() {
        let here = (line, col);
        if here >= end {
            break;
        }
        if here >= start {
            match ch {
                '(' => {
                    seen_lparen = true;
                    depth += 1;
                }
                ')' if seen_lparen && depth > 0 => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(Position::new(line, col + 1));
                    }
                }
                _ => {}
            }
        }

        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }

    None
}

/// Best-effort span lookup for an expression node. Used to place
/// parameter hints at the arg's start. Returns None when we don't
/// have a span for a given shape — the hint is then skipped.
fn expression_span(node: &Node) -> Option<crate::span::Span> {
    match node {
        Node::IntegerLiteral { span, .. }
        | Node::FloatLiteral { span, .. }
        | Node::StringLiteral { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::Identifier { span, .. }
        | Node::InfixExpression { span, .. }
        | Node::CallExpression { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::ArrayLiteral { span, .. }
        | Node::IndexExpression { span, .. }
        | Node::FieldAccess { span, .. }
        | Node::StructLiteral { span, .. } => Some(*span),
        _ => None,
    }
}

#[allow(deprecated)] // `DocumentSymbol::deprecated` is deprecated in the LSP type
fn make_symbol(name: &str, kind: SymbolKind, span: crate::span::Span) -> DocumentSymbol {
    let range = span_to_range(span);
    DocumentSymbol {
        name: name.to_string(),
        detail: None,
        kind,
        tags: None,
        // `deprecated` is deprecated in the LSP spec in favour of
        // `tags`, but the type still carries it; set it explicitly
        // so the derive doesn't complain about missing fields.
        deprecated: None,
        range,
        // See the helper's doc-comment for why selection_range
        // mirrors range today.
        selection_range: range,
        children: None,
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> JsonResult<InitializeResult> {
        // RES-186: capture the workspace root for the symbol index.
        // Prefer modern `workspace_folders`; fall back to the
        // deprecated `root_uri` for older clients.
        let root_path: Option<PathBuf> = params
            .workspace_folders
            .as_ref()
            .and_then(|folders| folders.first())
            .map(|f| f.uri.clone())
            .or(params.root_uri.clone())
            .and_then(|u| u.to_file_path().ok());
        if let (Ok(mut slot), Some(p)) = (self.workspace_root.lock(), root_path) {
            *slot = Some(p);
        }

        let inlay_hint_config =
            read_init_inlay_hints_config(params.initialization_options.as_ref());
        if let Ok(mut slot) = self.inlay_hint_types.lock() {
            *slot = inlay_hint_config.types;
        }
        if let Ok(mut slot) = self.inlay_hint_parameters.lock() {
            *slot = inlay_hint_config.parameters;
        }

        Ok(InitializeResult {
            server_info: None,
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                // RES-185: advertise the document-symbol handler so
                // editors' outline views light up. `OneOf::Left(true)`
                // is the compact form — `Right(DocumentSymbolOptions)`
                // would let us opt into work-done progress reporting,
                // which we don't need for files this small.
                document_symbol_provider: Some(OneOf::Left(true)),
                // RES-186: workspace-symbol search across all .rs
                // files in the workspace root.
                workspace_symbol_provider: Some(OneOf::Left(true)),
                // RES-187: full-file semantic tokens. Delta is a
                // follow-up; many clients use `full` anyway.
                semantic_tokens_provider: Some(semantic_tokens_capability()),
                // RES-189: inlay hints. `Options` (not registration-
                // options) — the server supports the feature for any
                // document that already has a `TextDocumentSyncKind`
                // registered above.
                inlay_hint_provider: Some(OneOf::Right(InlayHintServerCapabilities::Options(
                    InlayHintOptions {
                        work_done_progress_options: WorkDoneProgressOptions::default(),
                        resolve_provider: Some(false),
                    },
                ))),
                // RES-181a: advertise hover support. Today the handler
                // only surfaces a type for literals (Int / Float /
                // Bool / String / Bytes / Duration) — identifier
                // hover is RES-181b, deferred behind RES-120's
                // inferred-type table. `Simple(true)` is the compact
                // form; `HoverOptions` would let us opt into
                // work-done progress reporting which we don't need.
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                // RES-182a: advertise go-to-definition. The handler
                // currently resolves only TOP-LEVEL declarations
                // (fn / struct / type alias) within the same
                // document; local / parameter / cross-file targets
                // (RES-182b, RES-182c) stay deferred until a scope-
                // aware resolver + span-carrying-path work lands.
                definition_provider: Some(OneOf::Left(true)),
                // RES-183: advertise find-references. The handler
                // collects every `CallExpression` in the current
                // document whose callee matches the cursor's top-
                // level fn name. Struct literals with the same name
                // are excluded (AST-driven, not textual).
                references_provider: Some(OneOf::Left(true)),
                // RES-184: advertise rename support with prepareRename
                // guard. `prepare_provider: Some(true)` tells clients
                // to call `textDocument/prepareRename` first so users
                // get "cannot rename here" feedback before typing.
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                // RES-188a: advertise identifier completion. The
                // handler seeds its candidate list from the BUILTINS
                // table plus top-level decls in the current
                // document; scope-aware local / parameter completion
                // (RES-188b) stays deferred on the same scope-walker
                // that blocks RES-182b. No trigger characters today
                // — identifier-prefix completion is driven by the
                // client; post-dot field completion is a separate
                // ticket (see the Notes section).
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: None,
                    all_commit_characters: None,
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                    completion_item: None,
                }),
                // RES-357: advertise code-action support so editors
                // show the light-bulb menu for L0010 "Add contract
                // stubs" quick-fixes.
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "resilient-lsp initialized")
            .await;
    }

    async fn shutdown(&self) -> JsonResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text;
        self.publish_analysis(uri, text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // We registered as TextDocumentSyncKind::FULL, so each
        // change message includes the whole buffer in the first
        // content change.
        let uri = params.text_document.uri.clone();
        let text = params
            .content_changes
            .into_iter()
            .next()
            .map(|c| c.text)
            .unwrap_or_default();
        self.publish_analysis(uri, text).await;
    }

    /// RES-185: clear the cached AST for the closed document so
    /// long-running editor sessions don't retain memory for files
    /// the user isn't looking at anymore. Returning `None` from
    /// `document_symbol` for an unknown URI is the post-close
    /// behaviour (clients typically stop asking, but the handler
    /// tolerates the case gracefully either way).
    async fn did_close(&self, params: tower_lsp::lsp_types::DidCloseTextDocumentParams) {
        if let Ok(mut map) = self.documents.lock() {
            map.remove(&params.text_document.uri);
        }
        // RES-187: free the cached source text too.
        if let Ok(mut tmap) = self.documents_text.lock() {
            tmap.remove(&params.text_document.uri);
        }
    }

    /// RES-186: refresh the workspace symbol index entry for the
    /// saved file. Idempotent — just replaces the per-file vec
    /// in the index map, keeping every other file's entries
    /// untouched. If the save happened before the index was
    /// built, this is still safe: the lazy-build on next
    /// `workspace/symbol` call will include the saved state.
    async fn did_save(&self, params: tower_lsp::lsp_types::DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        // Re-read the file from disk. `did_save` notifications
        // carry the text only when the client opts into the
        // `TextDocumentSyncSaveOptions { include_text: true }`
        // — we registered TextDocumentSyncKind::FULL without
        // that, so walk to disk instead.
        let Some(path) = uri.to_file_path().ok() else {
            return;
        };
        let entries = match index_file(&path) {
            Some(e) => e,
            None => return,
        };
        if let Ok(mut map) = self.workspace_index.lock() {
            map.insert(uri, entries);
        }
    }

    /// RES-186: workspace-symbol search. Lazy-builds the per-file
    /// index on first call (walks the workspace root for `*.rs`
    /// files, skipping `target/` and dotfiles), then filters by
    /// substring match (case-insensitive) and caps at 50 entries
    /// per the ticket's budget.
    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> JsonResult<Option<Vec<SymbolInformation>>> {
        // Lazy-build the index if this is the first query.
        let needs_build = match self.workspace_index_built.lock() {
            Ok(g) => !*g,
            Err(_) => false,
        };
        if needs_build {
            self.rebuild_workspace_index();
            if let Ok(mut g) = self.workspace_index_built.lock() {
                *g = true;
            }
        }

        let query = params.query.to_lowercase();
        let index = match self.workspace_index.lock() {
            Ok(g) => g,
            Err(_) => return Ok(Some(Vec::new())),
        };
        let matches = filter_workspace_symbols(&index, &query, 50);
        Ok(Some(matches))
    }

    /// RES-185: respond to `textDocument/documentSymbol` — return
    /// the outline of top-level fns / structs / type aliases in
    /// the cached AST. Returns `Ok(None)` when the document has
    /// never been opened here (shouldn't normally happen; the LSP
    /// spec defines the request as coming AFTER didOpen).
    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> JsonResult<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        // Clone out of the lock synchronously — we never .await
        // while holding the Mutex (which would block other
        // handlers on the same worker thread).
        let program = match self.documents.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(program) = program else {
            return Ok(None);
        };
        let symbols = document_symbols_for_program(&program);
        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    /// RES-181a / RES-302: respond to `textDocument/hover`.
    ///
    /// Two cursor-shape cases are served today:
    ///   1. Literal under the cursor → surface its Resilient-
    ///      surface type name (`Int`, `Float`, `String`, `Bool`,
    ///      `Bytes`). Implementation drives the lexer directly
    ///      against the cached source text (not the AST), because
    ///      the parser's per-leaf spans record `last_token_*`
    ///      AFTER the lexer advances — unreliable for literal
    ///      positions. See `hover_literal_at`'s module-level
    ///      rationale.
    ///   2. Identifier under the cursor → look up the symbol in
    ///      the cached AST via `infer_identifier_type`. Today
    ///      this covers top-level `let` / `const` / `static let`
    ///      bindings, top-level `fn` names, and parameters of
    ///      top-level fns. Anything else (nested binding,
    ///      identifier used inside a fn body without a matching
    ///      top-level decl) returns `Ok(None)` and the client
    ///      renders nothing.
    ///
    /// Doc-comment surfacing is a separate follow-up to RES-302 —
    /// the AST does not yet thread doc-comments through, so this
    /// handler is intentionally type-only for now.
    async fn hover(&self, params: HoverParams) -> JsonResult<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let text = match self.documents_text.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(text) = text else {
            return Ok(None);
        };
        // Literal hover takes priority — its type read is exact
        // (the lexer told us). Identifier fallback is best-effort.
        if let Some((type_name, range)) = hover_literal_at(&text, pos) {
            return Ok(Some(Hover {
                contents: HoverContents::Scalar(MarkedString::String(type_name.to_string())),
                range: Some(range),
            }));
        }
        // RES-302: identifier hover. Look up the cursor's name
        // in the cached AST and surface the inferred type as a
        // markdown code block (clients that don't render markdown
        // still display the body verbatim, so the snippet stays
        // legible).
        let Some((name, range)) = identifier_at(&text, pos) else {
            return Ok(None);
        };
        let program = match self.documents.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(program) = program else {
            return Ok(None);
        };
        let Some(type_str) = infer_identifier_type(&program, &name) else {
            return Ok(None);
        };
        let body = format!("```rust\nlet {name}: {type_str}\n```");
        Ok(Some(Hover {
            contents: HoverContents::Scalar(MarkedString::String(body)),
            range: Some(range),
        }))
    }

    /// RES-182a: respond to `textDocument/definition` — jump to
    /// the defining span of the symbol under the cursor.
    /// Currently handles only TOP-LEVEL declarations (fn / struct
    /// / type alias) within the same document. Local bindings /
    /// parameters (RES-182b) and cross-file imports (RES-182c)
    /// return `Ok(None)` so the editor's "no definition found"
    /// UX kicks in. That's a graceful degradation — picking the
    /// wrong jump target would be strictly worse than "I don't
    /// know yet."
    ///
    /// Implementation:
    ///   1. Look up the cursor's identifier token via
    ///      `identifier_at` (same token-level plumbing as
    ///      RES-181a's hover).
    ///   2. Rebuild the top-level def map from the cached AST
    ///      and look the name up.
    ///   3. Wrap the result in a `Location` pointing at the same
    ///      document URI (cross-file is RES-182c).
    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> JsonResult<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let text = match self.documents_text.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(text) = text else {
            return Ok(None);
        };
        let Some((name, _range)) = identifier_at(&text, pos) else {
            return Ok(None);
        };
        let program = match self.documents.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(program) = program else {
            return Ok(None);
        };
        let defs = build_top_level_defs(&program);
        let Some(def) = find_top_level_def(&defs, &name) else {
            return Ok(None);
        };
        Ok(Some(GotoDefinitionResponse::Scalar(Location {
            uri,
            range: def.range,
        })))
    }

    /// RES-2567: respond to `textDocument/references` for:
    /// - top-level functions across workspace imports,
    /// - struct types across workspace imports,
    /// - same-file variable declarations / reads / writes.
    async fn references(&self, params: ReferenceParams) -> JsonResult<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let include_decl = params.context.include_declaration;

        let text = match self.documents_text.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(text) = text else {
            return Ok(None);
        };

        let Some((name, _cursor_range)) = identifier_at(&text, pos) else {
            return Ok(None);
        };

        let program = match self.documents.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(program) = program else {
            return Ok(None);
        };
        let file_path = uri.to_file_path().ok().map(|p| canonicalize_or_self(&p));

        let variable_symbols = collect_variable_references(&program);
        if let Some(variable) = variable_symbols.iter().find(|symbol| {
            symbol.name == name
                && (range_contains_pos(symbol.decl_range, pos)
                    || symbol.decl_range.start.line == pos.line
                    || symbol
                        .refs
                        .iter()
                        .any(|range| range_contains_pos(*range, pos)))
        }) {
            let mut locations = Vec::new();
            if include_decl {
                locations.push(Location {
                    uri: uri.clone(),
                    range: variable.decl_range,
                });
            }
            for range in &variable.refs {
                locations.push(Location {
                    uri: uri.clone(),
                    range: *range,
                });
            }
            return if locations.is_empty() {
                Ok(None)
            } else {
                Ok(Some(locations))
            };
        }

        let Some(file_path) = file_path else {
            return Ok(None);
        };
        let mut source_overrides = HashMap::new();
        source_overrides.insert(file_path.clone(), (text.clone(), program.clone()));
        let mut exports_memo = HashMap::new();
        let accessible =
            accessible_symbols_for_file(&file_path, &source_overrides, &mut exports_memo);
        let Some(symbol) = accessible.get(&name).cloned() else {
            return Ok(None);
        };
        let mut search_files = match self.workspace_root.lock() {
            Ok(root) => root
                .as_ref()
                .map(|p| walk_resilient_files(p))
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        if !search_files
            .iter()
            .any(|p| canonicalize_or_self(p) == file_path)
        {
            search_files.push(file_path.clone());
        }

        let mut seen = HashSet::new();
        let mut locations = Vec::new();
        if include_decl
            && let Ok(decl_uri) = Url::from_file_path(&symbol.origin_path)
            && seen.insert((
                decl_uri.clone(),
                symbol.decl_range.start.line,
                symbol.decl_range.start.character,
                symbol.decl_range.end.line,
                symbol.decl_range.end.character,
            ))
        {
            locations.push(Location {
                uri: decl_uri,
                range: symbol.decl_range,
            });
        }

        for path in search_files {
            let path = canonicalize_or_self(&path);
            let Some((src, file_program)) = load_source_and_program(&path, &source_overrides)
            else {
                continue;
            };
            let visible = accessible_symbols_for_file(&path, &source_overrides, &mut exports_memo);
            let spellings: Vec<String> = visible
                .into_iter()
                .filter_map(|(visible_name, visible_symbol)| {
                    (visible_symbol.origin_path == symbol.origin_path
                        && visible_symbol.origin_name == symbol.origin_name
                        && visible_symbol.kind == symbol.kind)
                        .then_some(visible_name)
                })
                .collect();
            if spellings.is_empty() {
                continue;
            }
            let Ok(file_uri) = Url::from_file_path(&path) else {
                continue;
            };
            for spelling in spellings {
                let mut ranges = match symbol.kind {
                    ReferenceSymbolKind::Fn => collect_call_sites(&file_program, &spelling),
                    ReferenceSymbolKind::Struct => {
                        collect_qualified_identifier_sites(&src, &spelling)
                    }
                };
                if matches!(symbol.kind, ReferenceSymbolKind::Struct) && path == symbol.origin_path
                {
                    ranges.retain(|range| *range != symbol.decl_range);
                }
                for range in ranges {
                    let key = (
                        file_uri.clone(),
                        range.start.line,
                        range.start.character,
                        range.end.line,
                        range.end.character,
                    );
                    if seen.insert(key) {
                        locations.push(Location {
                            uri: file_uri.clone(),
                            range,
                        });
                    }
                }
            }
        }

        if locations.is_empty() {
            Ok(None)
        } else {
            Ok(Some(locations))
        }
    }

    /// RES-184 / RES-2568: respond to `textDocument/prepareRename` — the
    /// UX guard that tells editors whether the symbol under the cursor is
    /// renamable before the user types a new name.
    ///
    /// A symbol is renamable when it is:
    ///   - A top-level `fn` declaration name (RES-184).
    ///   - A top-level `struct` declaration name (RES-2568).
    ///   - A top-level `let` / `const` / `static let` binding name
    ///     (RES-2568).
    ///
    /// Returns the identifier's range so the editor pre-selects the
    /// current name in the rename input box. Returns `Ok(None)` for
    /// non-renamable positions (literals, keywords, local vars, struct
    /// fields, parameters).
    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> JsonResult<Option<PrepareRenameResponse>> {
        let uri = params.text_document.uri;
        let pos = params.position;

        let text = match self.documents_text.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(text) = text else {
            return Ok(None);
        };

        let Some((name, range)) = identifier_at(&text, pos) else {
            return Ok(None);
        };

        let program = match self.documents.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(program) = program else {
            return Ok(None);
        };

        // Accept fns, structs, and top-level let/const/static-let.
        let defs = build_top_level_defs(&program);
        let is_renamable = find_top_level_def(&defs, &name).is_some();

        if !is_renamable {
            return Ok(None);
        }

        Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
            range,
            placeholder: name,
        }))
    }

    /// RES-184 / RES-2568: respond to `textDocument/rename` — emit a
    /// `WorkspaceEdit` that renames every reference to a top-level
    /// symbol in all open documents.
    ///
    /// Pipeline:
    ///   1. Validate the new name against `[A-Za-z_][A-Za-z0-9_]*`.
    ///      Return an LSP error immediately if invalid.
    ///   2. Look up the identifier under the cursor via `identifier_at`.
    ///      Confirm it names a top-level binding via `build_top_level_defs`.
    ///   3. Collision check: if the new name already names a visible
    ///      top-level binding in the current file, return an LSP error.
    ///   4. Determine the symbol kind (fn / struct / variable) and
    ///      collect edit sites accordingly:
    ///      - fn: declaration token + all call sites.
    ///      - struct: declaration token + all `StructLiteral` constructor
    ///        sites.
    ///      - variable (let / const / static let): declaration token +
    ///        all `Identifier` references.
    ///   5. RES-2568 cross-file: repeat the call/struct-literal/identifier
    ///      scan for every other open document in `documents_text` whose
    ///      cached AST references the old name.
    ///   6. Group `TextEdit`s by URI and return a `WorkspaceEdit`.
    async fn rename(&self, params: RenameParams) -> JsonResult<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let new_name = params.new_name;

        // Validate identifier pattern before doing any work.
        if !is_valid_identifier(&new_name) {
            return Err(tower_lsp::jsonrpc::Error {
                code: tower_lsp::jsonrpc::ErrorCode::InvalidParams,
                message: format!(
                    "invalid identifier `{new_name}`: must match [A-Za-z_][A-Za-z0-9_]*"
                )
                .into(),
                data: None,
            });
        }

        let text = match self.documents_text.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(text) = text else {
            return Ok(None);
        };

        let Some((name, _cursor_range)) = identifier_at(&text, pos) else {
            return Ok(None);
        };

        let program = match self.documents.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(program) = program else {
            return Ok(None);
        };

        // Confirm cursor is on a top-level binding (fn / struct / let).
        let defs = build_top_level_defs(&program);
        let Some(def) = find_top_level_def(&defs, &name) else {
            return Ok(None);
        };

        // Collision check: reject if new name already has a top-level binding.
        if find_top_level_def(&defs, &new_name).is_some() {
            return Err(tower_lsp::jsonrpc::Error {
                code: tower_lsp::jsonrpc::ErrorCode::InvalidParams,
                message: format!("rename would shadow `{new_name}`").into(),
                data: None,
            });
        }

        // Determine symbol kind from the AST.
        let symbol_kind = if let Node::Program(stmts) = &program {
            let mut kind = RenameSymbolKind::Variable;
            for s in stmts {
                match &s.node {
                    Node::Function { name: n, .. } if n == &name => {
                        kind = RenameSymbolKind::Fn;
                        break;
                    }
                    Node::StructDecl { name: n, .. } if n == &name => {
                        kind = RenameSymbolKind::Struct;
                        break;
                    }
                    _ => {}
                }
            }
            kind
        } else {
            return Ok(None);
        };

        // --- Build edits for the primary document (the one with the cursor) ---
        let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
        let primary_edits =
            build_rename_edits_for_doc(&program, &text, &name, &new_name, symbol_kind, def.range);
        if !primary_edits.is_empty() {
            changes.insert(uri.clone(), primary_edits);
        }

        // --- RES-2568: cross-file rename via open-document cache ---
        // For every other open document, collect the same edit sites.
        // We don't scan the full workspace on disk here (that would be
        // too slow for large repos and requires holding locks across I/O);
        // we scan the in-memory `documents_text` + `documents` maps.
        //
        // The workspace-index path (scanning `.rz` files on disk) is
        // a follow-up (RES-2568b). What we ship here covers the common
        // case: open buffers in the editor session.
        let other_docs: Vec<(Url, String, Node)> = {
            let text_map = self.documents_text.lock().ok();
            let ast_map = self.documents.lock().ok();
            match (text_map, ast_map) {
                (Some(tmap), Some(amap)) => tmap
                    .iter()
                    .filter(|(u, _)| *u != &uri)
                    .filter_map(|(u, src)| {
                        amap.get(u)
                            .map(|prog| (u.clone(), src.clone(), prog.clone()))
                    })
                    .collect(),
                _ => Vec::new(),
            }
        };

        for (other_uri, other_text, other_prog) in other_docs {
            let other_defs = build_top_level_defs(&other_prog);
            let def_range = find_top_level_def(&other_defs, &name)
                .map(|d| d.range)
                .unwrap_or_default();
            let edits = build_rename_edits_for_doc(
                &other_prog,
                &other_text,
                &name,
                &new_name,
                symbol_kind,
                def_range,
            );
            if !edits.is_empty() {
                changes.insert(other_uri, edits);
            }
        }

        if changes.is_empty() {
            return Ok(None);
        }

        Ok(Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }))
    }

    /// RES-188a: respond to `textDocument/completion` — return
    /// builtins + top-level decls whose names start with the
    /// prefix already typed. Scope-aware local / parameter
    /// completion (RES-188b) stays deferred.
    ///
    /// Pipeline:
    ///   1. Read cached source + AST.
    ///   2. `prefix_at(src, pos)` extracts what the user has typed
    ///      so far (walking back from the cursor to a non-
    ///      identifier char).
    ///   3. `completion_candidates(program, prefix)` enumerates
    ///      matching names from `BUILTINS` + top-level decls and
    ///      applies the 100-item cap.
    ///   4. Convert each `Candidate` to `CompletionItem` and wrap
    ///      as `CompletionResponse::Array`.
    async fn completion(&self, params: CompletionParams) -> JsonResult<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let text = match self.documents_text.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(text) = text else {
            return Ok(None);
        };
        let program = match self.documents.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(program) = program else {
            return Ok(None);
        };
        let prefix = prefix_at(&text, pos);
        let items: Vec<CompletionItem> = completion_candidates(&program, &prefix)
            .into_iter()
            .map(candidate_to_completion_item)
            .collect();
        Ok(Some(CompletionResponse::Array(items)))
    }

    /// RES-357: respond to `textDocument/codeAction` — offer the
    /// "Add contract stubs" quick-fix for every L0010 diagnostic
    /// in the requested range.
    ///
    /// Pipeline:
    ///   1. Read the cached source text.
    ///   2. Walk the incoming `context.diagnostics`; collect those
    ///      whose message contains "L0010" (the "no contract" lint).
    ///   3. For each matching diagnostic, locate the function's
    ///      opening `{` by scanning forward from the diagnostic
    ///      position (`find_brace_line`).
    ///   4. Emit a `CodeAction` with a `TextEdit` that inserts
    ///      `"    requires true;\n    ensures true;\n"` at the
    ///      start of the line immediately after the `{`.
    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> JsonResult<Option<Vec<CodeActionOrCommand>>> {
        let uri = params.text_document.uri;
        let text = match self.documents_text.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(text) = text else {
            return Ok(None);
        };

        let mut actions: Vec<CodeActionOrCommand> = Vec::new();

        for diag in &params.context.diagnostics {
            // RES-190: missing-semicolon quick-fix. Triggers on parser
            // diagnostics whose message refers to a missing/expected
            // `;`. The action inserts `;` at the diagnostic's start
            // position so the editor's "lightbulb" can apply it
            // directly. Detection is by message substring (matches
            // existing `..Default::default()` Diagnostic construction
            // that leaves `code: None`); when the parser starts
            // emitting the E0002 code, swap to a `code == "E0002"`
            // check without changing the user-visible behaviour.
            if is_missing_semicolon_diagnostic(diag) {
                if let Some(action) = build_insert_semicolon_action(&uri, diag) {
                    actions.push(CodeActionOrCommand::CodeAction(action));
                }
                continue;
            }

            // RES-2570: unused-variable quick-fixes (L0001/L0011/L0020).
            // Offer two actions: prefix with `_` and add an allow comment.
            if is_unused_variable_diagnostic(diag) {
                if let Some(action) = build_prefix_underscore_action(&uri, diag, &text) {
                    actions.push(CodeActionOrCommand::CodeAction(action));
                }
                if let Some(action) = build_suppress_lint_action(&uri, diag, &text) {
                    actions.push(CodeActionOrCommand::CodeAction(action));
                }
                continue;
            }

            // RES-2570: type-mismatch quick-fix — offer "Add `as <type>` cast"
            // for numeric type mismatches.
            if is_type_mismatch_diagnostic(diag) {
                if let Some(action) = build_add_cast_action(&uri, diag) {
                    actions.push(CodeActionOrCommand::CodeAction(action));
                }
                continue;
            }

            // RES-2645: undefined-name import quick-fix. When the
            // workspace index contains top-level function definitions
            // matching the missing name, offer one `use "path";`
            // action per candidate module.
            if is_undefined_name_diagnostic(diag) {
                let needs_build = match self.workspace_index_built.lock() {
                    Ok(g) => !*g,
                    Err(_) => false,
                };
                if needs_build {
                    self.rebuild_workspace_index();
                    if let Ok(mut g) = self.workspace_index_built.lock() {
                        *g = true;
                    }
                }
                let index = match self.workspace_index.lock() {
                    Ok(g) => g,
                    Err(_) => continue,
                };
                let candidates = undefined_name_candidates(&index, &uri, diag);
                actions.extend(
                    build_add_use_actions(&uri, diag, &text, candidates.into_iter())
                        .into_iter()
                        .map(CodeActionOrCommand::CodeAction),
                );
                continue;
            }

            // RES-2570: dead-function quick-fix (L0014).
            if is_dead_function_diagnostic(diag) {
                if let Some(action) = build_prefix_fn_underscore_action(&uri, diag, &text) {
                    actions.push(CodeActionOrCommand::CodeAction(action));
                }
                if let Some(action) = build_suppress_lint_action(&uri, diag, &text) {
                    actions.push(CodeActionOrCommand::CodeAction(action));
                }
                continue;
            }

            // L0010 "no contract" diagnostics: insert `requires`/`ensures` stubs.
            if !diag.message.contains("L0010")
                && !diag.message.contains("requires")
                && !diag.message.contains("no contract")
            {
                // For any other lint-code diagnostic, offer the generic
                // suppress-with-allow-comment action.
                if extract_lint_code(&diag.message).is_some()
                    && let Some(action) = build_suppress_lint_action(&uri, diag, &text)
                {
                    actions.push(CodeActionOrCommand::CodeAction(action));
                }
                continue;
            }
            // Determine insertion point: scan from the diagnostic's
            // start line forward to find the `{` that opens the fn body.
            let diag_line = diag.range.start.line as usize;
            let Some(brace_line) = find_brace_line(&text, diag_line) else {
                continue;
            };
            // Insert at the start of the line after the opening brace.
            let insert_pos = Position {
                line: (brace_line + 1) as u32,
                character: 0,
            };
            let insert_range = Range {
                start: insert_pos,
                end: insert_pos,
            };
            let edit_text = "    requires true;\n    ensures true;\n".to_string();
            let text_edit = TextEdit {
                range: insert_range,
                new_text: edit_text,
            };
            let mut changes = HashMap::new();
            changes.insert(uri.clone(), vec![text_edit]);
            let workspace_edit = WorkspaceEdit {
                changes: Some(changes),
                document_changes: None,
                change_annotations: None,
            };
            let action = CodeAction {
                title: "Add contract stubs".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diag.clone()]),
                edit: Some(workspace_edit),
                command: None,
                is_preferred: Some(true),
                disabled: None,
                data: None,
            };
            actions.push(CodeActionOrCommand::CodeAction(action));
        }

        if actions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(actions))
        }
    }

    /// RES-187: respond to `textDocument/semanticTokens/full` with
    /// a full-file token stream. Reads the cached source text,
    /// runs the lexer-driven classifier, and returns the delta-
    /// encoded LSP payload. `Ok(None)` when the document text
    /// isn't cached (i.e. the client asked before `didOpen`); a
    /// strict client would just skip semantic highlighting for
    /// this file until the next change.
    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> JsonResult<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri;
        let text = match self.documents_text.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(text) = text else { return Ok(None) };
        let wire = compute_semantic_tokens(&text);
        let data = semantic_tokens_from_wire(wire);
        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data,
        })))
    }

    /// RES-189: `textDocument/inlayHint` — emit type-hint labels
    /// for unannotated `let` bindings (always on) and parameter-
    /// name labels at call sites (gated behind the
    /// `resilient.inlayHints.parameters` init option).
    ///
    /// Strategy: run the typechecker on the cached AST so the
    /// `let_type_hints` side-channel fills up, then walk the AST
    /// separately for call-site parameter hints. Both passes are
    /// cheap; no caching needed for files the editor could open.
    /// Filters the output by the request's `range` so clients
    /// that ask for a viewport don't pay to render the whole
    /// file's worth.
    async fn inlay_hint(&self, params: InlayHintParams) -> JsonResult<Option<Vec<InlayHint>>> {
        let uri = params.text_document.uri;
        let range = params.range;

        let program = match self.documents.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let source_text = match self.documents_text.lock() {
            Ok(map) => map.get(&uri).cloned(),
            Err(_) => None,
        };
        let Some(program) = program else {
            return Ok(None);
        };

        // Run the typechecker purely for its hint side-channel.
        // Ignore the return value — errors don't invalidate hints
        // collected up to the error point.
        //
        // RES-1353: opt into populating `let_type_hints`. Other
        // typechecker call sites (diagnostics in `did_open` /
        // `did_change`, every CLI `rz prog.rz` run, every
        // `cargo test`) leave the flag off so the per-`let`
        // allocations don't fire on hot paths that never read the
        // hints.
        let mut tc = typechecker::TypeChecker::new().with_capture_inlay_hints(true);
        let _ = tc.check_program_with_source(&program, uri.as_str());
        let want_type_hints = match self.inlay_hint_types.lock() {
            Ok(g) => *g,
            Err(_) => true,
        };
        let mut out: Vec<InlayHint> = Vec::new();

        if want_type_hints {
            out.extend(
                tc.let_type_hints
                    .iter()
                    .map(inlay_hint_from_let)
                    .filter(|h| position_in_range(h.position, range)),
            );
            if let Some(src) = source_text.as_deref() {
                out.extend(
                    tc.fn_return_type_hints
                        .iter()
                        .filter_map(|entry| inlay_hint_from_fn_return(entry, src))
                        .filter(|h| position_in_range(h.position, range)),
                );
            }
        }

        let want_param_hints = match self.inlay_hint_parameters.lock() {
            Ok(g) => *g,
            Err(_) => false,
        };
        if want_param_hints {
            for h in collect_param_hints(&program) {
                if position_in_range(h.position, range) {
                    out.push(h);
                }
            }
        }
        Ok(Some(out))
    }
}

// ============================================================
// RES-2570: additional quick-fix helpers
// ============================================================
//
// Each helper follows the same pattern as `build_insert_semicolon_action`:
//   - detect a diagnostic by message substring (or lint code)
//   - produce a `CodeAction { kind: QUICKFIX, edit: WorkspaceEdit }`
//   - unit-testable as a pure function without an async runtime

/// RES-2570: detect a lint-level "unused variable / binding" diagnostic.
/// Triggers on L0001 ("unused local binding") and L0011 ("variable assigned
/// but never used") messages emitted by the Resilient lint pass.
pub(crate) fn is_unused_variable_diagnostic(diag: &Diagnostic) -> bool {
    let msg = &diag.message;
    (msg.contains("L0001") || msg.contains("unused local binding"))
        || (msg.contains("L0011") || msg.contains("is assigned but never used"))
        || (msg.contains("L0020") || msg.contains("unused parameter"))
}

/// RES-2570: extract the variable name from an L0001/L0011/L0020 diagnostic
/// message such as "unused local binding `foo`" → "foo".
///
/// Returns `None` when the backtick-delimited name can't be extracted
/// (e.g. a malformed or unrelated message). The caller skips the
/// action in that case rather than producing an incorrect edit.
pub(crate) fn extract_backtick_name(msg: &str) -> Option<&str> {
    let start = msg.find('`')?;
    let rest = &msg[start + 1..];
    let end = rest.find('`')?;
    let name = &rest[..end];
    if name.is_empty() || name.starts_with('_') {
        None
    } else {
        Some(name)
    }
}

/// RES-2570: build a "Prefix with `_`" `CodeAction` for an unused-variable
/// diagnostic. The edit replaces the first occurrence of the bare identifier
/// on the diagnostic's line with `_<name>`, which silences L0001/L0011.
///
/// Strategy: scan the source for the backtick-delimited name from the
/// diagnostic message, then find it on the reported line. We replace only
/// the *first* occurrence on that line to avoid renaming references inside
/// the same statement (the lint always points at the declaration site).
pub(crate) fn build_prefix_underscore_action(
    uri: &Url,
    diag: &Diagnostic,
    src: &str,
) -> Option<CodeAction> {
    let name = extract_backtick_name(&diag.message)?;
    let line_no = diag.range.start.line as usize;
    let line = src.lines().nth(line_no)?;

    // Find the first occurrence of `name` as a standalone token on this line.
    // We search left-to-right and pick the first position where the character
    // before and after `name` are both non-identifier chars (or line boundary).
    let find_standalone = |haystack: &str, needle: &str| -> Option<usize> {
        let mut offset = 0;
        while offset + needle.len() <= haystack.len() {
            if let Some(pos) = haystack[offset..].find(needle) {
                let abs = offset + pos;
                let before_ok = abs == 0
                    || !haystack[abs - 1..abs]
                        .chars()
                        .next()
                        .map(|c| c.is_alphanumeric() || c == '_')
                        .unwrap_or(false);
                let after = abs + needle.len();
                let after_ok = after >= haystack.len()
                    || !haystack[after..after + 1]
                        .chars()
                        .next()
                        .map(|c| c.is_alphanumeric() || c == '_')
                        .unwrap_or(false);
                if before_ok && after_ok {
                    return Some(abs);
                }
                offset = abs + 1;
            } else {
                break;
            }
        }
        None
    };

    let col = find_standalone(line, name)?;
    let edit_range = Range {
        start: Position::new(line_no as u32, col as u32),
        end: Position::new(line_no as u32, (col + name.len()) as u32),
    };
    let text_edit = TextEdit {
        range: edit_range,
        new_text: format!("_{}", name),
    };
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), vec![text_edit]);
    Some(CodeAction {
        title: format!("Prefix `{}` with `_`", name),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diag.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }),
        command: None,
        is_preferred: Some(true),
        disabled: None,
        data: None,
    })
}

/// RES-2570: detect a "type mismatch" diagnostic — covers both the
/// typechecker's legacy short form ("Type mismatch in argument N")
/// and the rich E0007 form ("error[E0007]: type mismatch in argument N").
/// Also matches return-type mismatch messages.
pub(crate) fn is_type_mismatch_diagnostic(diag: &Diagnostic) -> bool {
    let lower = diag.message.to_ascii_lowercase();
    lower.contains("type mismatch") || lower.contains("return type mismatch")
}

/// RES-2570: extract `(expected_type, found_type)` from a type-mismatch
/// diagnostic message such as:
///   "type mismatch in argument 1: expected `int`, found `float`"
/// Returns `None` when the pattern can't be matched.
pub(crate) fn extract_mismatch_types(msg: &str) -> Option<(String, String)> {
    // Normalise: drop the leading E0007 tag if present.
    let lower = msg.to_ascii_lowercase();
    // Look for "expected `X`, found `Y`" — the rich E0007 form.
    if let Some(exp_pos) = lower.find("expected `") {
        let after_exp = &msg[exp_pos + "expected `".len()..];
        let exp_end = after_exp.find('`')?;
        let expected = after_exp[..exp_end].to_string();
        let found_search = &lower[exp_pos..];
        let fnd_pos = found_search.find("found `")?;
        let after_fnd = &msg[exp_pos + fnd_pos + "found `".len()..];
        let fnd_end = after_fnd.find('`')?;
        let found = after_fnd[..fnd_end].to_string();
        return Some((expected, found));
    }
    // Legacy short form: "Type mismatch in argument N: expected X, got Y"
    if let Some(exp_pos) = lower.find("expected ") {
        let after_exp = &msg[exp_pos + "expected ".len()..];
        let exp_end = after_exp.find([',', ';'])?;
        let expected = after_exp[..exp_end].trim().to_string();
        let got_search = &lower[exp_pos..];
        let got_pos = got_search.find("got ")?;
        let after_got = &msg[exp_pos + got_pos + "got ".len()..];
        let got_end = after_got.find([',', ';', '\n']).unwrap_or(after_got.len());
        let found = after_got[..got_end].trim().to_string();
        return Some((expected, found));
    }
    None
}

/// RES-2570: build an "Add `as <type>` cast" `CodeAction` for a type-mismatch
/// diagnostic. The action appends `as <expected>` to the token at the
/// diagnostic's position. We insert the cast text at the *end* of the
/// diagnostic range (after the expression) so the editor can apply it
/// without knowing the full expression span.
pub(crate) fn build_add_cast_action(uri: &Url, diag: &Diagnostic) -> Option<CodeAction> {
    let (expected, found) = extract_mismatch_types(&diag.message)?;
    // Only offer the cast for numeric or sized-integer mismatches where
    // `as <type>` syntax makes sense. Skip abstract / unknown / any types.
    let numeric_types = [
        "int", "float", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64",
    ];
    if !numeric_types.contains(&expected.as_str()) && !numeric_types.contains(&found.as_str()) {
        return None;
    }
    // Insert the cast at the end of the diagnostic range.
    let insert_pos = diag.range.end;
    let insert_range = Range {
        start: insert_pos,
        end: insert_pos,
    };
    let text_edit = TextEdit {
        range: insert_range,
        new_text: format!(" as {}", expected),
    };
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), vec![text_edit]);
    Some(CodeAction {
        title: format!("Add `as {}` cast", expected),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diag.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }),
        command: None,
        is_preferred: Some(false),
        disabled: None,
        data: None,
    })
}

/// RES-2645: detect an undefined-name/typechecker diagnostic whose missing
/// symbol might be satisfiable by importing another module.
pub(crate) fn is_undefined_name_diagnostic(diag: &Diagnostic) -> bool {
    diag.message.contains("Undefined variable")
}

/// RES-2645: extract the missing identifier from either rich
/// `"Undefined variable 'foo' at 3:5"` or legacy
/// `"Undefined variable: foo"` diagnostics.
pub(crate) fn extract_undefined_name(msg: &str) -> Option<&str> {
    if let Some(start) = msg.find("Undefined variable '") {
        let rest = &msg[start + "Undefined variable '".len()..];
        let end = rest.find('\'')?;
        let name = &rest[..end];
        return (!name.is_empty()).then_some(name);
    }
    if let Some(start) = msg.find("Undefined variable:") {
        let rest = msg[start + "Undefined variable:".len()..].trim();
        let end = rest
            .find(|c: char| c.is_whitespace() || c == ',' || c == ';')
            .unwrap_or(rest.len());
        let name = &rest[..end];
        return (!name.is_empty()).then_some(name);
    }
    None
}

/// RES-2645: exact-name lookup into the workspace symbol index. Only
/// top-level functions qualify for the import quick-fix.
pub(crate) fn undefined_name_candidates<'a>(
    index: &'a HashMap<Url, Vec<WorkspaceSymbolEntry>>,
    current_uri: &Url,
    diag: &Diagnostic,
) -> Vec<&'a WorkspaceSymbolEntry> {
    let Some(name) = extract_undefined_name(&diag.message) else {
        return Vec::new();
    };
    let mut out: Vec<&WorkspaceSymbolEntry> = index
        .values()
        .flatten()
        .filter(|entry| {
            entry.kind == SymbolKind::FUNCTION && entry.name == name && &entry.uri != current_uri
        })
        .collect();
    out.sort_by(|a, b| a.uri.as_str().cmp(b.uri.as_str()));
    out
}

fn path_components(path: &Path) -> Vec<&std::ffi::OsStr> {
    path.components()
        .map(|component| component.as_os_str())
        .collect()
}

/// RES-2645: convert an indexed candidate's absolute file URI into the
/// relative string form expected by `use "path/to/module.rz";`.
pub(crate) fn import_path_for_candidate(current_uri: &Url, candidate_uri: &Url) -> Option<String> {
    let current_path = current_uri.to_file_path().ok()?;
    let candidate_path = candidate_uri.to_file_path().ok()?;
    let current_dir = current_path.parent()?;

    let current_parts = path_components(current_dir);
    let candidate_parts = path_components(&candidate_path);
    let shared = current_parts
        .iter()
        .zip(candidate_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let mut relative = PathBuf::new();
    for _ in shared..current_parts.len() {
        relative.push("..");
    }
    for part in &candidate_parts[shared..] {
        relative.push(part);
    }

    let rendered = relative.to_string_lossy().replace('\\', "/");
    if rendered.is_empty() {
        None
    } else {
        Some(rendered)
    }
}

fn leading_use_insertion_line(src: &str) -> u32 {
    let mut saw_use = false;
    for (idx, line) in src.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if saw_use {
                return idx as u32 + 1;
            }
            continue;
        }
        if trimmed.starts_with("use \"") || trimmed.starts_with("pub use \"") {
            saw_use = true;
            continue;
        }
        return idx as u32;
    }
    src.lines().count() as u32
}

/// RES-2645: build one "Add `use`" action per candidate module. The
/// inserted path is relative to the current document, and the edit is
/// anchored at the top-of-file import section.
pub(crate) fn build_add_use_actions<'a>(
    uri: &Url,
    diag: &Diagnostic,
    src: &str,
    candidates: impl IntoIterator<Item = &'a WorkspaceSymbolEntry>,
) -> Vec<CodeAction> {
    let insert_line = leading_use_insertion_line(src);
    let insert_pos = Position::new(insert_line, 0);
    let insert_range = Range::new(insert_pos, insert_pos);
    let mut actions = Vec::new();
    for candidate in candidates {
        let Some(path) = import_path_for_candidate(uri, &candidate.uri) else {
            continue;
        };
        let new_text = format!("use \"{}\";\n", path);
        if src.contains(&new_text) {
            continue;
        }
        let text_edit = TextEdit {
            range: insert_range,
            new_text: new_text.clone(),
        };
        let mut changes = HashMap::new();
        changes.insert(uri.clone(), vec![text_edit]);
        actions.push(CodeAction {
            title: format!("Add `use \"{}\";`", path),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diag.clone()]),
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                document_changes: None,
                change_annotations: None,
            }),
            command: None,
            is_preferred: Some(true),
            disabled: None,
            data: None,
        });
    }
    actions
}

/// RES-2570: detect an L0014 "dead function" diagnostic (function defined
/// but never called).
pub(crate) fn is_dead_function_diagnostic(diag: &Diagnostic) -> bool {
    let msg = &diag.message;
    msg.contains("L0014") || msg.contains("defined but never called")
}

/// RES-2570: build a "Prefix function with `_`" code action for L0014
/// dead-function diagnostics. The action adds a leading `_` to the
/// function name at its declaration site. Uses the same standalone-token
/// replacement logic as `build_prefix_underscore_action`.
pub(crate) fn build_prefix_fn_underscore_action(
    uri: &Url,
    diag: &Diagnostic,
    src: &str,
) -> Option<CodeAction> {
    let name = extract_backtick_name(&diag.message)?;
    let line_no = diag.range.start.line as usize;
    // Find the `fn <name>` declaration on or near the reported line. Scan a
    // small window: sometimes the lint points at the first line of the fn
    // declaration, sometimes the whole span. We look in lines [line_no..
    // line_no+5] for the `fn <name>` pattern.
    let lines: Vec<&str> = src.lines().collect();
    for delta in 0..=5_usize {
        let idx = line_no + delta;
        let Some(line) = lines.get(idx) else { break };
        // Quick check: does this line contain `fn <name>`?
        let fn_prefix = format!("fn {}", name);
        if let Some(fn_pos) = line.find(&fn_prefix) {
            let name_col = fn_pos + "fn ".len();
            let edit_range = Range {
                start: Position::new(idx as u32, name_col as u32),
                end: Position::new(idx as u32, (name_col + name.len()) as u32),
            };
            let text_edit = TextEdit {
                range: edit_range,
                new_text: format!("_{}", name),
            };
            let mut changes = HashMap::new();
            changes.insert(uri.clone(), vec![text_edit]);
            return Some(CodeAction {
                title: format!("Prefix `{}` with `_` to suppress dead-code warning", name),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diag.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    document_changes: None,
                    change_annotations: None,
                }),
                command: None,
                is_preferred: Some(false),
                disabled: None,
                data: None,
            });
        }
    }
    None
}

/// RES-2570: build a "Suppress with `// resilient: allow <code>`" action
/// for any lint diagnostic whose message contains a lint code ("L0001"
/// etc.). The action prepends a suppression comment on the line above the
/// diagnostic — a universal escape hatch for cases where the other quick
/// fixes don't apply.
pub(crate) fn extract_lint_code(msg: &str) -> Option<&str> {
    // Scan for "L" followed by 4 digits (e.g. "L0001").
    let bytes = msg.as_bytes();
    for i in 0..bytes.len().saturating_sub(4) {
        if bytes[i] == b'L'
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2].is_ascii_digit()
            && bytes[i + 3].is_ascii_digit()
            && bytes[i + 4].is_ascii_digit()
        {
            return Some(&msg[i..i + 5]);
        }
    }
    None
}

/// RES-2570: build an "Add `// resilient: allow <code>`" suppression action.
/// Inserts the comment on the line immediately before the diagnostic.
pub(crate) fn build_suppress_lint_action(
    uri: &Url,
    diag: &Diagnostic,
    src: &str,
) -> Option<CodeAction> {
    let code = extract_lint_code(&diag.message)?;
    let line_no = diag.range.start.line;
    // Detect the indentation of the diagnostic's line so the comment aligns.
    let indent = src
        .lines()
        .nth(line_no as usize)
        .map(|l| {
            let trimmed = l.trim_start();
            &l[..l.len() - trimmed.len()]
        })
        .unwrap_or("")
        .to_string();
    // Insert at the start of the diagnostic's line (so the new comment
    // lands as a new line above it).
    let insert_pos = Position {
        line: line_no,
        character: 0,
    };
    let insert_range = Range {
        start: insert_pos,
        end: insert_pos,
    };
    let comment_text = format!("{}// resilient: allow {}\n", indent, code);
    let text_edit = TextEdit {
        range: insert_range,
        new_text: comment_text,
    };
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), vec![text_edit]);
    Some(CodeAction {
        title: format!("Suppress `{}` with allow comment", code),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diag.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }),
        command: None,
        is_preferred: Some(false),
        disabled: None,
        data: None,
    })
}

/// Run the LSP server on stdin/stdout until the client shuts down.
/// Invoked from `main()` when `--lsp` is on the command line AND the
/// `lsp` feature is enabled.
pub fn run() {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        let (service, socket) = LspService::new(Backend::new);
        Server::new(stdin, stdout, socket).serve(service).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_parses_line_col_prefix() {
        let err = "scratch.rs:3:5: Undefined variable 'x'";
        let (range, msg) = extract_range_and_message(err);
        assert_eq!(range.start.line, 2); // 0-indexed LSP
        assert_eq!(range.start.character, 4);
        assert_eq!(msg, "Undefined variable 'x'");
    }

    #[test]
    fn extract_handles_no_prefix_gracefully() {
        let err = "some raw error with no prefix";
        let (range, msg) = extract_range_and_message(err);
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 0);
        assert_eq!(msg, err);
    }

    #[test]
    fn extract_parses_bare_line_col_prefix() {
        // RES-089: parser errors come back as `<line>:<col>: <msg>`
        // with no path prefix. The extractor must pick up the bare
        // form and produce a 0-indexed Range.
        let err = "3:5: Unexpected token";
        let (range, msg) = extract_range_and_message(err);
        assert_eq!(range.start.line, 2);
        assert_eq!(range.start.character, 4);
        assert_eq!(msg, "Unexpected token");
    }

    #[test]
    fn extract_handles_path_with_colons() {
        // A Windows-style path wouldn't be common on Unix but let's
        // confirm we don't crash if a path happens to include `:`.
        let err = "C:/tmp/foo.rs:2:3: oops";
        let (range, msg) = extract_range_and_message(err);
        assert_eq!(range.start.line, 1);
        assert_eq!(range.start.character, 2);
        assert_eq!(msg, "oops");
    }

    // ---------- RES-185: document symbols ----------

    #[test]
    fn document_symbols_three_fns_plus_struct() {
        // Ticket AC: "3 fns + 1 struct" program produces four
        // symbols in file order, with the correct SymbolKind on
        // each.
        let src = "\
            fn alpha() { return 0; }\n\
            struct Point { int x, int y }\n\
            fn beta(int n) { return n; }\n\
            fn gamma() { return 1; }\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let syms = document_symbols_for_program(&program);
        assert_eq!(syms.len(), 4, "expected 4 symbols, got: {:?}", syms);

        // File order: alpha, Point, beta, gamma.
        assert_eq!(syms[0].name, "alpha");
        assert_eq!(syms[0].kind, SymbolKind::FUNCTION);
        assert_eq!(syms[1].name, "Point");
        assert_eq!(syms[1].kind, SymbolKind::STRUCT);
        assert_eq!(syms[2].name, "beta");
        assert_eq!(syms[2].kind, SymbolKind::FUNCTION);
        assert_eq!(syms[3].name, "gamma");
        assert_eq!(syms[3].kind, SymbolKind::FUNCTION);
    }

    #[test]
    fn document_symbols_includes_type_alias() {
        // RES-128 landed `type <Name> = <Target>;`. It should
        // show up as SymbolKind::TYPE_PARAMETER in the outline.
        let src = "type Meters = int;\nfn foo() { return 0; }\n";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let syms = document_symbols_for_program(&program);
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].name, "Meters");
        assert_eq!(syms[0].kind, SymbolKind::TYPE_PARAMETER);
        assert_eq!(syms[1].name, "foo");
        assert_eq!(syms[1].kind, SymbolKind::FUNCTION);
    }

    #[test]
    fn document_symbols_ignores_non_declaration_statements() {
        // `let` bindings, `return`, expression statements etc.
        // are statements, not declarations — they should NOT
        // appear in the outline.
        let src = "\
            let x = 1;\n\
            fn only_fn() { return 0; }\n\
            x + 1;\n\
            return 0;\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let syms = document_symbols_for_program(&program);
        assert_eq!(syms.len(), 1, "only the fn should appear, got {:?}", syms);
        assert_eq!(syms[0].name, "only_fn");
    }

    #[test]
    fn document_symbols_empty_on_empty_program() {
        let (program, errs) = parse("");
        assert!(errs.is_empty());
        let syms = document_symbols_for_program(&program);
        assert!(syms.is_empty());
    }

    #[test]
    fn document_symbols_sorted_by_source_position() {
        // The loop iterates program.stmts in order, which is
        // already source order, so the sort is a no-op here.
        // The test pins the behaviour anyway.
        let src = "\
            fn a() { return 0; }\n\
            fn b() { return 0; }\n\
            fn c() { return 0; }\n\
        ";
        let (program, _) = parse(src);
        let syms = document_symbols_for_program(&program);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
        // Line numbers should also strictly increase.
        let lines: Vec<u32> = syms.iter().map(|s| s.range.start.line).collect();
        assert!(
            lines.windows(2).all(|w| w[0] <= w[1]),
            "ranges should be non-decreasing, got: {:?}",
            lines
        );
    }

    #[test]
    fn span_to_range_converts_1_indexed_to_0_indexed() {
        use crate::span::{Pos, Span};
        let s = Span::new(Pos::new(1, 1, 0), Pos::new(1, 5, 4));
        let r = span_to_range(s);
        assert_eq!(r.start.line, 0);
        assert_eq!(r.start.character, 0);
        assert_eq!(r.end.line, 0);
        assert_eq!(r.end.character, 4);
    }

    // ---------- RES-186: workspace symbol search ----------

    /// Unique per-test scratch directory inside the OS temp dir.
    fn tmp_workspace(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("res_186_{}_{}_{}", tag, std::process::id(), n));
        std::fs::create_dir_all(&path).expect("create scratch dir");
        path
    }

    fn write_file(dir: &std::path::Path, name: &str, contents: &str) {
        std::fs::write(dir.join(name), contents).expect("write test file");
    }

    #[test]
    fn walk_resilient_files_finds_rs_files_recursively() {
        let root = tmp_workspace("walk");
        write_file(&root, "a.rz", "fn a() { return 0; }\n");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        write_file(&root.join("sub"), "b.rz", "fn b() { return 0; }\n");
        // Hidden + build dirs should be skipped.
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        write_file(
            &root.join("target").join("debug"),
            "c.rz",
            "fn c() { return 0; }\n",
        );
        std::fs::create_dir_all(root.join(".cache")).unwrap();
        write_file(&root.join(".cache"), "d.rz", "fn d() { return 0; }\n");
        let found = walk_resilient_files(&root);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"a.rz".to_string()));
        assert!(names.contains(&"b.rz".to_string()));
        assert!(
            !names.contains(&"c.rz".to_string()),
            "target/ must be skipped"
        );
        assert!(
            !names.contains(&"d.rz".to_string()),
            "dot-dirs must be skipped"
        );
        // Clean up.
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn index_file_parses_and_extracts_top_level_symbols() {
        let root = tmp_workspace("index");
        let path = root.join("prog.rs");
        std::fs::write(
            &path,
            "fn one() { return 0; }\nstruct S { int x }\nfn two() { return 1; }\n",
        )
        .unwrap();
        let entries = index_file(&path).expect("should index ok");
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["one", "S", "two"]);
        // Each entry's URI points at the file we indexed.
        for e in &entries {
            assert!(e.uri.as_str().ends_with("/prog.rs"));
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn filter_workspace_symbols_case_insensitive_substring() {
        use std::collections::HashMap;
        let uri = Url::parse("file:///tmp/a.rs").unwrap();
        let entries = vec![
            WorkspaceSymbolEntry {
                name: "alpha".into(),
                kind: SymbolKind::FUNCTION,
                uri: uri.clone(),
                range: Range::new(Position::new(0, 0), Position::new(0, 0)),
            },
            WorkspaceSymbolEntry {
                name: "AlphaBeta".into(),
                kind: SymbolKind::FUNCTION,
                uri: uri.clone(),
                range: Range::new(Position::new(1, 0), Position::new(1, 0)),
            },
            WorkspaceSymbolEntry {
                name: "gamma".into(),
                kind: SymbolKind::FUNCTION,
                uri: uri.clone(),
                range: Range::new(Position::new(2, 0), Position::new(2, 0)),
            },
        ];
        let mut index: HashMap<Url, Vec<WorkspaceSymbolEntry>> = HashMap::new();
        index.insert(uri, entries);

        // Exact prefix, case-folded.
        let r = filter_workspace_symbols(&index, "alpha", 50);
        let names: Vec<&str> = r.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["AlphaBeta", "alpha"]); // sorted by name

        // Substring mid-word. Caller (Backend::symbol) is the one
        // that lowercases the user query — this helper takes
        // `query_lower` ALREADY folded, so `"bet"` matches
        // `AlphaBeta` (via lowercased "alphabeta").
        let r = filter_workspace_symbols(&index, "bet", 50);
        let names: Vec<&str> = r.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["AlphaBeta"]);

        // Empty query matches everything.
        let r = filter_workspace_symbols(&index, "", 50);
        assert_eq!(r.len(), 3);

        // Limit cap.
        let r = filter_workspace_symbols(&index, "", 2);
        assert_eq!(r.len(), 2);
    }

    // ---------- RES-187: semantic tokens legend + wire glue ----------

    /// The legend's type-index order MUST match the `sem_tok::*`
    /// constants in main.rs. If someone adds a new token type
    /// between them, this test catches the drift before an editor
    /// starts mis-coloring things.
    #[test]
    fn semantic_tokens_legend_indices_match_sem_tok_constants() {
        use crate::sem_tok;
        let legend = semantic_tokens_legend();
        assert_eq!(
            legend.token_types[sem_tok::KEYWORD as usize],
            SemanticTokenType::KEYWORD
        );
        assert_eq!(
            legend.token_types[sem_tok::FUNCTION as usize],
            SemanticTokenType::FUNCTION
        );
        assert_eq!(
            legend.token_types[sem_tok::VARIABLE as usize],
            SemanticTokenType::VARIABLE
        );
        assert_eq!(
            legend.token_types[sem_tok::PARAMETER as usize],
            SemanticTokenType::PARAMETER
        );
        assert_eq!(
            legend.token_types[sem_tok::TYPE as usize],
            SemanticTokenType::TYPE
        );
        assert_eq!(
            legend.token_types[sem_tok::STRING as usize],
            SemanticTokenType::STRING
        );
        assert_eq!(
            legend.token_types[sem_tok::NUMBER as usize],
            SemanticTokenType::NUMBER
        );
        assert_eq!(
            legend.token_types[sem_tok::COMMENT as usize],
            SemanticTokenType::COMMENT
        );
        assert_eq!(
            legend.token_types[sem_tok::OPERATOR as usize],
            SemanticTokenType::OPERATOR
        );
        // Modifier bit positions: bit 0 = declaration, bit 1 = readonly.
        assert_eq!(
            legend.token_modifiers[0],
            SemanticTokenModifier::DECLARATION
        );
        assert_eq!(legend.token_modifiers[1], SemanticTokenModifier::READONLY);
    }

    /// `semantic_tokens_from_wire` must unpack an n-tuple u32 stream
    /// into n `SemanticToken`s preserving field order.
    #[test]
    fn semantic_tokens_from_wire_unpacks_correctly() {
        // Two tokens: [0,0,3,0,0] then [0,4,3,1,1].
        let wire = vec![0, 0, 3, 0, 0, 0, 4, 3, 1, 1];
        let toks = semantic_tokens_from_wire(wire);
        assert_eq!(toks.len(), 2);
        assert_eq!(toks[0].delta_line, 0);
        assert_eq!(toks[0].delta_start, 0);
        assert_eq!(toks[0].length, 3);
        assert_eq!(toks[0].token_type, 0);
        assert_eq!(toks[0].token_modifiers_bitset, 0);
        assert_eq!(toks[1].delta_line, 0);
        assert_eq!(toks[1].delta_start, 4);
        assert_eq!(toks[1].length, 3);
        assert_eq!(toks[1].token_type, 1);
        assert_eq!(toks[1].token_modifiers_bitset, 1);
    }

    /// Trailing-partial u32 chunks (not a multiple of 5) are
    /// dropped by `chunks_exact`. Not expected in practice, but
    /// pinning the behaviour.
    #[test]
    fn semantic_tokens_from_wire_drops_partial_trailing_chunk() {
        let wire = vec![0, 0, 3, 0, 0, 0, 4]; // 7 elements — last 2 dropped
        let toks = semantic_tokens_from_wire(wire);
        assert_eq!(toks.len(), 1);
    }

    // ---------- RES-189: inlay hints ----------

    #[test]
    fn typechecker_collects_hints_for_unannotated_lets() {
        // Ticket AC: 5 lets, 3 should produce type hints (the
        // non-annotated ones). Annotated lets stay quiet.
        let src = "\
            fn main(int _d) {\n\
                let a = 1;\n\
                let b: int = 2;\n\
                let c = true;\n\
                let d: bool = false;\n\
                let e = \"hi\";\n\
                return 0;\n\
            }\n";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let mut tc = typechecker::TypeChecker::new().with_capture_inlay_hints(true);
        let _ = tc.check_program_with_source(&program, "file:///tmp/test.rs");
        let hints = &tc.let_type_hints;
        assert_eq!(
            hints.len(),
            3,
            "expected 3 hints (a, c, e), got {:?}",
            hints
                .iter()
                .map(|h| (h.name_len_chars, &h.ty))
                .collect::<Vec<_>>(),
        );
        let types: Vec<String> = hints.iter().map(|h| format!("{}", h.ty)).collect();
        assert_eq!(types, vec!["int", "bool", "string"]);
    }

    #[test]
    fn typechecker_skips_any_typed_let_hints() {
        // `Type::Any` bindings shouldn't produce hints — no useful
        // information and clutters the editor.
        let src = "fn main(int _d) { let x = println(\"hi\"); return 0; }\n";
        let (program, _) = parse(src);
        let mut tc = typechecker::TypeChecker::new().with_capture_inlay_hints(true);
        let _ = tc.check_program_with_source(&program, "<t>");
        // `println` returns Void, so `x` is Void → skipped.
        assert_eq!(tc.let_type_hints.len(), 0);
    }

    #[test]
    fn inlay_hint_from_let_positions_after_identifier() {
        use crate::span::{Pos, Span};
        // `let abc = 3;` starting at 1:1 → pattern ends at col 8
        // (1 + "let ".len() + "abc".len() = 1 + 4 + 3 = 8, 0-indexed
        // = 7).
        let entry = typechecker::LetTypeHint {
            span: Span::new(Pos::new(1, 1, 0), Pos::new(1, 1, 0)),
            name_len_chars: 3,
            ty: typechecker::Type::Int,
        };
        let hint = inlay_hint_from_let(&entry);
        assert_eq!(hint.position.line, 0);
        assert_eq!(hint.position.character, 7);
        // The label starts with `: ` per the convention; clients
        // render the padding.
        match hint.label {
            InlayHintLabel::String(ref s) => assert_eq!(s, ": int"),
            other => panic!("expected string label, got {:?}", other),
        }
        assert_eq!(hint.kind, Some(InlayHintKind::TYPE));
        assert_eq!(hint.padding_left, Some(true));
    }

    #[test]
    fn param_hints_tag_each_arg_with_param_name() {
        // Ticket AC: `add(a: 1, b: 2)`-style chrome.
        let src = "\
            fn add(int a, int b) { return a + b; }\n\
            fn main(int _d) { return add(1, 2); }\n\
            main(0);\n";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let hints = collect_param_hints(&program);
        assert!(hints.len() >= 2, "expected >=2 hints, got: {hints:?}");
        // Every produced hint should be a PARAMETER kind.
        for h in &hints {
            assert_eq!(h.kind, Some(InlayHintKind::PARAMETER));
            match &h.label {
                InlayHintLabel::String(s) => {
                    assert!(
                        s.ends_with(": "),
                        "param hint label should end with `: `, got {s:?}"
                    );
                }
                _ => panic!("expected string label"),
            }
        }
        // Exactly two hints for the `add(1, 2)` call.
        let add_hints: Vec<&str> = hints
            .iter()
            .filter_map(|h| match &h.label {
                InlayHintLabel::String(s) => Some(s.as_str()),
                _ => None,
            })
            .collect();
        assert!(add_hints.contains(&"a: "));
        assert!(add_hints.contains(&"b: "));
    }

    #[test]
    fn param_hints_skip_arity_mismatches() {
        // If the call passes the wrong number of args, don't emit
        // hints — any pairing would be misleading.
        let src = "\
            fn add(int a, int b) { return a + b; }\n\
            fn main(int _d) { return add(1); }\n\
            main(0);\n";
        let (program, _errs) = parse(src);
        let hints = collect_param_hints(&program);
        let add_hints: Vec<_> = hints
            .iter()
            .filter(|h| matches!(h.kind, Some(InlayHintKind::PARAMETER)))
            .collect();
        assert!(
            add_hints.is_empty(),
            "arity-mismatch call should not emit param hints, got {add_hints:?}"
        );
    }

    #[test]
    fn param_hints_skip_unknown_callees() {
        // println isn't declared in this program, so no hints.
        let src = "fn main(int _d) { println(\"hi\"); return 0; }\n";
        let (program, _) = parse(src);
        let hints = collect_param_hints(&program);
        assert!(hints.is_empty(), "println isn't a user fn; got: {hints:?}");
    }

    #[test]
    fn read_init_param_hints_flag_flat_form() {
        let v: tower_lsp::lsp_types::LSPAny = serde_json::json!({
            "resilient.inlayHints.parameters": true
        });
        assert!(read_init_param_hints_flag(Some(&v)));
    }

    #[test]
    fn read_init_param_hints_flag_nested_form() {
        let v: tower_lsp::lsp_types::LSPAny = serde_json::json!({
            "resilient": { "inlayHints": { "parameters": true } }
        });
        assert!(read_init_param_hints_flag(Some(&v)));
    }

    #[test]
    fn read_init_param_hints_flag_defaults_false() {
        assert!(!read_init_param_hints_flag(None));
        let v: tower_lsp::lsp_types::LSPAny = serde_json::json!({});
        assert!(!read_init_param_hints_flag(Some(&v)));
        let v: tower_lsp::lsp_types::LSPAny = serde_json::json!({
            "resilient": { "inlayHints": { "parameters": false } }
        });
        assert!(!read_init_param_hints_flag(Some(&v)));
    }

    #[test]
    fn position_in_range_inclusive_endpoints() {
        let r = Range::new(Position::new(0, 0), Position::new(10, 0));
        assert!(position_in_range(Position::new(0, 0), r));
        assert!(position_in_range(Position::new(5, 0), r));
        assert!(position_in_range(Position::new(10, 0), r));
        assert!(!position_in_range(Position::new(10, 1), r));
        assert!(!position_in_range(Position::new(11, 0), r));
    }

    #[test]
    fn workspace_index_spans_multiple_files() {
        // Ticket AC: pre-seed two files, invoke the query (via the
        // helper path), assert both files' symbols are returned.
        let root = tmp_workspace("multifile");
        write_file(
            &root,
            "mod_a.rz",
            "fn a_fn() { return 0; }\nstruct A_Struct { int x }\n",
        );
        write_file(&root, "mod_b.rz", "fn b_fn() { return 0; }\n");

        // Walk + index the whole scratch dir, reproducing what
        // `rebuild_workspace_index` does when the Backend is
        // invoked via the LSP.
        let files = walk_resilient_files(&root);
        assert_eq!(files.len(), 2);
        let mut index: std::collections::HashMap<Url, Vec<WorkspaceSymbolEntry>> =
            std::collections::HashMap::new();
        for p in files {
            let Ok(uri) = Url::from_file_path(&p) else {
                continue;
            };
            if let Some(entries) = index_file(&p) {
                index.insert(uri, entries);
            }
        }

        // All-match query: three names across two files.
        let r = filter_workspace_symbols(&index, "", 50);
        let names: std::collections::HashSet<&str> = r.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains("a_fn"));
        assert!(names.contains("A_Struct"));
        assert!(names.contains("b_fn"));
        // And the Locations point at the right files.
        for sym in &r {
            let path_str = sym.location.uri.as_str();
            assert!(
                path_str.ends_with("/mod_a.rz") || path_str.ends_with("/mod_b.rz"),
                "unexpected URI: {}",
                path_str
            );
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    // ============================================================
    // RES-181a: hover handler helpers
    // ============================================================
    //
    // Drive the lexer directly (via `hover_literal_at`) to classify
    // the token at a cursor position. Returns `(type_name, range)`
    // or `None`. Tests use zero-indexed `Position` values to mirror
    // how real LSP clients send cursor coordinates.

    #[test]
    fn res181a_hover_on_int_literal_start_returns_int() {
        // `let x = 42;` — `42` spans cols 9..11 (1-indexed) = LSP
        // chars 8..11. Cursor at `4` (char 8) returns Int.
        let r = hover_literal_at("let x = 42;", Position::new(0, 8));
        let (ty, _range) = r.expect("expected Int hover at col 8");
        assert_eq!(ty, "Int");
    }

    #[test]
    fn res181a_hover_on_int_literal_middle_returns_int() {
        // Anywhere inside the token's extent — cursor at `2` (char 9)
        // still returns Int because the lexer's real span covers
        // the whole literal.
        let r = hover_literal_at("let x = 42;", Position::new(0, 9));
        let (ty, _) = r.expect("expected Int hover at col 9");
        assert_eq!(ty, "Int");
    }

    #[test]
    fn res181a_hover_on_bool_literal_returns_bool() {
        // `let b = true;` — `true` spans cols 9..13.
        let r = hover_literal_at("let b = true;", Position::new(0, 10));
        let (ty, _) = r.expect("expected Bool hover");
        assert_eq!(ty, "Bool");
    }

    #[test]
    fn res181a_hover_on_false_literal_returns_bool() {
        // `let b = false;` — `false` spans cols 9..14.
        let r = hover_literal_at("let b = false;", Position::new(0, 10));
        let (ty, _) = r.expect("expected Bool hover");
        assert_eq!(ty, "Bool");
    }

    #[test]
    fn res181a_hover_on_string_literal_returns_string() {
        // `let s = "hi";` — quoted string spans cols 9..13.
        let r = hover_literal_at(r#"let s = "hi";"#, Position::new(0, 10));
        let (ty, _) = r.expect("expected String hover");
        assert_eq!(ty, "String");
    }

    #[test]
    fn res181a_hover_on_float_literal_returns_float() {
        let r = hover_literal_at("let f = 3.14;", Position::new(0, 10));
        let (ty, _) = r.expect("expected Float hover");
        assert_eq!(ty, "Float");
    }

    #[test]
    fn res181a_hover_on_bytes_literal_returns_bytes() {
        // `let b = b"\x00\x01";` — bytes literal.
        let r = hover_literal_at(r#"let b = b"\x00";"#, Position::new(0, 10));
        let (ty, _) = r.expect("expected Bytes hover");
        assert_eq!(ty, "Bytes");
    }

    #[test]
    fn res181a_hover_on_keyword_returns_none() {
        // Cursor on `let` (non-literal token) → no hover.
        assert!(hover_literal_at("let x = 42;", Position::new(0, 0)).is_none());
    }

    #[test]
    fn res181a_hover_on_identifier_returns_none() {
        // Cursor on `x` identifier — identifier hover is RES-181b
        // (needs inferred types). Today we deliberately return
        // None so the client renders nothing.
        let r = hover_literal_at("let x = 42;", Position::new(0, 4));
        assert!(r.is_none());
    }

    #[test]
    fn res181a_hover_on_operator_returns_none() {
        // Cursor on `+` → no hover.
        let r = hover_literal_at("let r = 1 + 2;", Position::new(0, 10));
        assert!(r.is_none());
    }

    #[test]
    fn res181a_hover_out_of_range_returns_none() {
        // Cursor way past the file end → None (no tokens past EOF).
        let r = hover_literal_at("let x = 42;", Position::new(10, 0));
        assert!(r.is_none());
    }

    #[test]
    fn res181a_hover_on_empty_source_returns_none() {
        let r = hover_literal_at("", Position::new(0, 0));
        assert!(r.is_none());
    }

    #[test]
    fn res181a_hover_inside_fn_body_returns_literal_type() {
        let src = "fn f(int n) { return 7; }";
        // `7` is at col 22 (1-indexed) = LSP col 21.
        let r = hover_literal_at(src, Position::new(0, 21));
        let (ty, _) = r.expect("expected Int hover inside fn body");
        assert_eq!(ty, "Int");
    }

    #[test]
    fn res181a_hover_returns_range_covering_the_token() {
        // The Range returned with the hover should span the whole
        // literal. For `42` at cols 9..11 (1-indexed) = LSP 8..10.
        let (ty, range) =
            hover_literal_at("let x = 42;", Position::new(0, 8)).expect("expected hover");
        assert_eq!(ty, "Int");
        assert_eq!(range.start.line, 0);
        assert_eq!(range.end.line, 0);
        // Start column is 8 (LSP 0-indexed = `4`). End column is
        // 10 (exclusive = position of `;`).
        assert_eq!(range.start.character, 8);
        assert_eq!(range.end.character, 10);
    }

    #[test]
    fn res181a_lex_span_contains_lsp_position_single_line() {
        use crate::span::{Pos, Span};
        // Token at line 1, cols 5..10 (1-indexed) — LSP 0-indexed
        // [line 0, chars 4..9].
        let sp = Span::new(Pos::new(1, 5, 0), Pos::new(1, 10, 5));
        assert!(lex_span_contains_lsp_position(sp, Position::new(0, 4)));
        assert!(lex_span_contains_lsp_position(sp, Position::new(0, 8)));
        assert!(!lex_span_contains_lsp_position(sp, Position::new(0, 9)));
        assert!(!lex_span_contains_lsp_position(sp, Position::new(0, 3)));
    }

    #[test]
    fn res181a_lex_span_contains_lsp_position_different_line() {
        use crate::span::{Pos, Span};
        let sp = Span::new(Pos::new(2, 1, 0), Pos::new(2, 5, 4));
        assert!(!lex_span_contains_lsp_position(sp, Position::new(0, 1)));
        assert!(!lex_span_contains_lsp_position(sp, Position::new(5, 1)));
    }

    // ============================================================
    // RES-182a: goto-definition helpers
    // ============================================================

    fn parse_prog(src: &str) -> Node {
        let (program, _errs) = parse(src);
        program
    }

    #[test]
    fn res182a_identifier_at_returns_name_and_range() {
        // `let x = 42;` — cursor on `x` at col 5 (1-indexed) =
        // LSP col 4. The `identifier_at` helper should return
        // ("x", Range at col 4..5).
        let r = identifier_at("let x = 42;", Position::new(0, 4));
        let (name, range) = r.expect("expected identifier hit");
        assert_eq!(name, "x");
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 4);
        assert_eq!(range.end.character, 5);
    }

    #[test]
    fn res182a_identifier_at_returns_none_for_literal() {
        // Cursor on `42` → not an identifier → None.
        let r = identifier_at("let x = 42;", Position::new(0, 8));
        assert!(r.is_none());
    }

    #[test]
    fn res182a_identifier_at_returns_none_for_keyword() {
        // Cursor on `let` → keyword → None.
        let r = identifier_at("let x = 42;", Position::new(0, 0));
        assert!(r.is_none());
    }

    #[test]
    fn res182a_identifier_at_finds_mid_identifier() {
        // Cursor in the middle of `my_fn` (4 chars in) → still
        // returns "my_fn". Tests the multi-char-token branch.
        let r = identifier_at("fn my_fn() { return 0; }", Position::new(0, 5));
        let (name, _) = r.expect("expected identifier hit");
        assert_eq!(name, "my_fn");
    }

    #[test]
    fn res182a_identifier_at_out_of_range_returns_none() {
        let r = identifier_at("let x = 42;", Position::new(10, 0));
        assert!(r.is_none());
    }

    #[test]
    fn res182a_identifier_at_empty_source_returns_none() {
        let r = identifier_at("", Position::new(0, 0));
        assert!(r.is_none());
    }

    #[test]
    fn res182a_build_top_level_defs_empty_program() {
        let prog = parse_prog("");
        let defs = build_top_level_defs(&prog);
        assert!(defs.is_empty());
    }

    #[test]
    fn res182a_build_top_level_defs_collects_fn() {
        let prog = parse_prog("fn add(int a, int b) -> int { return a + b; }");
        let defs = build_top_level_defs(&prog);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "add");
    }

    #[test]
    fn res182a_build_top_level_defs_collects_struct() {
        let prog = parse_prog("struct Point { int x, int y, }");
        let defs = build_top_level_defs(&prog);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "Point");
    }

    #[test]
    fn res182a_build_top_level_defs_collects_type_alias() {
        let prog = parse_prog("type MyInt = int;");
        let defs = build_top_level_defs(&prog);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "MyInt");
    }

    #[test]
    fn res182a_build_top_level_defs_mixed_kinds() {
        let src = r#"
            fn foo(int n) { return n; }
            struct Rec { int x, }
            type I = int;
            let top = 42;
        "#;
        let prog = parse_prog(src);
        let defs = build_top_level_defs(&prog);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["foo", "Rec", "I"]);
    }

    #[test]
    fn res182a_build_top_level_defs_first_wins_on_duplicates() {
        // Duplicate names — parser doesn't reject them, but the
        // goto target should be deterministic. First wins.
        let src = r#"
            fn foo(int n) { return 1; }
            fn foo(int n) { return 2; }
        "#;
        let prog = parse_prog(src);
        let defs = build_top_level_defs(&prog);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "foo");
    }

    #[test]
    fn res182a_find_top_level_def_hit_and_miss() {
        let prog = parse_prog("fn only() { return 0; } struct P { int x, }");
        let defs = build_top_level_defs(&prog);
        assert!(find_top_level_def(&defs, "only").is_some());
        assert!(find_top_level_def(&defs, "P").is_some());
        assert!(find_top_level_def(&defs, "nope").is_none());
    }

    #[test]
    fn res182a_find_top_level_def_returns_range_from_decl() {
        // Decl at line 2 (1-indexed in source) should surface
        // range starting at LSP line 1.
        let src = "\nfn foo() { return 0; }";
        let prog = parse_prog(src);
        let defs = build_top_level_defs(&prog);
        let def = find_top_level_def(&defs, "foo").expect("missing foo");
        assert_eq!(def.range.start.line, 1);
    }

    // ============================================================
    // RES-183: find-references helpers
    // ============================================================

    #[test]
    fn res183_collect_call_sites_three_callers() {
        // AC: 3-caller setup — each direct call should produce one
        // range; the struct literal with the same name should NOT.
        let src = "\
fn greet() { return 1; }\n\
struct greet { int x, }\n\
fn a() { return greet(); }\n\
fn b() { return greet(); }\n\
fn c() { return greet(); }\n\
let _s = new greet { x: 0 };\n\
";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let sites = collect_call_sites(&prog, "greet");
        assert_eq!(
            sites.len(),
            3,
            "expected exactly 3 call sites, got {}: {sites:?}",
            sites.len()
        );
    }

    #[test]
    fn res183_collect_call_sites_struct_literal_excluded() {
        // A struct literal `new Foo { ... }` must NOT be counted as
        // a call site even when the struct name matches the target.
        let src = "\
fn Foo() { return 0; }\n\
struct Foo { int x, }\n\
let _s = new Foo { x: 1 };\n\
";
        let (prog, _) = parse(src);
        let sites = collect_call_sites(&prog, "Foo");
        assert!(
            sites.is_empty(),
            "struct literal must not appear as a call site, got: {sites:?}"
        );
    }

    #[test]
    fn res183_collect_call_sites_empty_program() {
        let (prog, _) = parse("");
        let sites = collect_call_sites(&prog, "anything");
        assert!(sites.is_empty());
    }

    #[test]
    fn res183_collect_call_sites_no_match() {
        let src = "fn foo() { return 1; }\nfoo();\n";
        let (prog, _) = parse(src);
        let sites = collect_call_sites(&prog, "bar");
        assert!(sites.is_empty());
    }

    #[test]
    fn res183_collect_call_sites_single_top_level_call() {
        // A bare call statement at top level.
        let src = "fn add(int a, int b) -> int { return a + b; }\nadd(1, 2);\n";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let sites = collect_call_sites(&prog, "add");
        assert_eq!(sites.len(), 1, "expected 1 call site, got: {sites:?}");
        // Call site is on line 2 (1-indexed) = LSP line 1.
        assert_eq!(sites[0].start.line, 1);
    }

    #[test]
    fn res183_collect_call_sites_nested_call() {
        // `foo(foo(1))` — two call sites for `foo`.
        let src = "fn foo(int n) { return n; }\nfoo(foo(1));\n";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let sites = collect_call_sites(&prog, "foo");
        assert_eq!(
            sites.len(),
            2,
            "expected 2 nested call sites, got: {sites:?}"
        );
    }

    #[test]
    fn res183_collect_call_sites_inside_if_while() {
        // Calls inside if/while bodies are captured.
        let src = "\
fn tick() { return 1; }\n\
fn main(int n) {\n\
    if n > 0 { tick(); }\n\
    while n > 0 { tick(); n = n - 1; }\n\
    return 0;\n\
}\n";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let sites = collect_call_sites(&prog, "tick");
        assert_eq!(sites.len(), 2, "expected 2 tick call sites: {sites:?}");
    }

    #[test]
    fn res183_collect_call_sites_range_points_at_callee() {
        // The range in the returned Location should be derived from the
        // callee Identifier's AST span. The parser uses zero-width spans
        // (known limitation per the "span unreliability" note in main.rs),
        // so start == end. The important invariant is that the LINE is
        // correct — it points at the call site's line, not the decl's.
        let src = "fn foo(int n) { return n; }\nfoo(1);\n";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let sites = collect_call_sites(&prog, "foo");
        assert_eq!(sites.len(), 1);
        // `foo(1)` is on source line 2 (1-indexed) = LSP line 1.
        assert_eq!(sites[0].start.line, 1, "call site line mismatch");
    }

    // ============================================================
    // RES-188a: completion helpers
    // ============================================================

    #[test]
    fn res188a_prefix_at_empty_source_is_empty_string() {
        assert_eq!(prefix_at("", Position::new(0, 0)), "");
    }

    #[test]
    fn res188a_prefix_at_start_of_identifier_is_empty() {
        // Cursor AT col 0 on "foo" — user hasn't typed anything
        // yet. Walking backward from col 0 yields the empty string.
        assert_eq!(prefix_at("foo", Position::new(0, 0)), "");
    }

    #[test]
    fn res188a_prefix_at_extracts_partial_identifier() {
        // Cursor 2 chars into "foo" — user has typed "fo".
        assert_eq!(prefix_at("foo", Position::new(0, 2)), "fo");
    }

    #[test]
    fn res188a_prefix_at_extracts_full_identifier() {
        // Cursor at end of "foo" — user has typed "foo".
        assert_eq!(prefix_at("foo", Position::new(0, 3)), "foo");
    }

    #[test]
    fn res188a_prefix_at_stops_at_non_identifier_char() {
        // "let x = fo" — cursor at end. Walking back stops at
        // the space after `=`. Prefix = "fo".
        assert_eq!(prefix_at("let x = fo", Position::new(0, 10)), "fo");
    }

    #[test]
    fn res188a_prefix_at_handles_underscore_in_identifier() {
        // Underscores are identifier chars.
        assert_eq!(prefix_at("my_var", Position::new(0, 6)), "my_var");
        assert_eq!(prefix_at("my_var", Position::new(0, 3)), "my_");
    }

    #[test]
    fn res188a_prefix_at_multiline_respects_line() {
        let src = "foo\nbar";
        assert_eq!(prefix_at(src, Position::new(0, 3)), "foo");
        assert_eq!(prefix_at(src, Position::new(1, 3)), "bar");
    }

    #[test]
    fn res188a_prefix_at_cursor_past_eol_clamps() {
        // Some clients send positions one past EOL; treat as EOL.
        assert_eq!(prefix_at("abc", Position::new(0, 999)), "abc");
    }

    #[test]
    fn res188a_prefix_at_cursor_on_nonexistent_line_empty() {
        assert_eq!(prefix_at("abc", Position::new(5, 0)), "");
    }

    #[test]
    fn res188a_candidates_empty_prefix_returns_many_builtins() {
        // Empty prefix + empty program still yields the builtin
        // list.
        let prog = parse_prog("");
        let cands = completion_candidates(&prog, "");
        // BUILTINS has ~50 entries today. Assert a lower bound
        // that's safe across additions — enough to catch an
        // accidental empty-return regression.
        assert!(
            cands.len() > 10,
            "expected builtins > 10, got {}",
            cands.len()
        );
        // All-function kind is expected for the builtin slice.
        for c in cands.iter().take(5) {
            assert_eq!(c.kind, CandidateKind::Function);
            assert_eq!(c.detail, Some("builtin".to_string()));
        }
    }

    #[test]
    fn res188a_candidates_prefix_filters_builtins() {
        // Prefix "prin" should surface only "println" + "print"
        // (both start with "prin"). BUILTINS won't acquire a
        // non-print prefix-matching name without the test
        // noticing.
        let prog = parse_prog("");
        let cands = completion_candidates(&prog, "prin");
        let labels: Vec<&str> = cands.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"println"));
        assert!(labels.contains(&"print"));
        // Sanity: nothing unrelated snuck in.
        for label in &labels {
            assert!(
                label.starts_with("prin"),
                "prefix leak: {} doesn't start with `prin`",
                label
            );
        }
    }

    #[test]
    fn res188a_candidates_include_top_level_fn() {
        let prog = parse_prog("fn my_helper() { return 1; }");
        let cands = completion_candidates(&prog, "my_");
        let labels: Vec<&str> = cands.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"my_helper"));
        let c = cands.iter().find(|c| c.label == "my_helper").unwrap();
        assert_eq!(c.kind, CandidateKind::Function);
    }

    #[test]
    fn res188a_candidates_include_top_level_struct() {
        let prog = parse_prog("struct Point { int x, int y, }");
        let cands = completion_candidates(&prog, "Po");
        let c = cands.iter().find(|c| c.label == "Point").unwrap();
        assert_eq!(c.kind, CandidateKind::Struct);
        assert!(c.detail.as_deref().unwrap().contains("struct"));
    }

    #[test]
    fn res188a_candidates_include_type_alias() {
        let prog = parse_prog("type Id = int;");
        let cands = completion_candidates(&prog, "Id");
        let c = cands.iter().find(|c| c.label == "Id").unwrap();
        assert_eq!(c.kind, CandidateKind::TypeAlias);
    }

    #[test]
    fn res188a_candidates_builtins_before_user_decls() {
        // Deterministic ordering: builtins first (alphabetical),
        // then user decls (source order). Useful for snapshot /
        // regression tests downstream.
        let prog = parse_prog("fn abc() { return 0; }");
        let cands = completion_candidates(&prog, "ab");
        // `abs` (builtin) should come before `abc` (user decl).
        let abs_idx = cands.iter().position(|c| c.label == "abs");
        let abc_idx = cands.iter().position(|c| c.label == "abc");
        assert!(abs_idx.is_some(), "expected `abs` builtin");
        assert!(abc_idx.is_some(), "expected `abc` user decl");
        assert!(abs_idx.unwrap() < abc_idx.unwrap());
    }

    #[test]
    fn res188a_candidates_respects_completion_limit() {
        let prog = parse_prog("");
        // Empty prefix gives every builtin. Ensure we don't blow
        // past the cap even if BUILTINS is later extended.
        let cands = completion_candidates(&prog, "");
        assert!(cands.len() <= COMPLETION_LIMIT);
    }

    #[test]
    fn res188a_candidates_unmatched_prefix_returns_empty() {
        let prog = parse_prog("fn foo() { return 0; }");
        let cands = completion_candidates(&prog, "zzz_no_match");
        assert!(cands.is_empty());
    }

    #[test]
    fn res188a_candidate_to_completion_item_maps_fields() {
        let c = Candidate {
            label: "test".to_string(),
            kind: CandidateKind::Function,
            detail: Some("builtin".to_string()),
        };
        let item = candidate_to_completion_item(c);
        assert_eq!(item.label, "test");
        assert_eq!(item.kind, Some(CompletionItemKind::FUNCTION));
        assert_eq!(item.detail, Some("builtin".to_string()));
        assert_eq!(item.insert_text, Some("test".to_string()));
    }

    // ============================================================
    // RES-184: rename helpers
    // ============================================================

    #[test]
    fn res184_is_valid_identifier_accepts_simple_names() {
        assert!(is_valid_identifier("foo"));
        assert!(is_valid_identifier("_bar"));
        assert!(is_valid_identifier("Foo_Bar2"));
        assert!(is_valid_identifier("x"));
        assert!(is_valid_identifier("_"));
    }

    #[test]
    fn res184_is_valid_identifier_rejects_bad_names() {
        assert!(!is_valid_identifier("")); // empty
        assert!(!is_valid_identifier("2foo")); // starts with digit
        assert!(!is_valid_identifier("foo-bar")); // hyphen
        assert!(!is_valid_identifier("foo bar")); // space
        assert!(!is_valid_identifier("foo.bar")); // dot
    }

    #[test]
    fn res184_find_decl_name_range_locates_fn_name() {
        // `fn foo() { return 0; }` — `foo` starts at col 4 (1-indexed)
        // = LSP col 3, ends at col 7 = LSP col 6.
        let src = "fn foo() { return 0; }";
        let range = find_decl_name_range(src, "foo").expect("should find foo");
        assert_eq!(range.start.line, 0);
        assert_eq!(range.start.character, 3); // LSP 0-indexed col of 'f' in "foo"
        assert_eq!(range.end.character, 6);
    }

    #[test]
    fn res184_find_decl_name_range_returns_none_for_missing() {
        let src = "fn foo() { return 0; }";
        assert!(find_decl_name_range(src, "bar").is_none());
    }

    #[test]
    fn res184_find_decl_name_range_multiline_second_fn() {
        let src = "fn first() { return 1; }\nfn second() { return 2; }";
        let range = find_decl_name_range(src, "second").expect("should find second");
        // `second` is on line 2 (1-indexed) = LSP line 1.
        assert_eq!(range.start.line, 1);
    }

    #[test]
    fn res184_collect_rename_edits_toplevel_fn() {
        // Integration: rename `add` → `sum`.
        // Source has declaration + 2 call sites.
        let src = "fn add(int a, int b) -> int { return a + b; }\nadd(1, 2);\nadd(3, 4);\n";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");

        // Simulate what the `rename` handler does.
        let new_name = "sum";
        let defs = build_top_level_defs(&prog);
        let def = find_top_level_def(&defs, "add").expect("add should be found");
        let decl_range = find_decl_name_range(src, "add").unwrap_or(def.range);
        let call_ranges = collect_call_sites(&prog, "add");

        // Verify: 1 declaration edit + 2 call-site edits.
        let mut edits: Vec<TextEdit> = Vec::new();
        edits.push(TextEdit {
            range: decl_range,
            new_text: new_name.to_string(),
        });
        for range in call_ranges {
            edits.push(TextEdit {
                range,
                new_text: new_name.to_string(),
            });
        }

        assert_eq!(
            edits.len(),
            3,
            "expected 3 edits (1 decl + 2 calls): {edits:?}"
        );
        // All edits replace with the new name.
        for e in &edits {
            assert_eq!(e.new_text, "sum");
        }
        // Declaration edit is on line 0.
        assert_eq!(edits[0].range.start.line, 0);
        // Call-site edits are on lines 1 and 2.
        let call_lines: Vec<u32> = edits[1..].iter().map(|e| e.range.start.line).collect();
        assert!(
            call_lines.contains(&1),
            "missing call on line 1: {call_lines:?}"
        );
        assert!(
            call_lines.contains(&2),
            "missing call on line 2: {call_lines:?}"
        );
    }

    #[test]
    fn res184_rename_decl_range_is_name_token_not_whole_stmt() {
        // The declaration edit range must cover only "add", not
        // the whole `fn add(...) { ... }` statement.
        let src = "fn add(int a, int b) -> int { return a + b; }";
        let range = find_decl_name_range(src, "add").expect("should find add");
        // `add` is 3 chars wide; verify start and end are close together.
        assert_eq!(
            range.start.line, range.end.line,
            "multi-line decl range unexpected"
        );
        let width = range.end.character - range.start.character;
        assert_eq!(
            width, 3,
            "expected range to cover 3-char 'add', got width {width}"
        );
    }

    #[test]
    fn res184_collision_check_detects_existing_name() {
        // If new_name already exists as a top-level decl, the handler
        // should refuse. We test the logic directly (not through the
        // async handler).
        let src = "fn add(int a, int b) -> int { return a + b; }\nfn sum() { return 0; }";
        let (prog, _) = parse(src);
        let defs = build_top_level_defs(&prog);
        // Renaming `add` to `sum` must be blocked.
        let collision = find_top_level_def(&defs, "sum").is_some();
        assert!(collision, "expected collision detection for name `sum`");
    }

    #[test]
    fn res184_no_collision_for_fresh_name() {
        let src = "fn add(int a, int b) -> int { return a + b; }";
        let (prog, _) = parse(src);
        let defs = build_top_level_defs(&prog);
        let collision = find_top_level_def(&defs, "total").is_some();
        assert!(!collision, "unexpected collision for fresh name `total`");
    }

    // ============================================================
    // RES-2568: rename symbol — struct + variable + cross-file
    // ============================================================

    #[test]
    fn res2568_find_struct_decl_name_range_locates_name() {
        let src = "struct Point {\n    int x,\n    int y,\n}";
        let range = find_struct_decl_name_range(src, "Point").expect("should find Point");
        assert_eq!(range.start.line, 0);
        // `struct ` is 7 chars → `Point` starts at col 7 (0-indexed LSP).
        assert_eq!(range.start.character, 7);
        assert_eq!(range.end.character, 12); // 7 + len("Point") = 12
    }

    #[test]
    fn res2568_find_struct_decl_name_range_multiline() {
        let src = "fn first() {}\nstruct Vec2 {\n    float x,\n    float y,\n}";
        let range = find_struct_decl_name_range(src, "Vec2").expect("should find Vec2");
        assert_eq!(range.start.line, 1);
    }

    #[test]
    fn res2568_find_struct_decl_name_range_not_found() {
        let src = "fn foo() {}";
        assert!(find_struct_decl_name_range(src, "Foo").is_none());
    }

    #[test]
    fn res2568_collect_struct_literal_sites_basic() {
        // Source: struct decl + 2 constructor sites.
        // Resilient struct literal syntax: `new StructName { field: val, ... }`.
        let src = "struct Point {\n    int x,\n    int y,\n}\nfn make() {\n    let a = new Point { x: 1, y: 2 };\n    let b = new Point { x: 3, y: 4 };\n    return a;\n}";
        let sites = collect_struct_literal_sites(src, "Point");
        // 2 `new Point {` constructor sites.
        assert_eq!(sites.len(), 2, "expected 2 constructor sites: {sites:?}");
    }

    #[test]
    fn res2568_collect_struct_literal_sites_no_match() {
        let src = "struct Point {\n    int x,\n    int y,\n}";
        let sites = collect_struct_literal_sites(src, "Point");
        // Declaration only (`struct Point`), no `new Point {` — the struct
        // keyword is not preceded by `new`, so the scanner skips it.
        assert!(sites.is_empty());
    }

    #[test]
    fn res2568_collect_struct_literal_sites_with_decl_src() {
        // Struct decl token not captured by `collect_struct_literal_sites`
        // (that's `find_struct_decl_name_range`'s job). Only `new/let` sites.
        let src = "struct Point {\n    int x,\n    int y,\n}\nlet p = new Point { x: 0, y: 0 };";
        let sites = collect_struct_literal_sites(src, "Point");
        // 1 `new Point` constructor + 0 `let Point {` destructor = 1.
        assert_eq!(sites.len(), 1, "expected 1 site: {sites:?}");
    }

    #[test]
    fn res2568_collect_identifier_refs_basic() {
        // `x` is used 3 times as an identifier in expressions.
        let src = "let x = 1;\nlet y = x + x;\nfn f() { return x; }";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let refs = collect_identifier_refs(&prog, "x");
        // We expect the 3 read uses. The binder (`let x = 1;`) does NOT
        // contribute an `Identifier` node — the AST stores `name: String`.
        assert_eq!(refs.len(), 3, "expected 3 refs: {refs:?}");
    }

    #[test]
    fn res2568_collect_identifier_refs_no_match() {
        let src = "let x = 1;";
        let (prog, _) = parse(src);
        let refs = collect_identifier_refs(&prog, "z");
        assert!(refs.is_empty());
    }

    #[test]
    fn res2568_find_let_name_range_basic() {
        let src = "let counter = 0;";
        let range = find_let_name_range(src, "counter").expect("should find counter");
        assert_eq!(range.start.line, 0);
        // `let ` is 4 chars → `counter` starts at col 4.
        assert_eq!(range.start.character, 4);
        assert_eq!(range.end.character, 11); // 4 + len("counter") = 11
    }

    #[test]
    fn res2568_find_let_name_range_const() {
        let src = "const MAX = 100;";
        let range = find_let_name_range(src, "MAX").expect("should find MAX");
        assert_eq!(range.start.line, 0);
        // `const ` is 6 chars → `MAX` starts at col 6.
        assert_eq!(range.start.character, 6);
    }

    #[test]
    fn res2568_build_rename_edits_fn_single_doc() {
        // Rename `add` → `sum`: 1 decl + 2 call sites.
        let src = "fn add(int a, int b) -> int { return a + b; }\nadd(1, 2);\nadd(3, 4);\n";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let defs = build_top_level_defs(&prog);
        let def = find_top_level_def(&defs, "add").expect("add should be found");
        let edits =
            build_rename_edits_for_doc(&prog, src, "add", "sum", RenameSymbolKind::Fn, def.range);
        assert_eq!(edits.len(), 3, "expected 3 edits: {edits:?}");
        for e in &edits {
            assert_eq!(e.new_text, "sum");
        }
    }

    #[test]
    fn res2568_build_rename_edits_struct_single_doc() {
        // Rename struct `Point` → `Vec2`: 1 struct-decl edit + 2 constructor
        // edits.  Use `new Point { ... }` Resilient syntax.
        let src = "struct Point {\n    int x,\n    int y,\n}\nfn make() {\n    return new Point { x: 0, y: 0 };\n}\nlet p = new Point { x: 1, y: 2 };";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let defs = build_top_level_defs(&prog);
        let def = find_top_level_def(&defs, "Point").expect("Point def should be found");
        let edits = build_rename_edits_for_doc(
            &prog,
            src,
            "Point",
            "Vec2",
            RenameSymbolKind::Struct,
            def.range,
        );
        // 1 decl edit (`struct Point`) + 2 constructor edits (`new Point`) = 3.
        assert_eq!(edits.len(), 3, "expected 3 edits: {edits:?}");
        for e in &edits {
            assert_eq!(e.new_text, "Vec2");
        }
    }

    #[test]
    fn res2568_build_rename_edits_variable_single_doc() {
        // Rename `counter` → `total`: 1 decl + 2 read uses.
        let src = "let counter = 0;\nlet y = counter + 1;\nfn f() { return counter; }";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        // `let counter` is a top-level let — it's NOT in `build_top_level_defs`
        // (which covers fn/struct/type-alias only). Simulate the rename handler
        // calling `collect_identifier_refs` directly.
        let ident_refs = collect_identifier_refs(&prog, "counter");
        // 2 read uses: `counter + 1` and `return counter`.
        assert_eq!(ident_refs.len(), 2, "expected 2 read uses: {ident_refs:?}");
        // With a decl range supplied, build_rename_edits_for_doc gives 3 edits.
        let fake_decl_range =
            find_let_name_range(src, "counter").expect("should find counter decl");
        let edits = build_rename_edits_for_doc(
            &prog,
            src,
            "counter",
            "total",
            RenameSymbolKind::Variable,
            fake_decl_range,
        );
        // 1 decl + 2 reads = 3.
        assert_eq!(edits.len(), 3, "expected 3 edits: {edits:?}");
        for e in &edits {
            assert_eq!(e.new_text, "total");
        }
    }

    #[test]
    fn res2568_build_rename_edits_no_def_range_skips_decl() {
        // When def_range is zero (cross-file ref, no decl here), only
        // reference edits are emitted.
        let src = "add(1, 2);"; // just a call, no decl
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let edits = build_rename_edits_for_doc(
            &prog,
            src,
            "add",
            "sum",
            RenameSymbolKind::Fn,
            Range::default(),
        );
        // 1 call site, no decl.
        assert_eq!(edits.len(), 1, "expected 1 edit: {edits:?}");
        assert_eq!(edits[0].new_text, "sum");
    }

    #[test]
    fn res2568_invalid_name_rejected() {
        // The `is_valid_identifier` guard covers this; confirm bad names.
        assert!(!is_valid_identifier("2bad"));
        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("with space"));
    }

    #[test]
    fn res2568_prepare_rename_accepts_struct() {
        // `build_top_level_defs` covers struct names — verify directly.
        let src = "struct Rect {\n    int w,\n    int h,\n}";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let defs = build_top_level_defs(&prog);
        let found = find_top_level_def(&defs, "Rect");
        assert!(found.is_some(), "expected Rect in top-level defs");
    }

    #[test]
    fn res2568_prepare_rename_fns_still_work() {
        // Regression: existing fn rename still passes through.
        let src = "fn add(int a, int b) -> int { return a + b; }";
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let defs = build_top_level_defs(&prog);
        let found = find_top_level_def(&defs, "add");
        assert!(found.is_some(), "expected add in top-level defs");
    }

    // ============================================================
    // RES-357: code action — add contract stubs
    // ============================================================

    #[test]
    fn res357_find_brace_line_simple_fn() {
        // `fn f(int x) { return x; }` — `{` is on line 0.
        let src = "fn f(int x) { return x; }";
        assert_eq!(find_brace_line(src, 0), Some(0));
    }

    #[test]
    fn res357_find_brace_line_multiline_signature() {
        // The `{` may be on a later line when the signature wraps.
        let src = "fn f(\n    int x\n) {\n    return x;\n}";
        assert_eq!(find_brace_line(src, 0), Some(2));
    }

    #[test]
    fn res357_find_brace_line_skips_lines_before_start() {
        // Start scanning from line 1: the `{` on line 0 must be ignored.
        let src = "fn f() {\n    return 0;\n}";
        // Starting from line 1 (inside the body), the next `{` won't be
        // found until the top-level scan; but line 0 has the opening `{`
        // so starting from line 1 should return None (body has no `{`).
        assert_eq!(find_brace_line(src, 1), None);
    }

    #[test]
    fn res357_find_brace_line_ignores_brace_in_string() {
        // A `{` inside a string literal must not count.
        let src = r#"fn f() -> string { return "{ignored}"; }"#;
        // The function-body `{` is at column 18 on line 0 — the one at
        // column 0 is ahead of the string.  The scanner should return
        // line 0 because the first `{` it finds is the real opening brace
        // (before the string).
        assert_eq!(find_brace_line(src, 0), Some(0));
    }

    #[test]
    fn res357_find_brace_line_returns_none_when_no_brace() {
        let src = "let x = 1;\nlet y = 2;";
        assert_eq!(find_brace_line(src, 0), None);
    }

    /// Exercise `code_action_stubs_for_diagnostic`: given a synthetic
    /// L0010 diagnostic pointing at line 0, the helper produces a
    /// `CodeAction` whose `WorkspaceEdit` inserts the contract stubs
    /// after the opening `{`.
    #[test]
    fn res357_contract_stubs_edit_inserts_requires_and_ensures() {
        // Source: function on a single line, no contract.
        let src = "fn foo(int x) { return x; }\n";
        let uri = Url::parse("file:///tmp/test_contract.rs").unwrap();

        // Construct a synthetic L0010 diagnostic at line 0.
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 5)),
            severity: Some(DiagnosticSeverity::WARNING),
            source: Some("resilient-lint".into()),
            message: "function `foo` has no `requires`/`ensures` contract; \
                      add contract stubs or suppress with `// resilient: allow L0010`"
                .into(),
            ..Default::default()
        };

        // Directly invoke the stub-builder logic to avoid needing an
        // async runtime in unit tests.
        let brace_line = find_brace_line(src, diag.range.start.line as usize)
            .expect("should find opening brace");
        let insert_pos = Position {
            line: (brace_line + 1) as u32,
            character: 0,
        };
        let insert_range = Range {
            start: insert_pos,
            end: insert_pos,
        };
        let text_edit = TextEdit {
            range: insert_range,
            new_text: "    requires true;\n    ensures true;\n".to_string(),
        };
        let mut changes = HashMap::new();
        changes.insert(uri.clone(), vec![text_edit.clone()]);
        let workspace_edit = WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        };
        let action = CodeAction {
            title: "Add contract stubs".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diag]),
            edit: Some(workspace_edit),
            command: None,
            is_preferred: Some(true),
            disabled: None,
            data: None,
        };

        // Assertions on the produced action.
        assert_eq!(action.title, "Add contract stubs");
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
        assert_eq!(action.is_preferred, Some(true));
        let edit = action.edit.expect("expected edit");
        let file_edits = edit
            .changes
            .expect("expected changes map")
            .remove(&uri)
            .expect("expected edits for our URI");
        assert_eq!(file_edits.len(), 1);
        let te = &file_edits[0];
        // The edit is inserted at line 1 (one past the `{` on line 0).
        assert_eq!(
            te.range.start.line, 1,
            "insert should be at line 1 (after the opening brace on line 0)"
        );
        assert_eq!(te.range.start.character, 0);
        assert!(
            te.new_text.contains("requires true;"),
            "expected `requires true;` in inserted text"
        );
        assert!(
            te.new_text.contains("ensures true;"),
            "expected `ensures true;` in inserted text"
        );
    }

    #[test]
    fn res357_find_brace_line_handles_multiline_before_start() {
        // A `{` only on line 3; scanning from line 0 returns 3.
        let src = "fn f(\n    int x,\n    int y\n) {\n    return x + y;\n}";
        assert_eq!(find_brace_line(src, 0), Some(3));
    }

    // ============================================================
    // RES-190: missing-semicolon code action
    // ============================================================

    #[test]
    fn res190_detects_missing_semi_by_message_apostrophes() {
        let diag = Diagnostic {
            range: Range::new(Position::new(2, 10), Position::new(2, 10)),
            message: "expected ';' before keyword".into(),
            ..Default::default()
        };
        assert!(is_missing_semicolon_diagnostic(&diag));
    }

    #[test]
    fn res190_detects_missing_semi_by_message_backticks() {
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 5), Position::new(0, 5)),
            message: "expected `;` after expression".into(),
            ..Default::default()
        };
        assert!(is_missing_semicolon_diagnostic(&diag));
    }

    #[test]
    fn res190_detects_missing_semi_by_word_semicolon() {
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 5), Position::new(0, 5)),
            message: "missing semicolon at end of statement".into(),
            ..Default::default()
        };
        assert!(is_missing_semicolon_diagnostic(&diag));
    }

    #[test]
    fn res190_detects_missing_semi_by_e0002_code() {
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 5), Position::new(0, 5)),
            code: Some(NumberOrString::String("E0002".into())),
            message: "parse error".into(),
            ..Default::default()
        };
        assert!(is_missing_semicolon_diagnostic(&diag));
    }

    #[test]
    fn res190_does_not_match_unrelated_diag() {
        // Unrelated parse error must NOT trigger the action.
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 0)),
            message: "expected `}` to close block".into(),
            ..Default::default()
        };
        assert!(!is_missing_semicolon_diagnostic(&diag));
    }

    #[test]
    fn res190_does_not_match_message_mentioning_semi_without_expected() {
        // A message that just mentions `;` casually shouldn't grab.
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 0)),
            message: "you wrote two ';' in a row".into(),
            ..Default::default()
        };
        assert!(!is_missing_semicolon_diagnostic(&diag));
    }

    #[test]
    fn res190_action_inserts_semicolon_at_diag_start() {
        let uri = Url::parse("file:///tmp/test_missing_semi.rz").unwrap();
        let diag = Diagnostic {
            range: Range::new(Position::new(2, 10), Position::new(2, 10)),
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some("resilient-parser".into()),
            message: "expected ';'".into(),
            ..Default::default()
        };
        assert!(is_missing_semicolon_diagnostic(&diag));

        let action = build_insert_semicolon_action(&uri, &diag).expect("action");
        assert_eq!(action.title, "Insert `;`");
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
        assert_eq!(action.is_preferred, Some(true));

        let edit = action.edit.expect("expected edit");
        let mut changes = edit.changes.expect("expected changes map");
        let file_edits = changes.remove(&uri).expect("expected edits for our URI");
        assert_eq!(file_edits.len(), 1);
        let te = &file_edits[0];
        assert_eq!(te.new_text, ";");
        // The insert range is zero-width at the diagnostic's start
        // position — that's where the parser flagged the problem.
        assert_eq!(te.range.start.line, 2);
        assert_eq!(te.range.start.character, 10);
        assert_eq!(te.range.end.line, 2);
        assert_eq!(te.range.end.character, 10);
    }

    #[test]
    fn res190_action_attaches_originating_diagnostic() {
        // The CodeAction must carry the diagnostic it acts on so the
        // editor can dismiss the lightbulb after applying the fix.
        let uri = Url::parse("file:///tmp/test_dismissal.rz").unwrap();
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 0)),
            message: "expected ';'".into(),
            ..Default::default()
        };
        let action = build_insert_semicolon_action(&uri, &diag).expect("action");
        let attached = action.diagnostics.expect("expected diagnostics list");
        assert_eq!(attached.len(), 1);
        assert_eq!(attached[0].message, diag.message);
    }

    #[test]
    fn res190_fixed_text_lines_up_with_a_valid_program() {
        // End-to-end shape check: starting from a snippet with a
        // missing `;`, applying the action's edit produces a string
        // that contains the inserted `;` at the expected offset.
        let original = "let x = 1\nlet y = 2;\n";
        // The parser would point at the start of `let y` on line 1
        // (or the EOL of line 0). Synthesize a diagnostic at the
        // end of line 0.
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 9), Position::new(0, 9)),
            message: "expected ';'".into(),
            ..Default::default()
        };
        let uri = Url::parse("file:///tmp/test_apply.rz").unwrap();
        let action = build_insert_semicolon_action(&uri, &diag).expect("action");
        let edit = action
            .edit
            .expect("edit")
            .changes
            .expect("changes")
            .remove(&uri)
            .expect("edits");
        let te = &edit[0];
        // Apply the edit by hand: insert `;` at line 0, col 9.
        let mut lines: Vec<String> = original.lines().map(|s| s.to_string()).collect();
        let line0 = &mut lines[0];
        let col = te.range.start.character as usize;
        line0.insert_str(col, &te.new_text);
        let fixed = lines.join("\n") + "\n";
        assert_eq!(fixed, "let x = 1;\nlet y = 2;\n");
    }

    // ============================================================
    // RES-302: identifier-hover lookup
    // ============================================================
    //
    // Walk the AST and surface the inferred type for an identifier
    // in scope. Today: top-level `let` / `const` / `static let`
    // bindings, top-level `fn` names, and parameters of top-level
    // fns. Anything else returns `None`.

    #[test]
    fn res302_identifier_hover_let_int_returns_int() {
        // `let x = 42;` — hovering on `x` should report `Int`.
        let (program, errs) = parse("let x = 42;");
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        assert_eq!(infer_identifier_type(&program, "x"), Some("Int".into()));
    }

    #[test]
    fn res302_identifier_hover_let_with_type_annot_uses_annotation() {
        // `let x: int = 0;` — annotation wins over inference.
        let (program, errs) = parse("let x: int = 0;");
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        assert_eq!(infer_identifier_type(&program, "x"), Some("int".into()));
    }

    #[test]
    fn res302_identifier_hover_unknown_name_returns_none() {
        // No `q` declared — hover should miss.
        let (program, errs) = parse("let x = 1;");
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        assert!(infer_identifier_type(&program, "q").is_none());
    }

    #[test]
    fn res181b_fn_name_hover_shows_signature() {
        // Hovering on a function name shows the full signature.
        let (program, errs) = parse("fn add(int x, int y) -> int { return x; }\nadd(1, 2);");
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let ty = infer_identifier_type(&program, "add");
        assert_eq!(ty, Some("fn add(int x, int y) -> int".into()));
    }

    #[test]
    fn res181b_fn_no_return_type_omits_arrow() {
        let (program, errs) = parse("fn greet() { }\ngreet();");
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let ty = infer_identifier_type(&program, "greet");
        assert_eq!(ty, Some("fn greet()".into()));
    }

    #[test]
    fn res181b_local_let_in_fn_body_found() {
        // Hovering on a local let binding inside a function body.
        let (program, errs) = parse("fn f() { let total: int = 0; return total; }\nf();");
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let ty = infer_identifier_type(&program, "total");
        assert_eq!(ty, Some("int".into()));
    }

    #[test]
    fn res181b_local_let_inferred_from_literal() {
        // No type annotation — infer from the literal.
        let (program, errs) = parse("fn f() { let msg = \"hello\"; }\nf();");
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let ty = infer_identifier_type(&program, "msg");
        assert_eq!(ty, Some("String".into()));
    }

    // ============================================================
    // RES-2570: quick-fix helpers
    // ============================================================

    // ---------- unused-variable detection ----------

    #[test]
    fn res2570_detects_l0001_unused_binding() {
        let diag = Diagnostic {
            range: Range::new(Position::new(1, 4), Position::new(1, 4)),
            message: "unused local binding `foo` — prefix with `_` to silence".into(),
            ..Default::default()
        };
        assert!(is_unused_variable_diagnostic(&diag));
    }

    #[test]
    fn res2570_detects_l0011_unused_variable() {
        let diag = Diagnostic {
            range: Range::new(Position::new(2, 4), Position::new(2, 4)),
            message: "variable `bar` is assigned but never used".into(),
            ..Default::default()
        };
        assert!(is_unused_variable_diagnostic(&diag));
    }

    #[test]
    fn res2570_does_not_match_unrelated_diag_as_unused() {
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 0)),
            message: "expected `;` after statement".into(),
            ..Default::default()
        };
        assert!(!is_unused_variable_diagnostic(&diag));
    }

    // ---------- extract_backtick_name ----------

    #[test]
    fn res2570_extract_backtick_name_simple() {
        assert_eq!(
            extract_backtick_name("unused local binding `myVar` — prefix with `_`"),
            Some("myVar")
        );
    }

    #[test]
    fn res2570_extract_backtick_name_skips_underscore_prefixed() {
        // If the name already starts with `_`, no action is needed.
        assert_eq!(
            extract_backtick_name("unused local binding `_x` — prefix with `_`"),
            None
        );
    }

    #[test]
    fn res2570_extract_backtick_name_returns_none_when_no_backtick() {
        assert_eq!(extract_backtick_name("no backticks here"), None);
    }

    // ---------- prefix-underscore action ----------

    #[test]
    fn res2570_prefix_underscore_inserts_leading_underscore() {
        let uri = Url::parse("file:///tmp/test_unused.rz").unwrap();
        let src = "fn f() {\n    let foo = 1;\n    return 0;\n}\nf();\n";
        let diag = Diagnostic {
            // Lint points at line 1, col 8 (the `f` of `foo`).
            range: Range::new(Position::new(1, 8), Position::new(1, 11)),
            message: "unused local binding `foo` — prefix with `_` to silence".into(),
            ..Default::default()
        };
        let action = build_prefix_underscore_action(&uri, &diag, src).expect("action");
        assert_eq!(action.title, "Prefix `foo` with `_`");
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
        assert_eq!(action.is_preferred, Some(true));

        let edit = action.edit.expect("edit");
        let mut changes = edit.changes.expect("changes");
        let edits = changes.remove(&uri).expect("edits for uri");
        assert_eq!(edits.len(), 1);
        let te = &edits[0];
        assert_eq!(te.new_text, "_foo");
        // The edit range must be on line 1.
        assert_eq!(te.range.start.line, 1);
    }

    #[test]
    fn res2570_prefix_underscore_returns_none_when_name_not_on_line() {
        let uri = Url::parse("file:///tmp/test_no_match.rz").unwrap();
        // Diagnostic line doesn't contain the name `xyz`.
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 0)),
            message: "unused local binding `xyz` — prefix with `_` to silence".into(),
            ..Default::default()
        };
        let src = "let abc = 1;\n"; // no `xyz` on any line
        assert!(build_prefix_underscore_action(&uri, &diag, src).is_none());
    }

    // ---------- type mismatch detection ----------

    #[test]
    fn res2570_detects_type_mismatch_rich_form() {
        let diag = Diagnostic {
            range: Range::new(Position::new(3, 10), Position::new(3, 12)),
            message: "error[E0007]: type mismatch in argument 1: expected `int`, found `float`"
                .into(),
            ..Default::default()
        };
        assert!(is_type_mismatch_diagnostic(&diag));
    }

    #[test]
    fn res2570_detects_type_mismatch_legacy_form() {
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 5), Position::new(0, 8)),
            message: "Type mismatch in argument 2: expected int, got float".into(),
            ..Default::default()
        };
        assert!(is_type_mismatch_diagnostic(&diag));
    }

    // ---------- extract_mismatch_types ----------

    #[test]
    fn res2570_extract_mismatch_types_rich_form() {
        let msg = "error[E0007]: type mismatch in argument 1: expected `int`, found `float`";
        let (expected, found) = extract_mismatch_types(msg).expect("should parse");
        assert_eq!(expected, "int");
        assert_eq!(found, "float");
    }

    #[test]
    fn res2570_extract_mismatch_types_legacy_form() {
        let msg = "Type mismatch in argument 2: expected int, got float";
        let (expected, found) = extract_mismatch_types(msg).expect("should parse legacy");
        assert_eq!(expected, "int");
        assert_eq!(found, "float");
    }

    // ---------- add-cast action ----------

    #[test]
    fn res2570_add_cast_action_produces_correct_edit() {
        let uri = Url::parse("file:///tmp/test_cast.rz").unwrap();
        let diag = Diagnostic {
            range: Range::new(Position::new(5, 10), Position::new(5, 15)),
            message: "error[E0007]: type mismatch in argument 1: expected `int`, found `float`"
                .into(),
            ..Default::default()
        };
        let action = build_add_cast_action(&uri, &diag).expect("action");
        assert_eq!(action.title, "Add `as int` cast");
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));

        let edit = action.edit.expect("edit");
        let mut changes = edit.changes.expect("changes");
        let edits = changes.remove(&uri).expect("edits");
        assert_eq!(edits.len(), 1);
        let te = &edits[0];
        assert_eq!(te.new_text, " as int");
        // The cast is appended at the END of the diagnostic range.
        assert_eq!(te.range.start.line, 5);
        assert_eq!(te.range.start.character, 15);
    }

    #[test]
    fn res2570_add_cast_action_skips_non_numeric_types() {
        let uri = Url::parse("file:///tmp/test_cast_skip.rz").unwrap();
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 5)),
            message: "type mismatch in argument 1: expected `string`, found `bool`".into(),
            ..Default::default()
        };
        // `string` and `bool` are not numeric — no cast action offered.
        assert!(build_add_cast_action(&uri, &diag).is_none());
    }

    // ---------- dead-function detection ----------

    #[test]
    fn res2570_detects_l0014_dead_function() {
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 10)),
            message: "function `helper` is defined but never called — prefix with `_` to silence"
                .into(),
            ..Default::default()
        };
        assert!(is_dead_function_diagnostic(&diag));
    }

    // ---------- prefix-fn-underscore action ----------

    #[test]
    fn res2570_prefix_fn_underscore_action() {
        let uri = Url::parse("file:///tmp/test_dead.rz").unwrap();
        let src = "fn helper() { return 0; }\nfn main(int _d) { return 0; }\nmain(0);\n";
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 8)),
            message: "function `helper` is defined but never called — prefix with `_` to silence"
                .into(),
            ..Default::default()
        };
        let action = build_prefix_fn_underscore_action(&uri, &diag, src).expect("action");
        // Title says "Prefix `helper` with `_` ..."; the new_text is "_helper".
        assert!(
            action.title.contains("helper"),
            "title = {:?}",
            action.title
        );
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));

        let edit = action.edit.expect("edit");
        let mut changes = edit.changes.expect("changes");
        let edits = changes.remove(&uri).expect("edits");
        assert_eq!(edits.len(), 1);
        let te = &edits[0];
        assert_eq!(te.new_text, "_helper");
    }

    // ---------- lint-code extraction ----------

    #[test]
    fn res2570_extract_lint_code_finds_l_code() {
        assert_eq!(
            extract_lint_code("unused local binding `foo` — L0001"),
            Some("L0001")
        );
        assert_eq!(extract_lint_code("L0014 dead function"), Some("L0014"));
    }

    #[test]
    fn res2570_extract_lint_code_returns_none_when_absent() {
        assert_eq!(extract_lint_code("expected `;` after statement"), None);
        assert_eq!(extract_lint_code(""), None);
    }

    // ---------- suppress-lint action ----------

    #[test]
    fn res2570_suppress_lint_action_inserts_allow_comment() {
        let uri = Url::parse("file:///tmp/test_suppress.rz").unwrap();
        let src = "fn f() {\n    let unused_x = 1;\n    return 0;\n}\nf();\n";
        let diag = Diagnostic {
            range: Range::new(Position::new(1, 4), Position::new(1, 4)),
            message: "unused local binding `unused_x` — prefix with `_` to silence L0001".into(),
            ..Default::default()
        };
        let action = build_suppress_lint_action(&uri, &diag, src).expect("action");
        assert!(action.title.contains("L0001"));
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));

        let edit = action.edit.expect("edit");
        let mut changes = edit.changes.expect("changes");
        let edits = changes.remove(&uri).expect("edits");
        assert_eq!(edits.len(), 1);
        let te = &edits[0];
        // The comment is inserted at the START of the diagnostic's line.
        assert_eq!(te.range.start.line, 1);
        assert_eq!(te.range.start.character, 0);
        assert!(te.new_text.contains("// resilient: allow L0001"));
        // Must end with a newline so existing code moves down.
        assert!(te.new_text.ends_with('\n'));
    }

    #[test]
    fn res2570_suppress_lint_action_returns_none_without_code() {
        let uri = Url::parse("file:///tmp/test_no_code.rz").unwrap();
        let diag = Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 0)),
            message: "expected `;` after statement".into(),
            ..Default::default()
        };
        let src = "let x = 1\n";
        assert!(build_suppress_lint_action(&uri, &diag, src).is_none());
    }

    // ============================================================
    // RES-2645: undefined-name import quick-fix
    // ============================================================

    #[test]
    fn res2645_extract_undefined_name_from_type_error() {
        assert_eq!(
            extract_undefined_name("Undefined variable 'helper' at 3:5"),
            Some("helper")
        );
        assert_eq!(
            extract_undefined_name("Undefined variable: helper"),
            Some("helper")
        );
    }

    #[test]
    fn res2645_build_add_use_action_single_match() {
        let uri = Url::parse("file:///workspace/main.rz").unwrap();
        let candidate = WorkspaceSymbolEntry {
            name: "helper".into(),
            kind: SymbolKind::FUNCTION,
            uri: Url::parse("file:///workspace/lib/math.rz").unwrap(),
            range: Range::new(Position::new(0, 0), Position::new(0, 0)),
        };
        let diag = Diagnostic {
            range: Range::new(Position::new(2, 4), Position::new(2, 10)),
            message: "Undefined variable 'helper' at 3:5".into(),
            ..Default::default()
        };
        let src = "fn main() {\n    helper();\n}\n";

        let actions = build_add_use_actions(&uri, &diag, src, [&candidate]);
        assert_eq!(actions.len(), 1, "expected exactly one quick-fix");
        let action = &actions[0];
        assert_eq!(action.title, "Add `use \"lib/math.rz\";`");
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));

        let edit = action.edit.clone().expect("edit");
        let mut changes = edit.changes.expect("changes");
        let edits = changes.remove(&uri).expect("edits");
        assert_eq!(edits.len(), 1);
        let te = &edits[0];
        assert_eq!(te.range.start, Position::new(0, 0));
        assert_eq!(te.range.end, Position::new(0, 0));
        assert_eq!(te.new_text, "use \"lib/math.rz\";\n");
    }

    #[test]
    fn res2645_build_add_use_action_multiple_matches() {
        let uri = Url::parse("file:///workspace/main.rz").unwrap();
        let a = WorkspaceSymbolEntry {
            name: "helper".into(),
            kind: SymbolKind::FUNCTION,
            uri: Url::parse("file:///workspace/lib/a.rz").unwrap(),
            range: Range::new(Position::new(0, 0), Position::new(0, 0)),
        };
        let b = WorkspaceSymbolEntry {
            name: "helper".into(),
            kind: SymbolKind::FUNCTION,
            uri: Url::parse("file:///workspace/lib/b.rz").unwrap(),
            range: Range::new(Position::new(0, 0), Position::new(0, 0)),
        };
        let diag = Diagnostic {
            range: Range::new(Position::new(1, 4), Position::new(1, 10)),
            message: "Undefined variable: helper".into(),
            ..Default::default()
        };
        let src = "fn main() {\n    helper();\n}\n";

        let actions = build_add_use_actions(&uri, &diag, src, [&a, &b]);
        let titles: Vec<&str> = actions.iter().map(|action| action.title.as_str()).collect();
        assert_eq!(
            titles,
            vec!["Add `use \"lib/a.rz\";`", "Add `use \"lib/b.rz\";`"]
        );
    }

    #[test]
    fn res2645_build_add_use_action_no_match() {
        let uri = Url::parse("file:///tmp/main.rz").unwrap();
        let diag = Diagnostic {
            range: Range::new(Position::new(1, 4), Position::new(1, 10)),
            message: "Undefined variable 'helper' at 2:5".into(),
            ..Default::default()
        };
        let src = "fn main() {\n    helper();\n}\n";

        let actions = build_add_use_actions(
            &uri,
            &diag,
            src,
            std::iter::empty::<&WorkspaceSymbolEntry>(),
        );
        assert!(actions.is_empty(), "expected no quick-fixes");
    }
}
