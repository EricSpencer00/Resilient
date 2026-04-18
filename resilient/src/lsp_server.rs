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

use std::collections::HashMap;
use std::sync::Mutex;

use tower_lsp::jsonrpc::Result as JsonResult;
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidOpenTextDocumentParams,
    DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse, InitializeParams,
    InitializeResult, InitializedParams, MessageType, OneOf, Position, Range,
    ServerCapabilities, SymbolKind, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::{parse, typechecker, Node};

/// The LSP backend. Holds a `Client` handle for publishing diagnostics.
///
/// RES-185: the `documents` map caches each open document's last-
/// parsed AST, keyed by `Url`. Document-symbol (and future
/// cursor-aware) handlers consume from here instead of re-parsing
/// on every request.
pub struct Backend {
    client: Client,
    /// URI → latest parsed Program AST. Mutex-guarded because
    /// LSP handlers run on the tokio runtime's worker threads,
    /// and we never hold the lock across an `.await`.
    documents: Mutex<HashMap<Url, Node>>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Mutex::new(HashMap::new()),
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
            Node::Function { name, .. } => {
                make_symbol(name, SymbolKind::FUNCTION, spanned.span)
            }
            Node::StructDecl { name, .. } => {
                make_symbol(name, SymbolKind::STRUCT, spanned.span)
            }
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
    async fn initialize(&self, _params: InitializeParams) -> JsonResult<InitializeResult> {
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
}
